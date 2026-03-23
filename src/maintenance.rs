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
use crate::store::KnowledgeStore;
use crate::store::live::LiveKnowledgeStore;
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
    let store = crate::store::live::LiveKnowledgeStore::new(config.memory_dir.clone());

    // Each rumination phase is an atomic "thought" — one commit per phase.

    // 0. Compute corroboration strength.
    store.begin_thought();
    if let Ok(graphs) = store.list_graphs() {
        for g in &graphs {
            if g.node_count >= 2 {
                let _ = store.compute_corroboration_strength(&g.name);
            }
        }
    }
    let _ = store.end_thought(&format!("[{}] corroboration strength updated", config.ant_name));

    // 1. Synthesis — cheap, no AI tokens.
    if config.rumination.synthesis_enabled {
        store.begin_thought();
        let count = run_synthesis(config, &store, &mut log);
        if count > 0 {
            let _ = store.end_thought(&format!("[{}] synthesis: created {} transitive edges", config.ant_name, count));
            log::info!("[{}] Synthesis created {} new edges", config.ant_name, count);
        } else {
            let _ = store.end_thought(&format!("[{}] synthesis: no candidates", config.ant_name));
        }
    }

    // 1b. Investigate undetermined connections ('?' edges).
    run_undetermined_connections(config, &store, request_tx, &mut log);

    // 2. Competition — pit similar ideas against each other.
    run_competition(config, &store, request_tx, &mut log);

    // 3. Cross-domain pattern transfer.
    run_pattern_transfer(config, &store, request_tx, &mut log);

    // 4. Active refutation.
    if config.rumination.refutation_enabled {
        run_refutation(config, &store, request_tx, &mut log);
    }

    // 5. Contradiction resolution.
    if config.rumination.contradiction_resolution {
        run_contradiction_resolution(config, &store, request_tx, &mut log);
    }

    // 6. Autonomous initiative.
    if config.rumination.initiative_enabled {
        run_initiative(config, &store, request_tx, &mut log);
    }

    // 7. Citation consolidation — ensure sources are tracked and linked.
    run_citation_consolidation(config, &store, request_tx, &mut log);

    // 8. Meta-rumination.
    run_meta_rumination(config, &store, request_tx, &mut log);

    // Drop the store to release locks before consolidation.
    drop(store);

    // Post a short summary to the chat history so the human can see what happened.
    let summary = build_rumination_summary(&log);
    if !summary.is_empty() {
        post_to_chat_history(config, &summary);
    }

    // Consolidate after rumination — link orphans, dedup, keep things tidy.
    run_consolidation(config);

    // Git commit after rumination — creates a meaningful restore point.
    git_commit_memory(config, "rumination cycle complete");

    log::info!("[{}] Rumination cycle complete", config.ant_name);
}

// ── Undetermined Connections ─────────────────────────────────────────

/// Find '?' edges (undetermined connections) and ask the AI to investigate them.
fn run_undetermined_connections(
    config: &MaintenanceConfig,
    store: &LiveKnowledgeStore,
    request_tx: &mpsc::UnboundedSender<CliRequest>,
    log: &mut RuminationLog,
) {
    let mut sent = 0u32;

    // Collect all undetermined connections across all topics.
    let mut all_undetermined: Vec<(String, String, String)> = Vec::new();
    for topic in filtered_topics(config, store) {
        let undetermined = match store.undetermined_connections(&topic, 10) {
            Ok(u) => u,
            Err(_) => continue,
        };
        for (from, to) in undetermined {
            all_undetermined.push((topic.clone(), from, to));
        }
    }

    if all_undetermined.is_empty() { return; }

    // Batch multiple ? edges into a single prompt for efficiency.
    let batch_size = all_undetermined.len().min(8);
    let batch = &all_undetermined[..batch_size];

    let mut edge_list = String::new();
    for (topic, from, to) in batch {
        edge_list.push_str(&format!("  - '{}' ↔ '{}' (in {})\n", from, to, topic));
    }

    let prompt = format!(
        "RUMINATION — UNDETERMINED CONNECTIONS\n\n\
         The following {} connections have relation '?' — they exist in the graph but \
         their relationship hasn't been established:\n\n{}\n\
         For EACH connection:\n\
         1. Search the web or your knowledge for how these concepts relate\n\
         2. Look at their other connections in the graph for context\n\
         3. If you can determine a relationship:\n\
            - Replace the '?' edge with a proper relation name\n\
            - Set basis to 'inferred' or 'observed' (if you found a source)\n\
            - Add a citation if you found an external source\n\
         4. If they genuinely don't relate, REMOVE the '?' edge entirely —\n\
            don't leave meaningless connections cluttering the graph\n\
         5. Update all affected topic graph files{}",
        batch_size, edge_list, RUMINATION_STOP_DIRECTIVE
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
        topic: "multiple".into(),
        description: format!("Investigating {} undetermined connections", batch_size),
        edges_created: 0,
        edges_updated: 0,
    }, &config.memory_dir);

    sent = batch_size as u32;
}

