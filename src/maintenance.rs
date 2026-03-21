//! Background knowledge graph maintenance and rumination engine.
//!
//! Runs periodically per ANT, independent of user requests:
//! - Consolidate each topic graph (dedup, merge, collapse)
//! - Cross-link topic graphs in the meta-graph
//! - Migrate misplaced nodes from meta-graph to topic graphs
//! - Decay untested conjectures
//! - Detect contradictions and clusters
//! - Log all changes
//!
//! Rumination engine (when enabled):
//! - Active refutation — challenge existing beliefs
//! - Idea synthesis — conjecture transitive relationships
//! - Contradiction resolution — pit conflicting beliefs against each other
//! - Autonomous initiative — open-ended self-improvement

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::mpsc;

use crate::ai_worker::{CliRequest, TaskMap};
use crate::config::RuminationConfig;
use crate::registry::WsEvent;

/// System chat_id for rumination requests — negative to avoid collision with real users.
const RUMINATION_CHAT_ID: i64 = -1;

/// Appended to every rumination prompt to prevent the AI from asking "what next?"
const RUMINATION_STOP_DIRECTIVE: &str = "\n\n\
If you have questions that need human input (decisions, opinions, clarifications), \
write them to memory/questions.json — the human will see them next time they come \
online. Format: {\"questions\": [{\"timestamp\": \"YYYY-MM-DD\", \"topic\": \"...\", \
\"question\": \"...\", \"context\": \"...\"}]}. Append to existing questions, don't overwrite.\n\n\
IMPORTANT: This is an autonomous rumination task. Complete the work above, \
update the graph files, then STOP. Do not ask follow-up questions. \
Do not ask what to do next. Do not wait for input. \
Output a brief summary of what you changed and stop.";

/// Configuration for the maintenance daemon.
pub struct MaintenanceConfig {
    /// Root memory directory (contains knowledge.json and graphs/).
    pub memory_dir: PathBuf,
    /// How often to run consolidation (default: 1 hour).
    pub consolidation_interval: Duration,
    /// How often to run cross-linking (default: 6 hours).
    pub cross_link_interval: Duration,
    /// ANT name for logging.
    pub ant_name: String,
    /// Channel to send requests to the AI worker (None = rumination disabled).
    pub request_tx: Option<mpsc::UnboundedSender<CliRequest>>,
    /// Shared task map to check if the ANT is idle.
    pub tasks: Option<TaskMap>,
    /// Rumination engine configuration.
    pub rumination: RuminationConfig,
    /// Event broadcast channel for live graph updates in the dashboard.
    pub event_tx: Option<tokio::sync::broadcast::Sender<WsEvent>>,
}

/// A single entry in the rumination log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuminationEntry {
    /// When this rumination occurred.
    pub timestamp: String,
    /// What kind: "refutation", "synthesis", "contradiction", "initiative".
    pub kind: String,
    /// Which topic graph was involved.
    pub topic: String,
    /// Human-readable description of what was done.
    pub description: String,
    /// Number of edges created.
    pub edges_created: u32,
    /// Number of edges updated.
    pub edges_updated: u32,
}

/// Persistent rumination log.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct RuminationLog {
    pub entries: Vec<RuminationEntry>,
}

impl RuminationLog {
    /// Maximum entries to keep.
    const MAX_ENTRIES: usize = 200;

    pub fn load(memory_dir: &std::path::Path) -> Self {
        let path = memory_dir.join("rumination_log.json");
        if path.exists() {
            if let Ok(contents) = std::fs::read_to_string(&path) {
                if let Ok(log) = serde_json::from_str(&contents) {
                    return log;
                }
            }
        }
        Self::default()
    }

    fn save(&self, memory_dir: &std::path::Path) {
        let path = memory_dir.join("rumination_log.json");
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let tmp = path.with_extension("json.tmp");
            if std::fs::write(&tmp, &json).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
        }
    }

    fn append(&mut self, entry: RuminationEntry, memory_dir: &std::path::Path) {
        self.entries.push(entry);
        // Trim to max size.
        if self.entries.len() > Self::MAX_ENTRIES {
            let excess = self.entries.len() - Self::MAX_ENTRIES;
            self.entries.drain(..excess);
        }
        self.save(memory_dir);
    }
}

/// Run the background maintenance loop. Spawned as a tokio task per ANT.
pub async fn maintenance_loop(config: MaintenanceConfig) {
    let mut last_consolidation = std::time::Instant::now();
    let mut last_cross_link = std::time::Instant::now();
    let mut last_rumination = std::time::Instant::now();
    let mut idle_since: Option<std::time::Instant> = None;

    // Wait a bit before first run so the ANT has time to start.
    tokio::time::sleep(Duration::from_secs(60)).await;

    loop {
        tokio::time::sleep(Duration::from_secs(300)).await; // Check every 5 minutes.

        let now = std::time::Instant::now();

        // Consolidation pass — run on each topic graph.
        if now.duration_since(last_consolidation) >= config.consolidation_interval {
            last_consolidation = now;
            run_consolidation(&config);
        }

        // Cross-linking pass — find shared entities across topic graphs.
        if now.duration_since(last_cross_link) >= config.cross_link_interval {
            last_cross_link = now;
            run_cross_linking(&config);
        }

        // Rumination pass — autonomous thinking when idle.
        if config.rumination.enabled {
            if let Some(ref request_tx) = config.request_tx {
                let is_idle = config.tasks.as_ref()
                    .and_then(|t| t.lock().ok())
                    .map(|t| t.is_empty())
                    .unwrap_or(true);

                if is_idle {
                    // Track how long we've been idle.
                    let idle_start = idle_since.get_or_insert(now);
                    let idle_duration = now.duration_since(*idle_start);
                    let min_idle = Duration::from_secs(config.rumination.min_idle_secs);
                    let interval = Duration::from_secs(config.rumination.interval_secs);

                    if idle_duration >= min_idle
                        && now.duration_since(last_rumination) >= interval
                    {
                        last_rumination = now;
                        run_rumination(&config, request_tx).await;
                    }
                } else {
                    // ANT is busy — reset idle tracker.
                    idle_since = None;
                }
            }
        }
    }
}