// ── Darwinian Competition ───────────────────────────────────────────

/// Find competing hypotheses and send research prompts to evaluate them.
fn run_competition(
    config: &MaintenanceConfig,
    store: &LiveKnowledgeStore,
    request_tx: &mpsc::UnboundedSender<CliRequest>,
    log: &mut RuminationLog,
) {
    let mut sent = 0u32;

    for topic in filtered_topics(config, store) {
        let groups = match store.find_competitors(&topic) {
            Ok(g) => g,
            Err(_) => continue,
        };

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
    store: &LiveKnowledgeStore,
    request_tx: &mpsc::UnboundedSender<CliRequest>,
    log: &mut RuminationLog,
) {
    let topics = filtered_topics(config, store);
    if topics.len() < 2 { return; }

    // Compare each pair of topic graphs for patterns.
    let mut best_match: Option<(String, String, crate::knowledge::PatternMatch)> = None;

    for i in 0..topics.len() {
        for j in (i + 1)..topics.len() {
            if let Ok(patterns) = store.cross_domain_patterns(&topics[i], &topics[j], 1) {
                if let Some(pattern) = patterns.into_iter().next() {
                    if best_match.is_none() {
                        best_match = Some((topics[i].clone(), topics[j].clone(), pattern));
                    }
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
fn run_synthesis(config: &MaintenanceConfig, store: &LiveKnowledgeStore, log: &mut RuminationLog) -> u32 {
    let mut total_created = 0;

    for topic in filtered_topics(config, store) {
        let candidates = match store.synthesis_candidates(&topic, 5) {
            Ok(c) if !c.is_empty() => c,
            _ => continue,
        };

        let now = chrono_now();
        let mut created = 0u32;

        for (a_id, c_id, b_label, r1, r2) in &candidates {
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
            let _ = store.add_edge_by_id(&topic, *a_id, *c_id, edge);
            created += 1;
        }

        if created > 0 {
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
    store: &LiveKnowledgeStore,
    request_tx: &mpsc::UnboundedSender<CliRequest>,
    log: &mut RuminationLog,
) {
    let mut all_candidates: Vec<(String, String, String, String, f64, f64)> = Vec::new();

    for topic in filtered_topics(config, store) {
        if let Ok(candidates) = store.refutation_candidates(&topic, 3) {
            for (from, to, relation, confidence, importance) in candidates {
                all_candidates.push((topic.clone(), from, to, relation, confidence, importance));
            }
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
             Your task — ATTEMPT TO REFUTE THIS BELIEF WITH EXTERNAL EVIDENCE:\n\n\
             STEP 1 — Search for counter-evidence OUTSIDE the knowledge graph:\n\
             - Use web search to find sources that CONTRADICT or CHALLENGE this claim\n\
             - PRIORITISE high-quality sources: peer-reviewed papers, official reports,\n\
               authoritative books. These carry more weight than blog posts or opinions.\n\
             - Look for recent research, expert analysis, or data that disagrees\n\
             - Search for exceptions, edge cases, or contexts where this doesn't hold\n\
             - If you find a relevant source, fetch it and read the actual content —\n\
               what matters is the IDEAS within the source and whether they hold up\n\
             - Save useful sources to files/ for future reference\n\n\
             STEP 2 — Also check WITHIN the knowledge graph:\n\
             - Look for inconsistencies with other beliefs\n\
             - Check if the evidence trail is one-sided (all confirmations, no challenges)\n\n\
             STEP 3 — Evaluate honestly and record your findings:\n\n\
             THREE POSSIBLE OUTCOMES:\n\n\
             A) You found specific external evidence that COULD have disproved this but DIDN'T:\n\
                → Use evidence_type 'refutation_survived' (BF=2.5 — genuinely strengthens)\n\
                → Record WHAT source you checked, WHAT it said, and WHY it failed to disprove\n\
                → Add the source as a citation on the edge using graph_add_citation\n\n\
             B) You found evidence that DOES disprove or seriously undermine this:\n\
                → Use evidence_type 'refutation_failed' (BF=0.1 — sharply weakens)\n\
                → Record the contradicting evidence and its source\n\
                → Add the source as a citation\n\n\
             C) You searched but found NOTHING relevant either way:\n\
                → Use evidence_type 'inconsequential_search' (BF=1.0 — NO CHANGE)\n\
                → Absence of counter-evidence does NOT strengthen the belief\n\
                → An untested idea remains untested\n\n\
             CRITICAL: Do NOT use 'refutation_survived' just because you didn't find \
             anything wrong in your own reasoning. That is confirmation bias — an echo \
             chamber of self-agreement. Only use it when you found a SPECIFIC EXTERNAL \
             source that COULD HAVE refuted the idea but FAILED TO.{}",
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
    store: &LiveKnowledgeStore,
    request_tx: &mpsc::UnboundedSender<CliRequest>,
    log: &mut RuminationLog,
) {
    let mut sent = 0u32;

    for topic in filtered_topics(config, store) {
        let pairs = match store.contradiction_pairs(&topic) {
            Ok(p) => p,
            Err(_) => continue,
        };

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
    store: &LiveKnowledgeStore,
    request_tx: &mpsc::UnboundedSender<CliRequest>,
    log: &mut RuminationLog,
) {
    // Find the topic graph with the most uncertain edges.
    let mut weakest_topic: Option<(String, f64, usize)> = None;

    for topic in filtered_topics(config, store) {
        if let Ok(stats) = store.uncertainty_stats(&topic) {
            if stats.edge_count > 0 {
                let weakness_score = stats.uncertain_edge_count as f64 / stats.edge_count as f64;
                match &weakest_topic {
                    Some((_, best_score, _)) if weakness_score <= *best_score => {}
                    _ => weakest_topic = Some((topic, weakness_score, stats.uncertain_edge_count)),
                }
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
         2. Identify gaps — are there missing relationships or concepts?\n\
         3. For uncertain edges, search the web for evidence that supports OR refutes them\n\
         4. If you find a useful source, fetch it, save it to files/, and add it as a citation\n\
         5. Add new conjectures based on external evidence you find (not just inference)\n\
         6. Strengthen edges where you find supporting external sources (use 'corroboration')\n\
         7. Weaken edges where you find contradicting external sources (use 'inconsistency')\n\
         8. For edges based purely on AI inference with no external backing, look for real sources\n\n\
         IMPORTANT: Go beyond internal reasoning. Use web search to find real-world evidence. \
         An idea backed by external sources is stronger than one backed only by AI inference. \
         Every change should be a testable conjecture with provenance.{}",
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

// ── Citation Consolidation ───────────────────────────────────────────

/// Ensure citation sources are tracked in the citations graph and linked to topic graph edges.
/// This is a core maintenance task — every ANT should have a well-maintained citations graph.
fn run_citation_consolidation(
    config: &MaintenanceConfig,
    store: &LiveKnowledgeStore,
    request_tx: &mpsc::UnboundedSender<CliRequest>,
    log: &mut RuminationLog,
) {
    // Count uncited edges across topic graphs to decide if consolidation is needed.
    let mut uncited_edges = 0u32;
    let mut total_edges = 0u32;
    let mut topics_with_edges: Vec<String> = Vec::new();

    for topic in filtered_topics(config, store) {
        if let Ok(stats) = store.uncertainty_stats(&topic) {
            if stats.edge_count > 0 {
                total_edges += stats.edge_count as u32;
                topics_with_edges.push(topic);
            }
        }
    }

    // Also check if citations graph exists and has unresolved '?' edges.
    let has_citations_graph = store.list_graphs()
        .map(|gs| gs.iter().any(|g| g.name == "citations"))
        .unwrap_or(false);

    // Check for edges lacking citations across all topic graphs.
    for topic in &topics_with_edges {
        if let Ok(viz) = store.to_visualization(topic) {
            if let Some(links) = viz.get("links").and_then(|l| l.as_array()) {
                for link in links {
                    let has_cite = link.get("citations")
                        .and_then(|c| c.as_array())
                        .map(|a| !a.is_empty())
                        .unwrap_or(false);
                    if !has_cite {
                        uncited_edges += 1;
                    }
                }
            }
        }
    }

    // Only run if there's meaningful work: no citations graph yet, or many uncited edges.
    if !has_citations_graph || uncited_edges > total_edges / 3 {
        let topics_list = topics_with_edges.join(", ");
        let prompt = format!(
            "RUMINATION — CITATION ANALYSIS\n\n\
             Currently {} of {} edges lack citations. Topic graphs: {}.\n\n\
             1. For each citation in the citations graph that has a URL:\n\
                - Check files/ first, otherwise fetch and save to files/\n\
                - Extract the TOP 3 CORE IDEAS from the source content\n\
                - Store as the node summary: 'Core ideas: (1) ... (2) ... (3) ...'\n\
                - Check what the source itself CITES — follow upstream to find\n\
                  more authoritative sources (peer-reviewed papers, official reports)\n\
             2. Compare core ideas across citations:\n\
                - Add 'corroborates' edges between citations with overlapping ideas\n\
                - Add 'cites' edges when one source references another\n\
                - Identify CORE CITATIONS that others reference — tag as 'core_source'\n\
             3. Link citations to topic graph edges using graph_add_citation:\n\
                - Match by core ideas, prefer core/upstream sources\n\
                - For edges with only ai_inference, search for real sources\n\
             4. WRITE ALL updated graph files{}",
            uncited_edges, total_edges, topics_list, RUMINATION_STOP_DIRECTIVE
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
            kind: "citations".into(),
            topic: "citations".into(),
            description: format!(
                "Citation consolidation: {}/{} edges uncited, citations graph {}",
                uncited_edges, total_edges,
                if has_citations_graph { "exists" } else { "will be created" }
            ),
            edges_created: 0,
            edges_updated: 0,
        }, &config.memory_dir);
    }
}

// ── Meta-Rumination (Self-Modification) ─────────────────────────────

/// Review and evolve the ANT's own thinking process.
/// The thinking process itself is a conjecture — open to improvement.
fn run_meta_rumination(
    config: &MaintenanceConfig,
    _store: &LiveKnowledgeStore,
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

// ── File Housekeeping ────────────────────────────────────────────────

/// Move stray graph files into graphs/ and clean up .corrupted/.tmp files.
fn run_file_housekeeping(config: &MaintenanceConfig) {
    let memory_dir = &config.memory_dir;
    let graphs_dir = memory_dir.join("graphs");
    let _ = std::fs::create_dir_all(&graphs_dir);

    // Files that belong in memory/ root — don't move these.
    let root_files = [
        "knowledge.json", "knowledge-archive.json",
        "episodes.json", "embeddings.json",
        "reputation.json", "questions.json",
        "rumination_log.json", "rumination.md",
        "thinking_process.md",
    ];

    let entries = match std::fs::read_dir(memory_dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    let mut moved = 0u32;
    let mut cleaned = 0u32;

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() { continue; }

        let filename = path.file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default();

        // Clean up .corrupted and .tmp files.
        if filename.ends_with(".corrupted") || filename.ends_with(".json.tmp") || filename.ends_with(".cbor.tmp") {
            let _ = std::fs::remove_file(&path);
            cleaned += 1;
            continue;
        }

        // Skip non-JSON files and known root files.
        if !filename.ends_with(".json") { continue; }
        if root_files.iter().any(|&f| filename == f) { continue; }
        // Skip per-user memory files (numeric chat IDs like "123456.md" — but these are .md not .json).
        // Skip any file that starts with a digit (user memory like "-1.json").
        if filename.starts_with('-') || filename.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) {
            continue;
        }

        // Check if it looks like a knowledge graph (has "nodes" and "edges" keys).
        let is_graph = std::fs::read_to_string(&path)
            .map(|c| c.contains("\"nodes\"") && c.contains("\"edges\""))
            .unwrap_or(false);
        if !is_graph { continue; }

        // Move to graphs/.
        let dest = graphs_dir.join(&filename);
        if dest.exists() {
            // Merge: load both, combine nodes/edges, save.
            log::info!("[{}] Stray graph '{}' already exists in graphs/ — skipping (manual merge needed)",
                config.ant_name, filename);
            continue;
        }

        if std::fs::rename(&path, &dest).is_ok() {
            moved += 1;
            log::info!("[{}] Moved stray graph '{}' → graphs/", config.ant_name, filename);
        }
    }

    // Recursively clean up .corrupted and .tmp files in memory/ and all subdirectories.
    cleaned += clean_temp_files_recursive(&config.memory_dir);

    if moved > 0 || cleaned > 0 {
        log::info!("[{}] Housekeeping: moved {} stray graphs, cleaned {} temp files",
            config.ant_name, moved, cleaned);
    }
}

/// Recursively clean .corrupted and .tmp files from a directory and all subdirectories.
fn clean_temp_files_recursive(dir: &std::path::Path) -> u32 {
    let mut cleaned = 0u32;
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return 0,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // Recurse into subdirectories.
            cleaned += clean_temp_files_recursive(&path);
        } else if path.is_file() {
            let filename = path.file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_default();
            if filename.ends_with(".corrupted") || filename.ends_with(".json.tmp")
                || filename.ends_with(".tmp")
            {
                if std::fs::remove_file(&path).is_ok() {
                    cleaned += 1;
                    log::debug!("Cleaned up: {}", path.display());
                }
            }
        }
    }
    cleaned
}

fn run_consolidation(config: &MaintenanceConfig) {
    // Housekeeping: move stray graph files into graphs/ and clean up corrupted files.
    run_file_housekeeping(config);

    // Use the store for consolidation.
    use crate::store::KnowledgeStore;
    let store = crate::store::live::LiveKnowledgeStore::new(config.memory_dir.clone());

    // Consolidate meta-graph.
    // First, extract misplaced nodes (non-topic nodes that don't belong in meta).
    if let Ok(relocated) = store.extract_misplaced_meta_nodes() {
        if relocated > 0 {
            log::info!("[{}] Relocated {} misplaced nodes from meta-graph to 'uncategorised' graph",
                config.ant_name, relocated);
        }
    }
    if let Ok(report) = store.consolidate("meta") {
        let _ = store.backfill_thurisaz("meta");
        let _ = store.link_orphans("meta");
        if report.nodes_merged > 0 || report.edges_merged > 0 || report.chains_collapsed > 0 {
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

    // Consolidate each topic graph.
    if let Ok(graphs) = store.list_graphs() {
        for g in &graphs {
            if g.name == "meta" { continue; } // already done
            if let Ok(report) = store.consolidate(&g.name) {
                let _ = store.backfill_thurisaz(&g.name);
                let _ = store.link_orphans(&g.name);
                if report.nodes_merged > 0 || report.edges_merged > 0 {
                    log::info!("[{}] Topic '{}' consolidated: {} merged, {} edges merged",
                        config.ant_name, g.name, report.nodes_merged, report.edges_merged);
                }
                broadcast_graph_update(config, &g.name, "consolidation");
                for warning in &report.contradictions {
                    log::warn!("[{}] Topic '{}' contradiction: {}", config.ant_name, g.name, warning);
                }
            }
        }
    }
}

/// Cross-link topic graphs: find entities that appear in multiple topics
/// and add cross-reference edges in the meta-graph.
fn run_cross_linking(config: &MaintenanceConfig) {
    let meta_path = config.memory_dir.join("knowledge.json");
    let store = LiveKnowledgeStore::new(config.memory_dir.clone());

    // Collect all entity labels per topic.
    let mut topic_entities: Vec<(String, Vec<String>)> = Vec::new();

    if let Ok(graphs) = store.list_graphs() {
        for g in &graphs {
            if g.name == "meta" { continue; }
            if let Ok(labels) = store.list_nodes(&g.name) {
                if !labels.is_empty() {
                    topic_entities.push((g.name.clone(), labels));
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

/// Commit memory changes to local git with a descriptive message.
fn git_commit_memory(config: &MaintenanceConfig, message: &str) {
    // The working directory (parent of memory/) is the git repo root.
    let working_dir = config.memory_dir.parent().unwrap_or(&config.memory_dir);

    // Stage all memory changes.
    let add_result = std::process::Command::new("git")
        .args(["add", "memory/"])
        .current_dir(working_dir)
        .output();

    if let Ok(output) = add_result {
        if !output.status.success() { return; }
    } else {
        return;
    }

    // Check if there's anything to commit.
    let status = std::process::Command::new("git")
        .args(["diff", "--cached", "--quiet"])
        .current_dir(working_dir)
        .output();

    let has_changes = status.map(|o| !o.status.success()).unwrap_or(false);
    if !has_changes { return; }

    // Commit with descriptive message.
    let commit_msg = format!("[{}] {}", config.ant_name, message);
    let _ = std::process::Command::new("git")
        .args(["commit", "-m", &commit_msg])
        .current_dir(working_dir)
        .output();
}

/// Get filtered topic graph names from the store.
fn filtered_topics(config: &MaintenanceConfig, store: &LiveKnowledgeStore) -> Vec<String> {
    let graphs = match store.list_graphs() {
        Ok(g) => g,
        Err(_) => return Vec::new(),
    };
    graphs.into_iter()
        .filter(|g| g.name != "meta")
        .filter(|g| {
            config.rumination.topics.is_empty()
                || config.rumination.topics.iter().any(|t| t == &g.name)
        })
        .map(|g| g.name)
        .collect()
}

fn chrono_now() -> String {
    crate::dateutil::datetime_now()
}