/// Run a full rumination cycle — synthesis, refutation, contradiction, initiative.
async fn run_rumination(
    config: &MaintenanceConfig,
    request_tx: &mpsc::UnboundedSender<CliRequest>,
) {
    log::info!("[{}] Rumination cycle starting", config.ant_name);
    let mut log = RuminationLog::load(&config.memory_dir);

    // 0. Compute corroboration strength across all topic graphs.
    run_corroboration_update(config);

    // 1. Synthesis first — cheap, no AI tokens.
    if config.rumination.synthesis_enabled {
        let count = run_synthesis(config, &mut log);
        if count > 0 {
            log::info!("[{}] Synthesis created {} new edges", config.ant_name, count);
        }
    }

    // 1b. Investigate undetermined connections ('?' edges).
    run_undetermined_connections(config, request_tx, &mut log);

    // 2. Competition — pit similar ideas against each other.
    run_competition(config, request_tx, &mut log);

    // 3. Cross-domain pattern transfer — find insights across topics.
    run_pattern_transfer(config, request_tx, &mut log);

    // 4. Active refutation — core capability.
    if config.rumination.refutation_enabled {
        run_refutation(config, request_tx, &mut log);
    }

    // 5. Contradiction resolution.
    if config.rumination.contradiction_resolution {
        run_contradiction_resolution(config, request_tx, &mut log);
    }

    // 6. Autonomous initiative (most expensive, opt-in).
    if config.rumination.initiative_enabled {
        run_initiative(config, request_tx, &mut log);
    }

    // 7. Meta-rumination — review and evolve the thinking process itself.
    run_meta_rumination(config, request_tx, &mut log);

    // Post a short summary to the chat history so the human can see what happened.
    let summary = build_rumination_summary(&log);
    if !summary.is_empty() {
        post_to_chat_history(config, &summary);
    }

    log::info!("[{}] Rumination cycle complete", config.ant_name);
}

// ── Corroboration Network Strength ──────────────────────────────────

/// Recompute corroboration_strength for all edges in all topic graphs.
/// This gives ideas that are well-connected to other strong ideas a fitness boost.
fn run_corroboration_update(config: &MaintenanceConfig) {
    let graphs_dir = config.memory_dir.join("graphs");
    if !graphs_dir.exists() { return; }

    let entries = match std::fs::read_dir(&graphs_dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !is_topic_graph(&path) { continue; }

        let mut kg = crate::knowledge::KnowledgeGraph::load(&path);
        if kg.node_count() < 2 { continue; }

        kg.compute_corroboration_strength();
        kg.save();
        broadcast_graph_update(config, &topic_name(&path), "rumination");
    }
}

// ── Undetermined Connections ─────────────────────────────────────────

/// Find '?' edges (undetermined connections) and ask the AI to investigate them.
fn run_undetermined_connections(
    config: &MaintenanceConfig,
    request_tx: &mpsc::UnboundedSender<CliRequest>,
    log: &mut RuminationLog,
) {
    let graphs_dir = config.memory_dir.join("graphs");
    if !graphs_dir.exists() { return; }

    let entries = match std::fs::read_dir(&graphs_dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    let mut sent = 0u32;

    for entry in entries.flatten() {
        let path = entry.path();
        if !is_topic_graph(&path) { continue; }

        let topic = topic_name(&path);
        if !config.rumination.topics.is_empty()
            && !config.rumination.topics.iter().any(|t| t == &topic)
        {
            continue;
        }

        let kg = crate::knowledge::KnowledgeGraph::load(&path);
        let undetermined = kg.undetermined_connections(3);

        for (from, to) in &undetermined {
            let prompt = format!(
                "RUMINATION — UNDETERMINED CONNECTION\n\n\
                 In the topic graph 'memory/graphs/{}.json', there is a '?' connection \
                 between '{}' and '{}'. This means these entities are in the graph but \
                 their relationship hasn't been established yet.\n\n\
                 Your task:\n\
                 1. Consider what relationship might exist between '{}' and '{}'\n\
                 2. Look at their other connections in the graph for clues\n\
                 3. If you can determine a relationship:\n\
                    - Replace the '?' edge with a proper relation name\n\
                    - Set basis to 'inferred', confidence based on how certain you are\n\
                    - Add an evidence_log entry explaining your reasoning\n\
                 4. If you cannot determine a relationship:\n\
                    - Leave it as '?' — don't make something up\n\
                    - Consider adding a question to memory/questions.json for the human\n\
                 5. Update the topic graph file{}",
                topic, from, to, from, to, RUMINATION_STOP_DIRECTIVE
            );

            let _ = request_tx.send(CliRequest {
                chat_id: RUMINATION_CHAT_ID,
                message: prompt,
                new_session: true,
                task_id: 0,
                source: "rumination".into(),
            });

            log.append(RuminationEntry {
                timestamp: chrono_now(),
                kind: "undetermined".into(),
                topic: topic.clone(),
                description: format!("Investigating '?' connection: {} ↔ {}", from, to),
                edges_created: 0,
                edges_updated: 0,
            }, &config.memory_dir);

            sent += 1;
            if sent >= 2 { break; }
        }

        if sent >= 2 { break; }
    }
}

// ── Darwinian Competition ───────────────────────────────────────────

/// Find competing hypotheses and send research prompts to evaluate them.
/// Ideas that explain the same thing compete; the stronger is reinforced,
/// the weaker is penalised. Source quality and beneficial impact matter.
fn run_competition(
    config: &MaintenanceConfig,
    request_tx: &mpsc::UnboundedSender<CliRequest>,
    log: &mut RuminationLog,
) {
    let graphs_dir = config.memory_dir.join("graphs");
    if !graphs_dir.exists() { return; }

    let entries = match std::fs::read_dir(&graphs_dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    let mut sent = 0u32;

    for entry in entries.flatten() {
        let path = entry.path();
        if !is_topic_graph(&path) { continue; }

        let topic = topic_name(&path);
        if !config.rumination.topics.is_empty()
            && !config.rumination.topics.iter().any(|t| t == &topic)
        {
            continue;
        }

        let kg = crate::knowledge::KnowledgeGraph::load(&path);
        let groups = kg.find_competitors();

        for group in groups.iter().take(1) {
            let competitors_desc: Vec<String> = group.competitors.iter()
                .map(|c| format!("  - '{}' ({:.0}%): {}", c.relation, c.confidence * 100.0, c.context))
                .collect();

            let prompt = format!(
                "RUMINATION — IDEA COMPETITION\n\n\
                 Multiple competing hypotheses exist about the relationship between \
                 '{}' and '{}' in topic graph: memory/graphs/{}.json\n\n\
                 Competing ideas:\n{}\n\n\
                 Your task:\n\
                 1. RESEARCH which hypothesis has the strongest evidence\n\
                 2. Consider the QUALITY of the sources behind each claim\n\
                 3. Consider the CORROBORATION — which idea is best supported by \
                    other strong ideas in the graph?\n\
                 4. Consider BENEFICIAL IMPACT — does one interpretation serve people \
                    and the planet better? If so, note it with a beneficial_impact score \
                    (-1.0 to 1.0) on the edge\n\
                 5. Strengthen the winner: update with evidence_type 'competition_won'\n\
                 6. Weaken the losers: update with evidence_type 'competition_lost'\n\
                 7. If ideas can coexist (not truly competing), explain why and keep both\n\n\
                 The goal is survival of the fittest idea — but fitness includes \
                 being beneficial, well-sourced, and well-corroborated.{}",
                group.node_a_label, group.node_b_label, topic,
                competitors_desc.join("\n"), RUMINATION_STOP_DIRECTIVE,
            );

            let _ = request_tx.send(CliRequest {
                chat_id: RUMINATION_CHAT_ID,
                message: prompt,
                new_session: true,
                task_id: 0,
                source: "rumination".into(),
            });

            log.append(RuminationEntry {
                timestamp: chrono_now(),
                kind: "competition".into(),
                topic: topic.clone(),
                description: format!(
                    "Competition between {} ideas about {} ↔ {}: {}",
                    group.competitors.len(),
                    group.node_a_label, group.node_b_label,
                    group.competitors.iter().map(|c| c.relation.as_str()).collect::<Vec<_>>().join(" vs "),
                ),
                edges_created: 0,
                edges_updated: 0,
            }, &config.memory_dir);

            sent += 1;
        }

        if sent >= 2 { break; }
    }
}

// ── Cross-Domain Pattern Transfer ───────────────────────────────────

/// Find similar patterns across different topic graphs.
/// When something learned in one domain mirrors a pattern in another,
/// the insight can strengthen both.
fn run_pattern_transfer(
    config: &MaintenanceConfig,
    request_tx: &mpsc::UnboundedSender<CliRequest>,
    log: &mut RuminationLog,
) {
    let graphs_dir = config.memory_dir.join("graphs");
    if !graphs_dir.exists() { return; }

    // Load all topic graphs.
    let mut topic_graphs: Vec<(String, crate::knowledge::KnowledgeGraph)> = Vec::new();
    let entries = match std::fs::read_dir(&graphs_dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !is_topic_graph(&path) { continue; }

        let topic = topic_name(&path);
        if !config.rumination.topics.is_empty()
            && !config.rumination.topics.iter().any(|t| t == &topic)
        {
            continue;
        }

        let kg = crate::knowledge::KnowledgeGraph::load(&path);
        if kg.node_count() >= 3 {
            topic_graphs.push((topic, kg));
        }
    }

    if topic_graphs.len() < 2 { return; }

    // Compare each pair of topic graphs for patterns.
    let mut best_match: Option<(String, String, crate::knowledge::PatternMatch)> = None;

    for i in 0..topic_graphs.len() {
        for j in (i + 1)..topic_graphs.len() {
            let patterns = topic_graphs[i].1.find_cross_domain_patterns(&topic_graphs[j].1, 1);
            if let Some(pattern) = patterns.into_iter().next() {
                // Only pick the best match (first found).
                if best_match.is_none() {
                    best_match = Some((
                        topic_graphs[i].0.clone(),
                        topic_graphs[j].0.clone(),
                        pattern,
                    ));
                }
            }
        }
    }

    let Some((topic_a, topic_b, pattern)) = best_match else { return };

    let prompt = format!(
        "RUMINATION — CROSS-DOMAIN PATTERN TRANSFER\n\n\
         I found a similar pattern across two different topic areas:\n\n\
         In '{topic_a}' (memory/graphs/{topic_a}.json):\n\
           '{}' {} '{}' ({:.0}% confidence)\n\n\
         In '{topic_b}' (memory/graphs/{topic_b}.json):\n\
           '{}' {} '{}' ({:.0}% confidence)\n\n\
         Similarity: {}\n\n\
         Your task:\n\
         1. Analyse whether insights from one domain can strengthen the other\n\
         2. Can what we learned in '{topic_a}' inform our understanding in '{topic_b}', \
            or vice versa?\n\
         3. If the pattern transfer is valid, update the weaker edge with \
            evidence_type 'pattern_transfer'\n\
         4. Consider whether this reveals a deeper underlying principle — if so, \
            add it as a new 'principle' or 'concept' node\n\
         5. Update the relevant topic graph files\n\n\
         Cross-pollination of ideas across domains is how breakthroughs happen.{}",
        pattern.source_from, pattern.source_relation, pattern.source_to,
        pattern.source_confidence * 100.0,
        pattern.target_from, pattern.target_relation, pattern.target_to,
        pattern.target_confidence * 100.0,
        pattern.similarity_reason,
        RUMINATION_STOP_DIRECTIVE,
    );

    let _ = request_tx.send(CliRequest {
        chat_id: RUMINATION_CHAT_ID,
        message: prompt,
        new_session: true,
        task_id: 0,
        source: "rumination".into(),
    });

    log.append(RuminationEntry {
        timestamp: chrono_now(),
        kind: "pattern_transfer".into(),
        topic: format!("{} ↔ {}", topic_a, topic_b),
        description: format!(
            "Pattern: '{}' in {} ≈ '{}' in {}",
            pattern.source_relation, topic_a,
            pattern.target_relation, topic_b,
        ),
        edges_created: 0,
        edges_updated: 0,
    }, &config.memory_dir);
}

// ── Synthesis (Phase 2) ─────────────────────────────────────────────

/// Run idea synthesis on all topic graphs. Returns number of edges created.
fn run_synthesis(config: &MaintenanceConfig, log: &mut RuminationLog) -> u32 {
    let graphs_dir = config.memory_dir.join("graphs");
    if !graphs_dir.exists() { return 0; }

    let mut total_created = 0;

    let entries = match std::fs::read_dir(&graphs_dir) {
        Ok(e) => e,
        Err(_) => return 0,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !is_topic_graph(&path) { continue; }

        let topic = topic_name(&path);
        if !config.rumination.topics.is_empty()
            && !config.rumination.topics.iter().any(|t| t == &topic)
        {
            continue;
        }

        let mut kg = crate::knowledge::KnowledgeGraph::load(&path);
        if kg.node_count() < 3 { continue; }

        let candidates = kg.synthesis_candidates(5);
        if candidates.is_empty() { continue; }

        let now = chrono_now();
        let mut created = 0u32;

        for (a_idx, c_idx, b_label, r1, r2) in &candidates {
            let relation = format!("{} (via {})", r1, b_label);
            let context = format!(
                "Synthesised: {} → {} and {} → {} imply this transitive link",
                r1, b_label, b_label, r2
            );
            let edge = crate::knowledge::KnowledgeEdge::new(
                &relation,
                &context,
                &now,
                crate::knowledge::Basis::Inferred,
            );
            kg.add_edge(*a_idx, *c_idx, edge);
            created += 1;
        }

        if created > 0 {
            kg.save();
            broadcast_graph_update(config, &topic, "rumination");
            total_created += created;

            let a_labels: Vec<String> = candidates.iter()
                .take(3)
                .map(|(_, _, b, r1, _)| format!("{} via {}", r1, b))
                .collect();
            log.append(RuminationEntry {
                timestamp: now.clone(),
                kind: "synthesis".into(),
                topic: topic.clone(),
                description: format!("Created {} transitive edges: {}", created, a_labels.join("; ")),
                edges_created: created,
                edges_updated: 0,
            }, &config.memory_dir);
        }
    }

    total_created
}

// ── Active Refutation (Phase 3) ─────────────────────────────────────

/// Pick beliefs to challenge and send refutation prompts to the AI.
fn run_refutation(
    config: &MaintenanceConfig,
    request_tx: &mpsc::UnboundedSender<CliRequest>,
    log: &mut RuminationLog,
) {
    let graphs_dir = config.memory_dir.join("graphs");
    if !graphs_dir.exists() { return; }

    let entries = match std::fs::read_dir(&graphs_dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    let mut all_candidates: Vec<(String, String, String, String, f64, f64)> = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if !is_topic_graph(&path) { continue; }

        let topic = topic_name(&path);
        if !config.rumination.topics.is_empty()
            && !config.rumination.topics.iter().any(|t| t == &topic)
        {
            continue;
        }

        let kg = crate::knowledge::KnowledgeGraph::load(&path);
        for (from, to, relation, confidence, importance) in kg.refutation_candidates(3) {
            all_candidates.push((topic.clone(), from, to, relation, confidence, importance));
        }
    }

    // Sort by priority: importance × (1 - confidence) descending.
    all_candidates.sort_by(|a, b| {
        let score_a = a.5 * (1.0 - a.4);
        let score_b = b.5 * (1.0 - b.4);
        score_b.partial_cmp(&score_a).unwrap_or(std::cmp::Ordering::Equal)
    });

    // Send top 2 refutation prompts.
    for (topic, from, to, relation, confidence, _importance) in all_candidates.iter().take(2) {
        let prompt = format!(
            "RUMINATION — ACTIVE REFUTATION\n\n\
             I currently believe: '{}' {} '{}' (confidence: {:.0}%)\n\
             This belief is in the topic graph: memory/graphs/{}.json\n\n\
             Your task — ATTEMPT TO REFUTE THIS BELIEF:\n\
             1. Formulate specific ways this could be WRONG\n\
             2. Search for evidence that would DISPROVE this claim\n\
             3. Check the knowledge graph for inconsistencies\n\n\
             THREE POSSIBLE OUTCOMES (be honest about which applies):\n\n\
             A) You found specific evidence that COULD have disproved this but DIDN'T:\n\
                → Use evidence_type 'refutation_survived' (BF=2.5 — genuinely strengthens)\n\
                → Record WHAT you tested and WHY it failed to disprove\n\n\
             B) You found evidence that DOES disprove or seriously undermine this:\n\
                → Use evidence_type 'refutation_failed' (BF=0.1 — sharply weakens)\n\
                → Record the contradicting evidence\n\n\
             C) You searched but found NOTHING relevant either way:\n\
                → Use evidence_type 'inconsequential_search' (BF=1.0 — NO CHANGE)\n\
                → Absence of counter-evidence does NOT strengthen the belief\n\
                → An untested idea remains untested\n\n\
             CRITICAL: Do NOT use 'refutation_survived' just because you didn't find \
             anything wrong. That would be confirmation bias. Only use it when you found \
             specific evidence that COULD HAVE refuted the idea but FAILED TO.\n\n\
             Update the topic graph file with your findings.{}",
            from, relation, to, confidence * 100.0, topic, RUMINATION_STOP_DIRECTIVE
        );

        let _ = request_tx.send(CliRequest {
            chat_id: RUMINATION_CHAT_ID,
            message: prompt,
            new_session: true,
            task_id: 0, // assigned by worker
            source: "rumination".into(),
        });

        log.append(RuminationEntry {
            timestamp: chrono_now(),
            kind: "refutation".into(),
            topic: topic.clone(),
            description: format!(
                "Challenging: '{}' {} '{}' ({:.0}%)",
                from, relation, to, confidence * 100.0
            ),
            edges_created: 0,
            edges_updated: 0, // updated after AI responds
        }, &config.memory_dir);
    }
}

// ── Contradiction Resolution (Phase 4) ──────────────────────────────

/// Find contradictions and send resolution prompts to the AI.
fn run_contradiction_resolution(
    config: &MaintenanceConfig,
    request_tx: &mpsc::UnboundedSender<CliRequest>,
    log: &mut RuminationLog,
) {
    let graphs_dir = config.memory_dir.join("graphs");
    if !graphs_dir.exists() { return; }

    let entries = match std::fs::read_dir(&graphs_dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    let mut sent = 0u32;

    for entry in entries.flatten() {
        let path = entry.path();
        if !is_topic_graph(&path) { continue; }

        let topic = topic_name(&path);
        if !config.rumination.topics.is_empty()
            && !config.rumination.topics.iter().any(|t| t == &topic)
        {
            continue;
        }

        let kg = crate::knowledge::KnowledgeGraph::load(&path);
        let pairs = kg.contradiction_pairs();

        for pair in pairs.iter().take(1) {
            let prompt = format!(
                "RUMINATION — CONTRADICTION RESOLUTION\n\n\
                 I hold two contradictory beliefs in topic graph: memory/graphs/{}.json\n\n\
                 Belief A: '{}' ↔ '{}' via '{}' ({:.0}% confidence)\n\
                   Context: {}\n\n\
                 Belief B: '{}' ↔ '{}' via '{}' ({:.0}% confidence)\n\
                   Context: {}\n\n\
                 Your task:\n\
                 1. Evaluate which belief is more likely correct based on evidence and reasoning\n\
                 2. Strengthen the winner with 'corroboration' evidence\n\
                 3. Weaken the loser with 'contradiction' evidence\n\
                 4. If both can coexist (apparent contradiction), add context explaining how\n\
                 5. Update the topic graph file\n\n\
                 Let the stronger idea survive.{}",
                topic,
                pair.node_a_label, pair.node_b_label, pair.edge_a_relation,
                pair.edge_a_confidence * 100.0, pair.edge_a_context,
                pair.node_a_label, pair.node_b_label, pair.edge_b_relation,
                pair.edge_b_confidence * 100.0, pair.edge_b_context,
                RUMINATION_STOP_DIRECTIVE,
            );

            let _ = request_tx.send(CliRequest {
                chat_id: RUMINATION_CHAT_ID,
                message: prompt,
                new_session: true,
                task_id: 0,
                source: "rumination".into(),
            });

            log.append(RuminationEntry {
                timestamp: chrono_now(),
                kind: "contradiction".into(),
                topic: topic.clone(),
                description: format!(
                    "Resolving: '{}' ({:.0}%) vs '{}' ({:.0}%) between {} ↔ {}",
                    pair.edge_a_relation, pair.edge_a_confidence * 100.0,
                    pair.edge_b_relation, pair.edge_b_confidence * 100.0,
                    pair.node_a_label, pair.node_b_label,
                ),
                edges_created: 0,
                edges_updated: 0,
            }, &config.memory_dir);

            sent += 1;
        }

        if sent >= 2 { break; } // Limit per cycle.
    }
}

// ── Autonomous Initiative (Phase 5) ─────────────────────────────────

/// Find weak spots in the knowledge and send improvement prompts.
fn run_initiative(
    config: &MaintenanceConfig,
    request_tx: &mpsc::UnboundedSender<CliRequest>,
    log: &mut RuminationLog,
) {
    let graphs_dir = config.memory_dir.join("graphs");
    if !graphs_dir.exists() { return; }

    // Find the topic graph with the most uncertain edges.
    let mut weakest_topic: Option<(String, f64, usize)> = None;

    let entries = match std::fs::read_dir(&graphs_dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !is_topic_graph(&path) { continue; }

        let topic = topic_name(&path);
        if !config.rumination.topics.is_empty()
            && !config.rumination.topics.iter().any(|t| t == &topic)
        {
            continue;
        }

        let kg = crate::knowledge::KnowledgeGraph::load(&path);
        let stats = kg.uncertainty_stats();

        // Score: more uncertain edges and lower average confidence = weaker.
        if stats.edge_count > 0 {
            let weakness_score = stats.uncertain_edge_count as f64 / stats.edge_count as f64;
            match &weakest_topic {
                Some((_, best_score, _)) if weakness_score <= *best_score => {}
                _ => weakest_topic = Some((topic, weakness_score, stats.uncertain_edge_count)),
            }
        }
    }

    let Some((topic, _score, uncertain_count)) = weakest_topic else { return };

    let prompt = format!(
        "RUMINATION — AUTONOMOUS INITIATIVE\n\n\
         Review the topic graph 'memory/graphs/{}.json' and improve it.\n\
         This topic has {} uncertain edges that need attention.\n\n\
         Your task:\n\
         1. Read the topic graph file\n\
         2. Identify gaps in reasoning chains — are there missing links?\n\
         3. Look for edges that should exist but don't\n\
         4. Can you infer new relationships from existing strong ones?\n\
         5. Are there nodes that need better summaries or are miscategorised?\n\
         6. Strengthen well-supported edges with 'consistency' evidence\n\
         7. Weaken poorly-supported ones with 'inconsistency' evidence\n\n\
         Focus on quality over quantity. Every change should be a testable conjecture.{}",
        topic, uncertain_count, RUMINATION_STOP_DIRECTIVE
    );

    let _ = request_tx.send(CliRequest {
        chat_id: RUMINATION_CHAT_ID,
        message: prompt,
        new_session: true,
        task_id: 0,
        source: "rumination".into(),
    });

    log.append(RuminationEntry {
        timestamp: chrono_now(),
        kind: "initiative".into(),
        topic: topic.clone(),
        description: format!("Self-improving topic '{}' ({} uncertain edges)", topic, uncertain_count),
        edges_created: 0,
        edges_updated: 0,
    }, &config.memory_dir);
}

// ── Existing maintenance functions ──────────────────────────────────

/// Consolidate all topic graphs and the meta-graph.
// ── Meta-Rumination (Self-Modification) ─────────────────────────────

/// Review and evolve the ANT's own thinking process.
/// The thinking process itself is a conjecture — open to improvement.
fn run_meta_rumination(
    config: &MaintenanceConfig,
    request_tx: &mpsc::UnboundedSender<CliRequest>,
    log: &mut RuminationLog,
) {
    // Only run meta-rumination occasionally — every ~5 cycles.
    // Check if the rumination log has enough entries to warrant self-review.
    let rumination_count = log.entries.len();
    if rumination_count < 10 || rumination_count % 5 != 0 {
        return;
    }

    // Gather recent rumination stats for the prompt.
    let recent = &log.entries[log.entries.len().saturating_sub(20)..];
    let inconsequential = recent.iter().filter(|e| e.description.contains("inconsequential")).count();
    let total_recent = recent.len();

    let thinking_process_file = config.memory_dir.join("thinking_process.md");
    let has_process = thinking_process_file.exists();

    let prompt = format!(
        "RUMINATION — META-COGNITION (self-review)\n\n\
         Review your own thinking process and consider whether it can be improved.\n\n\
         Recent rumination stats ({} entries in last batch):\n\
         - {} resulted in 'inconsequential_search' (found nothing)\n\
         - That's {:.0}% inconsequential rate\n\n\
         Your thinking process file: memory/thinking_process.md {}\n\
         Your meta-cognition graph: memory/graphs/meta-cognition.json\n\n\
         Consider:\n\
         1. Are your refutation attempts actually rigorous, or are you just searching\n\
            broadly and finding nothing? If many are inconsequential, your strategy\n\
            needs improvement.\n\
         2. Are you selecting the RIGHT beliefs to test? Maybe you should focus on\n\
            beliefs that are more central, or more recent, or more likely to be wrong.\n\
         3. Is your evidence evaluation honest? Are you inflating confidence because\n\
            ideas 'seem right'? Or being too harsh?\n\
         4. What worked well? What didn't? Record observations in the meta-cognition\n\
            topic graph.\n\
         5. If you identify an improvement to your process, update thinking_process.md.\n\
            Include: what you changed, why, and what you expect to improve.\n\
         6. Every change to your thinking process is itself a CONJECTURE — it should be\n\
            tested and refined, not assumed to be better.\n\n\
         The goal: evolve a thinking process that grows STRONGER ideas — ideas that\n\
         survive genuine scrutiny, are well-sourced, well-corroborated, and beneficial\n\
         for people and the planet.{}",
        total_recent,
        inconsequential,
        if total_recent > 0 { inconsequential as f64 / total_recent as f64 * 100.0 } else { 0.0 },
        if has_process { "(exists — review and improve)" } else { "(doesn't exist yet — create it)" },
        RUMINATION_STOP_DIRECTIVE,
    );

    let _ = request_tx.send(CliRequest {
        chat_id: RUMINATION_CHAT_ID,
        message: prompt,
        new_session: true,
        task_id: 0,
        source: "rumination".into(),
    });

    log.append(RuminationEntry {
        timestamp: chrono_now(),
        kind: "meta".into(),
        topic: "meta-cognition".into(),
        description: format!(
            "Self-review: {}/{} recent ruminations inconsequential ({:.0}%)",
            inconsequential, total_recent,
            if total_recent > 0 { inconsequential as f64 / total_recent as f64 * 100.0 } else { 0.0 },
        ),
        edges_created: 0,
        edges_updated: 0,
    }, &config.memory_dir);
}

// ── Existing maintenance functions ──────────────────────────────────

fn run_consolidation(config: &MaintenanceConfig) {
    let graphs_dir = config.memory_dir.join("graphs");
    let meta_path = config.memory_dir.join("knowledge.json");

    // Consolidate meta-graph.
    if meta_path.exists() {
        let mut kg = crate::knowledge::KnowledgeGraph::load(&meta_path);
        if kg.node_count() > 0 {
            let report = kg.consolidate();
            if report.nodes_merged > 0 || report.edges_merged > 0 || report.chains_collapsed > 0 {
                kg.save();
                broadcast_graph_update(config, "meta", "consolidation");
                log::info!("[{}] Meta-graph consolidated: {} merged, {} edges merged, {} collapsed",
                    config.ant_name, report.nodes_merged, report.edges_merged, report.chains_collapsed);
            }
            for warning in &report.contradictions {
                log::warn!("[{}] Meta-graph contradiction: {}", config.ant_name, warning);
            }
            for cluster in &report.clusters {
                if cluster.len() >= 3 {
                    log::info!("[{}] Meta-graph cluster: {}", config.ant_name, cluster.join(", "));
                }
            }
        }
    }

    // Consolidate each topic graph.
    if graphs_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&graphs_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map(|e| e == "json").unwrap_or(false)
                    && !path.to_string_lossy().contains("-archive")
                {
                    let topic = path.file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default();
                    let mut kg = crate::knowledge::KnowledgeGraph::load(&path);
                    if kg.node_count() == 0 { continue; }

                    let report = kg.consolidate();
                    // Link orphan nodes to the graph's hub.
                    kg.link_orphans(&topic);
                    if report.nodes_merged > 0 || report.edges_merged > 0 {
                        log::info!("[{}] Topic '{}' consolidated: {} merged, {} edges merged",
                            config.ant_name, topic, report.nodes_merged, report.edges_merged);
                    }
                    kg.save();
                    broadcast_graph_update(config, &topic, "consolidation");
                    for warning in &report.contradictions {
                        log::warn!("[{}] Topic '{}' contradiction: {}", config.ant_name, topic, warning);
                    }
                }
            }
        }
    }
}

/// Cross-link topic graphs: find entities that appear in multiple topics
/// and add cross-reference edges in the meta-graph.
fn run_cross_linking(config: &MaintenanceConfig) {
    let graphs_dir = config.memory_dir.join("graphs");
    let meta_path = config.memory_dir.join("knowledge.json");

    if !graphs_dir.exists() { return; }

    // Collect all entity labels per topic.
    let mut topic_entities: Vec<(String, Vec<String>)> = Vec::new();

    if let Ok(entries) = std::fs::read_dir(&graphs_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e == "json").unwrap_or(false)
                && !path.to_string_lossy().contains("-archive")
            {
                let topic = path.file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();
                let kg = crate::knowledge::KnowledgeGraph::load(&path);
                let labels: Vec<String> = kg.all_node_labels();
                if !labels.is_empty() {
                    topic_entities.push((topic, labels));
                }
            }
        }
    }

    if topic_entities.len() < 2 { return; }

    // Find shared entities between topics.
    let mut shared_pairs: Vec<(String, String, Vec<String>)> = Vec::new();
    for i in 0..topic_entities.len() {
        for j in (i + 1)..topic_entities.len() {
            let (ref topic_a, ref labels_a) = topic_entities[i];
            let (ref topic_b, ref labels_b) = topic_entities[j];
            let shared: Vec<String> = labels_a.iter()
                .filter(|l| labels_b.iter().any(|b| b.to_lowercase() == l.to_lowercase()))
                .cloned()
                .collect();
            if !shared.is_empty() {
                shared_pairs.push((topic_a.clone(), topic_b.clone(), shared));
            }
        }
    }

    if shared_pairs.is_empty() { return; }

    // Update the meta-graph with cross-reference edges.
    let mut meta = crate::knowledge::KnowledgeGraph::load(&meta_path);
    let mut added = 0;

    for (topic_a, topic_b, shared) in &shared_pairs {
        // Ensure topic nodes exist.
        let idx_a = meta.find_by_label(topic_a).unwrap_or_else(|| {
            meta.add_topic_node(topic_a)
        });
        let idx_b = meta.find_by_label(topic_b).unwrap_or_else(|| {
            meta.add_topic_node(topic_b)
        });

        // Check if a "shares_entities" edge already exists.
        let already_linked = meta.has_edge_between(idx_a, idx_b, "shares_entities");
        if !already_linked {
            let context = format!("Shared entities: {}", shared.join(", "));
            meta.add_cross_link(idx_a, idx_b, &context);
            added += 1;
        }
    }

    if added > 0 {
        meta.rebuild_index();
        meta.save();
        broadcast_graph_update(config, "meta", "consolidation");
        log::info!("[{}] Cross-linked {} topic pairs in meta-graph", config.ant_name, added);
    }
}

// ── Helpers ─────────────────────────────────────────────────────────

/// Build a short human-readable summary of the most recent rumination entries.
fn build_rumination_summary(log: &RuminationLog) -> String {
    // Gather entries from this cycle (last few entries with recent timestamps).
    let recent: Vec<&RuminationEntry> = log.entries.iter().rev().take(6).collect();
    if recent.is_empty() { return String::new(); }

    let mut lines = Vec::new();
    lines.push("**Rumination update** — here's what I was thinking about while idle:".to_string());

    for entry in recent.iter().rev() {
        let icon = match entry.kind.as_str() {
            "refutation" => "?",
            "synthesis" => "+",
            "contradiction" => "!",
            "competition" => "vs",
            "pattern_transfer" => "~",
            "initiative" => "*",
            _ => "-",
        };
        lines.push(format!("  [{}] {}: {}", icon, entry.topic, entry.description));
    }

    lines.push(String::new());
    lines.push("_(Use the rumination log in the dashboard for full details)_".to_string());

    lines.join("\n")
}

/// Post a rumination summary to the chat history file.
/// Writes directly to the JSONL file so the human sees it in the chat.
fn post_to_chat_history(config: &MaintenanceConfig, summary: &str) {
    // Find the history directory — it's at the supervisor level, not per-ANT.
    // The history files are stored as <ant_name>.jsonl in the history dir.
    // Convention: ~/.config/anthill/history/<ant_name>.jsonl
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let history_file = std::path::PathBuf::from(&home)
        .join(".config/anthill/history")
        .join(format!("{}.jsonl", config.ant_name));

    if !history_file.parent().map(|p| p.exists()).unwrap_or(false) {
        return; // History dir doesn't exist — supervisor not using history.
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let msg = crate::history::ChatMessage {
        role: "bot".into(),
        text: summary.into(),
        task_id: 0,
        timestamp: now,
    };

    if let Ok(json) = serde_json::to_string(&msg) {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&history_file)
        {
            let _ = writeln!(f, "{}", json);
        }
    }
}

/// Broadcast a graph update event so the dashboard refreshes in real time.
fn broadcast_graph_update(config: &MaintenanceConfig, graph_name: &str, source: &str) {
    if let Some(ref tx) = config.event_tx {
        let _ = tx.send(WsEvent::GraphUpdated {
            bot: config.ant_name.clone(),
            graph: graph_name.into(),
            source: source.into(),
        });
    }
}

fn is_topic_graph(path: &std::path::Path) -> bool {
    path.extension().map(|e| e == "json").unwrap_or(false)
        && !path.to_string_lossy().contains("-archive")
}

fn topic_name(path: &std::path::Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default()
}

fn chrono_now() -> String {
    // Simple ISO date without chrono dependency.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Convert to YYYY-MM-DD HH:MM:SS (approximate — good enough for logging).
    let days = now / 86400;
    let secs_today = now % 86400;
    let hours = secs_today / 3600;
    let minutes = (secs_today % 3600) / 60;
    let seconds = secs_today % 60;

    // Days since epoch to date — simplified calculation.
    let mut y = 1970i64;
    let mut remaining = days as i64;
    loop {
        let days_in_year = if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) { 366 } else { 365 };
        if remaining < days_in_year { break; }
        remaining -= days_in_year;
        y += 1;
    }
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let month_days = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut m = 0;
    for (i, &md) in month_days.iter().enumerate() {
        if remaining < md as i64 { m = i + 1; break; }
        remaining -= md as i64;
    }
    let d = remaining + 1;

    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", y, m, d, hours, minutes, seconds)
}
