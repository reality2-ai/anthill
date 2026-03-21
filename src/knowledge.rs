//! Knowledge graph + episodic memory for ANTs.
//!
//! Two memory systems:
//! 1. Knowledge graph — entities and conjectural relationships (Popperian)
//! 2. Episodic memory — timestamped conversation summaries
//!
//! Both persisted to JSON. Context-aware retrieval extracts relevant
//! subgraphs and episodes for the AI prompt.

use petgraph::stable_graph::{NodeIndex, StableGraph};
use petgraph::visit::EdgeRef;
use petgraph::Direction;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::epistemic::{
    self, bayesian_update, to_log_odds, to_probability,
    DecayCategory, Evidence, EvidenceType, JustificationStep,
};

/// Node types the AI can create.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Person,
    Project,
    Server,
    Tool,
    #[default]
    Concept,
    Decision,
    Event,
    Fact,
    // Extended kinds used in topic-specific graphs (thurisaz, reality2, etc.)
    Theory,
    Mechanism,
    Principle,
    Constraint,
    Epistemology,
    Problem,
    #[serde(rename = "claim_type")]
    ClaimType,
    Claim,
    #[serde(rename = "open_question")]
    OpenQuestion,
    Implementation,
    Entity,
    Spec,
    Repo,
    Platform,
    Framework,
    #[serde(rename = "r2_spec")]
    R2Spec,
    /// Catch-all for unknown node kinds from external graphs.
    #[serde(other)]
    Other,
}

impl std::fmt::Display for NodeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Person => write!(f, "person"),
            Self::Project => write!(f, "project"),
            Self::Server => write!(f, "server"),
            Self::Tool => write!(f, "tool"),
            Self::Concept => write!(f, "concept"),
            Self::Decision => write!(f, "decision"),
            Self::Event => write!(f, "event"),
            Self::Fact => write!(f, "fact"),
            Self::Theory => write!(f, "theory"),
            Self::Mechanism => write!(f, "mechanism"),
            Self::Principle => write!(f, "principle"),
            Self::Constraint => write!(f, "constraint"),
            Self::Epistemology => write!(f, "epistemology"),
            Self::Problem => write!(f, "problem"),
            Self::ClaimType => write!(f, "claim_type"),
            Self::Claim => write!(f, "claim"),
            Self::OpenQuestion => write!(f, "open_question"),
            Self::Implementation => write!(f, "implementation"),
            Self::Entity => write!(f, "entity"),
            Self::Spec => write!(f, "spec"),
            Self::Repo => write!(f, "repo"),
            Self::Platform => write!(f, "platform"),
            Self::Framework => write!(f, "framework"),
            Self::R2Spec => write!(f, "r2_spec"),
            Self::Other => write!(f, "other"),
        }
    }
}

/// A node in the knowledge graph.
/// Tolerates extra fields from topic-specific graphs (e.g. "id", "spec", "source", "deps", "layer").
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KnowledgeNode {
    pub label: String,
    pub kind: NodeKind,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub created: String,
    #[serde(default)]
    pub updated: String,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Extra fields from topic graphs (id, spec, source, deps, layer, status, etc.)
    /// Not used in code — only consumed by serde to avoid parse failures.
    #[serde(flatten, default, skip_serializing)]
    pub(crate) _extra: serde_json::Map<String, serde_json::Value>,
}

/// How a conjecture was originally formed.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Basis {
    /// Directly observed by the AI.
    Observed,
    /// Told by the user.
    Told,
    /// Inferred from other knowledge.
    Inferred,
    /// Assumed without evidence.
    #[default]
    Assumed,
    /// Catch-all for unknown basis values the AI might write.
    #[serde(other)]
    Other,
}

#[allow(dead_code)]
impl Basis {
    /// Initial confidence for a new conjecture based on how it was formed.
    pub fn initial_confidence(&self) -> f64 {
        match self {
            Self::Observed => 0.7,
            Self::Told => 0.6,
            Self::Inferred => 0.4,
            Self::Assumed | Self::Other => 0.3,
        }
    }
}

/// Edge view classification (MAGMA-inspired orthogonal perspectives).
/// The same pair of nodes can have edges in different views.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum EdgeView {
    /// What things mean and how they relate conceptually.
    Semantic,
    /// When things happened, temporal ordering, validity periods.
    Temporal,
    /// Why things happened, cause-and-effect chains.
    Causal,
    /// Which entities are involved, structural connections.
    #[default]
    Entity,
    /// Catch-all for unknown view types the AI might write.
    #[serde(other)]
    Other,
}

/// An edge in the knowledge graph — a conjecture with confidence.
///
/// Follows Popperian epistemology: knowledge is conjectural and strengthened
/// through surviving refutation, not through confirmation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeEdge {
    pub relation: String,
    #[serde(default)]
    pub context: String,
    #[serde(default)]
    pub since: String,

    // --- Popperian fields ---

    /// Current confidence (0.0–1.0). Determines influence in the prompt.
    #[serde(default = "default_confidence")]
    pub confidence: f64,
    /// How many times this conjecture has been tested (encountered relevant context).
    #[serde(default)]
    pub tests: u32,
    /// How many tests it survived without contradiction.
    #[serde(default)]
    pub survived: u32,
    /// How the conjecture was originally formed.
    #[serde(default)]
    pub basis: Basis,
    /// When this conjecture was last tested or reinforced.
    #[serde(default)]
    pub last_tested: String,

    // --- Importance ---

    /// How important this relationship is (0.0–1.0).
    /// High importance = shown prominently even at lower confidence.
    /// Set by the AI based on how central this is to the project/user.
    #[serde(default = "default_importance")]
    pub importance: f64,
    /// How many times this edge has been referenced in conversations.
    #[serde(default)]
    pub references: u32,

    // --- Temporal validity (inspired by Zep/Graphiti) ---

    /// When this relationship became valid. Empty = since creation.
    #[serde(default)]
    pub valid_from: String,
    /// When this relationship stopped being valid. Empty = still valid.
    /// A non-empty valid_until means this is a historical fact, not a current one.
    #[serde(default)]
    pub valid_until: String,

    // --- Edge view classification (inspired by MAGMA) ---

    /// Which perspective this edge represents.
    /// Enables orthogonal retrieval: query by view type, not just by keyword.
    #[serde(default)]
    pub view: EdgeView,

    // --- Provenance ---

    /// Where this conjecture came from: document name, conversation date, or "observation".
    /// Enables "why do I believe this?" tracing.
    #[serde(default)]
    pub source: String,

    // --- Refutation log ---

    /// Audit trail of the conjecture-and-refutation process.
    /// Each entry records what was tested, what evidence was considered,
    /// and whether the conjecture survived. This IS the Popperian process.
    #[serde(default)]
    pub refutation_log: Vec<RefutationEntry>,

    // --- Thurisaz fields (Bayesian epistemic engine) ---

    /// Internal belief state in log-odds space (source of truth for confidence).
    /// confidence is computed as sigmoid(log_odds).
    /// Default: computed from confidence field on deserialization.
    #[serde(default)]
    pub log_odds: f64,

    /// Typed evidence trail with Bayes factors.
    #[serde(default)]
    pub evidence_log: Vec<Evidence>,

    /// Provenance chain: why do I believe this?
    #[serde(default)]
    pub justificatory_chain: Vec<JustificationStep>,

    /// Source identifier — links to the reputation registry.
    #[serde(default)]
    pub source_id: String,

    /// Decay category — controls how quickly this belief fades.
    #[serde(default)]
    pub decay_category: DecayCategory,

    // --- Darwinian competition fields ---

    /// Beneficial impact score (-1.0 to 1.0). Positive = beneficial for people/planet.
    /// Acts as a fitness modifier: beneficial ideas get an evolutionary advantage.
    /// 0.0 = neutral (default), 1.0 = strongly beneficial, -1.0 = harmful.
    #[serde(default)]
    pub beneficial_impact: f64,

    /// Corroboration strength: how strongly this edge is supported by its neighbours.
    /// Computed from the confidence of edges that connect to the same nodes.
    /// Higher = better-connected in the knowledge network. Recomputed during consolidation.
    #[serde(default)]
    pub corroboration_strength: f64,

    /// Competition group: edges with the same group ID are competing hypotheses.
    /// Empty = no competitors. Set by the AI or by the competition detection algorithm.
    #[serde(default)]
    pub competition_group: String,
}

/// A single entry in the refutation audit trail.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefutationEntry {
    /// When this test occurred.
    pub date: String,
    /// What was tested: "Does this relationship still hold given X?"
    pub test: String,
    /// What evidence was considered.
    pub evidence: String,
    /// Outcome: "survived", "weakened", "contradicted".
    pub outcome: String,
    /// Confidence before and after this test.
    #[serde(default)]
    pub confidence_before: f64,
    #[serde(default)]
    pub confidence_after: f64,
}

fn default_confidence() -> f64 { 0.5 }
fn default_importance() -> f64 { 0.5 }

#[allow(dead_code)]
impl KnowledgeEdge {
    /// Create a new conjecture.
    pub fn new(relation: &str, context: &str, since: &str, basis: Basis) -> Self {
        let confidence = basis.initial_confidence();
        let log_odds = to_log_odds(confidence);
        let basis_name = format!("{:?}", basis).to_lowercase();
        let decay_category = DecayCategory::from_basis(&basis_name);
        Self {
            relation: relation.into(),
            context: context.into(),
            since: since.into(),
            confidence,
            tests: 0,
            survived: 0,
            basis,
            last_tested: String::new(),
            importance: 0.5,
            references: 0,
            valid_from: since.into(),
            valid_until: String::new(),
            view: EdgeView::Entity,
            source: String::new(),
            refutation_log: vec![RefutationEntry {
                date: since.into(),
                test: "Initial conjecture".into(),
                evidence: context.into(),
                outcome: format!("conjectured (basis: {})", basis_name),
                confidence_before: 0.0,
                confidence_after: confidence,
            }],
            log_odds,
            evidence_log: Vec::new(),
            justificatory_chain: vec![JustificationStep {
                step: 1,
                process: format!("Initial conjecture (basis: {})", basis_name),
                confidence,
                source: String::new(),
            }],
            source_id: String::new(),
            decay_category,
            beneficial_impact: 0.0,
            corroboration_strength: 0.0,
            competition_group: String::new(),
        }
    }

    /// Record that this edge was referenced in a conversation.
    /// Importance grows logarithmically with reference count.
    pub fn reference(&mut self) {
        self.references += 1;
        // Importance grows with references: 0.5 at 0 refs, ~0.8 at 10 refs, ~0.9 at 50 refs.
        self.importance = 0.5 + 0.5 * (1.0 - 1.0 / (1.0 + self.references as f64 / 10.0));
        self.importance = self.importance.clamp(0.0, 1.0);
    }

    /// Combined score: confidence × importance × fitness. Used for prompt prioritisation.
    /// Beneficial ideas get a fitness advantage; harmful ideas are penalised.
    /// Corroboration strength provides a network effect bonus.
    pub fn relevance_score(&self) -> f64 {
        let fitness = 1.0 + 0.2 * self.beneficial_impact; // range 0.8–1.2
        let network_bonus = 1.0 + 0.1 * self.corroboration_strength; // mild boost
        self.confidence * self.importance * fitness * network_bonus
    }

    /// Ensure log_odds and confidence are in sync.
    /// Call after deserialization or manual confidence changes.
    pub fn ensure_log_odds(&mut self) {
        // Check if log_odds and confidence are out of sync
        let expected_conf = to_probability(self.log_odds);
        if (expected_conf - self.confidence).abs() > 0.01 {
            // Confidence was likely set directly — recompute log_odds from it
            self.log_odds = to_log_odds(self.confidence);
        }
    }

    /// Sync confidence from log_odds (call after any log_odds change).
    fn sync_confidence(&mut self) {
        self.confidence = to_probability(self.log_odds);
    }

    // ── Primary update path: typed evidence ────────────────────────

    /// Update this edge with typed evidence from a source with known reputation.
    /// This is the Thurisaz-compliant primary update path.
    pub fn update_with_evidence(&mut self, evidence_type: EvidenceType, date: &str,
                                 test: &str, detail: &str, source_id: &str,
                                 source_reputation: f64) {
        self.ensure_log_odds();
        let before_lo = self.log_odds;
        let before_conf = self.confidence;

        // Compute effective Bayes factor
        let bf = evidence_type.effective_bayes_factor(source_reputation);

        // Apply Bayesian update
        self.log_odds = bayesian_update(self.log_odds, bf);
        self.sync_confidence();

        // Update test counters for backward compatibility
        self.tests += 1;
        if bf > 1.0 { self.survived += 1; }
        self.last_tested = date.into();

        // Record in evidence log (Thurisaz)
        self.evidence_log.push(Evidence {
            date: date.into(),
            evidence_type: evidence_type.clone(),
            test: test.into(),
            detail: detail.into(),
            source_id: source_id.into(),
            source_reputation,
            bayes_factor: bf,
            log_odds_before: before_lo,
            log_odds_after: self.log_odds,
        });

        // Record in refutation log (backward compatibility)
        let outcome = match &evidence_type {
            EvidenceType::RefutationSurvived | EvidenceType::Corroboration |
            EvidenceType::HumanAttestation | EvidenceType::Consistency |
            EvidenceType::Synthesis | EvidenceType::CompetitionWon |
            EvidenceType::PatternTransfer => "survived",
            EvidenceType::RefutationFailed => "contradicted",
            EvidenceType::Contradiction | EvidenceType::Inconsistency |
            EvidenceType::CompetitionLost => "weakened",
            EvidenceType::InconsequentialSearch | EvidenceType::Unknown => "inconsequential",
        };
        self.refutation_log.push(RefutationEntry {
            date: date.into(),
            test: if test.is_empty() { evidence_type.description().into() } else { test.into() },
            evidence: detail.into(),
            outcome: outcome.into(),
            confidence_before: before_conf,
            confidence_after: self.confidence,
        });

        // Update justificatory chain
        let step = self.justificatory_chain.len() as u32 + 1;
        self.justificatory_chain.push(JustificationStep {
            step,
            process: format!("{} (BF={:.2}, rep={:.2})", evidence_type.description(), bf, source_reputation),
            confidence: self.confidence,
            source: source_id.into(),
        });
    }

    // ── Convenience wrappers (backward-compatible API) ─────────────

    /// The conjecture survived a refutation attempt — strengthen it.
    pub fn strengthen(&mut self, date: &str) {
        self.strengthen_with(date, "", "");
    }

    /// Strengthen with a record of the test and evidence.
    pub fn strengthen_with(&mut self, date: &str, test: &str, evidence: &str) {
        self.update_with_evidence(
            EvidenceType::RefutationSurvived,
            date,
            if test.is_empty() { "Encountered in context — no contradiction found" } else { test },
            evidence,
            &self.source_id.clone(),
            0.5, // default neutral reputation for legacy calls
        );
    }

    /// The conjecture was tested and evidence weakened it (but didn't refute it).
    pub fn weaken(&mut self, date: &str) {
        self.weaken_with(date, "", "");
    }

    /// Weaken with a record of the test and evidence.
    pub fn weaken_with(&mut self, date: &str, test: &str, evidence: &str) {
        self.update_with_evidence(
            EvidenceType::Inconsistency,
            date,
            if test.is_empty() { "Encountered counter-evidence" } else { test },
            evidence,
            &self.source_id.clone(),
            0.5,
        );
    }

    /// Direct contradiction — sharp confidence penalty.
    pub fn contradict(&mut self, date: &str) {
        self.contradict_with(date, "", "");
    }

    /// Contradict with a record of the evidence.
    pub fn contradict_with(&mut self, date: &str, test: &str, evidence: &str) {
        self.update_with_evidence(
            EvidenceType::RefutationFailed,
            date,
            if test.is_empty() { "Direct contradiction encountered" } else { test },
            evidence,
            &self.source_id.clone(),
            0.5,
        );
    }

    /// Apply time decay — beliefs fade toward uncertainty based on decay category.
    /// Call with the number of days since last tested.
    pub fn decay(&mut self, days_since_tested: u32) {
        if days_since_tested == 0 { return; }
        self.ensure_log_odds();
        let elapsed_secs = days_since_tested as f64 * 86400.0;
        let half_life = self.decay_category.half_life_secs();
        self.log_odds = epistemic::decay(self.log_odds, elapsed_secs, half_life);
        self.sync_confidence();
        if self.confidence < 0.01 { self.confidence = 0.01; self.log_odds = to_log_odds(0.01); }
    }

    /// Confidence tier for rendering.
    pub fn confidence_label(&self) -> &'static str {
        if self.confidence >= 0.8 { "established" }
        else if self.confidence >= 0.6 { "likely" }
        else if self.confidence >= 0.4 { "possible" }
        else if self.confidence >= 0.2 { "uncertain" }
        else { "doubtful" }
    }
}

/// Minimum confidence for an edge to appear in the prompt.
pub const MIN_PROMPT_CONFIDENCE: f64 = 0.15;

// --- Query result types ---

/// An edge in a query result, with path confidence.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct WeightedEdge {
    pub from: NodeIndex,
    pub to: NodeIndex,
    pub edge: KnowledgeEdge,
    /// Cumulative confidence along the path that led to this edge.
    pub path_confidence: f64,
}

/// A path between two nodes with cumulative confidence.
#[derive(Debug, Clone)]
pub struct ConfidencePath {
    pub nodes: Vec<NodeIndex>,
    /// Product of edge confidences along the path (weakest link chain).
    pub cumulative_confidence: f64,
}

/// Result of a graph query.
#[derive(Debug, Default)]
pub struct QueryResult {
    /// Nodes found, with relevance scores.
    pub nodes: Vec<(NodeIndex, KnowledgeNode, f64)>,
    /// Edges in the result subgraph.
    pub edges: Vec<WeightedEdge>,
    /// Paths found (for path queries).
    pub paths: Vec<ConfidencePath>,
}

/// A pair of contradictory edges between the same nodes.
#[derive(Debug, Clone)]
pub struct ContradictionPair {
    pub node_a_label: String,
    pub node_b_label: String,
    pub edge_a_relation: String,
    pub edge_a_confidence: f64,
    pub edge_a_context: String,
    pub edge_b_relation: String,
    pub edge_b_confidence: f64,
    pub edge_b_context: String,
}

/// Statistics about uncertainty in a graph.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct UncertaintyStats {
    pub edge_count: usize,
    pub uncertain_edge_count: usize,
    pub avg_confidence: f64,
}

/// A group of competing hypotheses between the same pair of nodes.
#[derive(Debug, Clone)]
pub struct CompetitorGroup {
    pub node_a_label: String,
    pub node_b_label: String,
    pub competitors: Vec<Competitor>,
}

/// A single competitor in a group.
#[derive(Debug, Clone)]
pub struct Competitor {
    pub relation: String,
    pub confidence: f64,
    pub context: String,
}

/// A cross-domain pattern match between two topic graphs.
#[derive(Debug, Clone)]
pub struct PatternMatch {
    pub source_from: String,
    pub source_to: String,
    pub source_relation: String,
    pub source_confidence: f64,
    pub target_from: String,
    pub target_to: String,
    pub target_relation: String,
    pub target_confidence: f64,
    pub similarity_reason: String,
}

/// Confidence below which edges are archived (moved to separate file).
pub const ARCHIVE_CONFIDENCE: f64 = 0.10;

/// Maximum active nodes before auto-archiving triggers.
#[allow(dead_code)]
pub const MAX_ACTIVE_NODES: usize = 500;

/// Serializable graph format (petgraph's serde format).
/// Tolerates extra top-level fields (e.g. "meta") from topic-specific graphs.
#[derive(Serialize, Deserialize)]
pub struct GraphData {
    pub nodes: Vec<Option<KnowledgeNode>>,
    pub edges: Vec<(usize, usize, KnowledgeEdge)>,
    /// Catch-all for extra fields (e.g. "meta" in topic graphs).
    #[serde(flatten)]
    _extra: serde_json::Map<String, serde_json::Value>,
}

/// Knowledge graph with keyword index for retrieval.
pub struct KnowledgeGraph {
    pub(crate) graph: StableGraph<KnowledgeNode, KnowledgeEdge>,
    keyword_index: HashMap<String, HashSet<NodeIndex>>,
    #[allow(dead_code)]
    file_path: PathBuf,
}

impl KnowledgeGraph {
    /// Load from JSON file, or create empty.
    /// On parse failure, tries the archive as fallback and preserves the
    /// corrupted file for manual recovery.
    pub fn load(path: &Path) -> Self {
        let mut kg = Self {
            graph: StableGraph::new(),
            keyword_index: HashMap::new(),
            file_path: path.to_path_buf(),
        };

        if path.exists() {
            if let Ok(contents) = std::fs::read_to_string(path) {
                match serde_json::from_str::<GraphData>(&contents) {
                    Ok(data) => {
                        kg.load_graph_data(&data);
                    }
                    Err(e) => {
                        log::warn!(
                            "Strict parse failed for {}: {} — trying lenient parse",
                            path.display(), e
                        );

                        // Recovery strategy 0: lenient parse — extract what we can.
                        // Parse as generic JSON and deserialize nodes/edges individually,
                        // keeping what works and skipping what doesn't.
                        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&contents) {
                            let (recovered, skipped) = kg.load_lenient(&value);
                            if recovered > 0 {
                                log::warn!(
                                    "Lenient parse recovered {} nodes/edges, skipped {} from {}",
                                    recovered, skipped, path.display()
                                );
                                // Save the cleaned version so future loads succeed strictly.
                                kg.save();
                            }
                        }

                        // If lenient parse also got nothing, try harder recovery.
                        if kg.graph.node_count() == 0 {
                            log::error!(
                                "Lenient parse also failed for {} — trying git/archive recovery",
                                path.display()
                            );

                            // Preserve corrupted file for investigation.
                            let corrupted = path.with_extension("json.corrupted");
                            let _ = std::fs::copy(path, &corrupted);
                        }

                        // Recovery strategy 1: git checkout (most reliable).
                        // The working directory is a git repo — restore the last committed version.
                        let git_recovered = std::process::Command::new("git")
                            .args(["checkout", "HEAD", "--"])
                            .arg(path.as_os_str())
                            .current_dir(path.parent().and_then(|p| p.parent()).unwrap_or(Path::new(".")))
                            .output()
                            .ok()
                            .map(|o| o.status.success())
                            .unwrap_or(false);

                        if git_recovered {
                            // Re-read the restored file.
                            if let Ok(restored) = std::fs::read_to_string(path) {
                                if let Ok(data) = serde_json::from_str::<GraphData>(&restored) {
                                    kg.load_graph_data(&data);
                                    log::warn!(
                                        "Recovered {} nodes via git checkout: {}",
                                        kg.graph.node_count(), path.display()
                                    );
                                }
                            }
                        }

                        // Recovery strategy 2: archive fallback.
                        if kg.graph.node_count() == 0 {
                            let archive = path.with_file_name("knowledge-archive.json");
                            if archive.exists() {
                                if let Ok(arc_contents) = std::fs::read_to_string(&archive) {
                                    if let Ok(data) = serde_json::from_str::<GraphData>(&arc_contents) {
                                        kg.load_graph_data(&data);
                                        log::warn!(
                                            "Recovered {} nodes from archive {}",
                                            kg.graph.node_count(), archive.display()
                                        );
                                    }
                                }
                            }
                        }

                        if kg.graph.node_count() == 0 {
                            log::error!("No recovery possible — starting with empty graph");
                        }
                    }
                }
            }
        }

        // Also load topic-specific graphs from a sibling graphs/ directory.
        // Each graph is merged into the main graph with a prefix to avoid ID collisions.
        if let Some(parent) = path.parent() {
            let graphs_dir = parent.join("graphs");
            if graphs_dir.is_dir() {
                if let Ok(entries) = std::fs::read_dir(&graphs_dir) {
                    for entry in entries.flatten() {
                        let p = entry.path();
                        if p.extension().and_then(|e| e.to_str()) == Some("json") {
                            if let Ok(contents) = std::fs::read_to_string(&p) {
                                if let Ok(data) = serde_json::from_str::<GraphData>(&contents) {
                                    let topic = p.file_stem()
                                        .and_then(|s| s.to_str())
                                        .unwrap_or("unknown");
                                    // Build index map for this topic graph.
                                    let mut index_map: Vec<Option<NodeIndex>> = Vec::new();
                                    for node_opt in &data.nodes {
                                        if let Some(node) = node_opt {
                                            // Check for duplicate labels — skip if already in graph.
                                            let existing = kg.graph.node_indices()
                                                .find(|&idx| kg.graph[idx].label == node.label);
                                            if let Some(idx) = existing {
                                                index_map.push(Some(idx));
                                            } else {
                                                let idx = kg.graph.add_node(node.clone());
                                                index_map.push(Some(idx));
                                            }
                                        } else {
                                            index_map.push(None);
                                        }
                                    }
                                    let mut edge_count = 0;
                                    for (from, to, edge) in &data.edges {
                                        if let (Some(Some(from_idx)), Some(Some(to_idx))) =
                                            (index_map.get(*from), index_map.get(*to))
                                        {
                                            kg.graph.add_edge(*from_idx, *to_idx, edge.clone());
                                            edge_count += 1;
                                        }
                                    }
                                    log::debug!(
                                        "Loaded topic graph '{}': {} nodes, {} edges",
                                        topic, data.nodes.len(), edge_count
                                    );
                                } else {
                                    log::warn!(
                                        "Failed to parse topic graph at {}, skipping",
                                        p.display()
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }

        kg.rebuild_index();
        kg
    }

    /// Load nodes and edges from serialized graph data.
    fn load_graph_data(&mut self, data: &GraphData) {
        let mut index_map: Vec<Option<NodeIndex>> = Vec::new();
        for node_opt in &data.nodes {
            if let Some(node) = node_opt {
                let idx = self.graph.add_node(node.clone());
                index_map.push(Some(idx));
            } else {
                index_map.push(None);
            }
        }
        for (from, to, edge) in &data.edges {
            if let (Some(Some(from_idx)), Some(Some(to_idx))) =
                (index_map.get(*from), index_map.get(*to))
            {
                let mut edge = edge.clone();
                edge.ensure_log_odds();
                self.graph.add_edge(*from_idx, *to_idx, edge);
            }
        }
    }

    /// Lenient parse: extract nodes and edges individually from generic JSON.
    /// Tolerates per-item parse failures — keeps what works, skips what doesn't.
    /// Returns (recovered_count, skipped_count).
    fn load_lenient(&mut self, value: &serde_json::Value) -> (usize, usize) {
        let mut recovered = 0usize;
        let mut skipped = 0usize;

        // Parse nodes.
        let mut index_map: Vec<Option<NodeIndex>> = Vec::new();
        if let Some(nodes) = value.get("nodes").and_then(|v| v.as_array()) {
            for node_val in nodes {
                if node_val.is_null() {
                    index_map.push(None);
                    continue;
                }
                match serde_json::from_value::<KnowledgeNode>(node_val.clone()) {
                    Ok(node) => {
                        let idx = self.graph.add_node(node);
                        index_map.push(Some(idx));
                        recovered += 1;
                    }
                    Err(e) => {
                        // Try minimal extraction — at least get the label.
                        let label = node_val.get("label")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown");
                        let node = KnowledgeNode {
                            label: label.into(),
                            kind: NodeKind::Other,
                            summary: format!("(recovered from parse error: {})", e),
                            ..Default::default()
                        };
                        let idx = self.graph.add_node(node);
                        index_map.push(Some(idx));
                        recovered += 1;
                        log::warn!("Lenient: recovered node '{}' with parse error: {}", label, e);
                    }
                }
            }
        }

        // Parse edges.
        if let Some(edges) = value.get("edges").and_then(|v| v.as_array()) {
            for edge_val in edges {
                let arr = match edge_val.as_array() {
                    Some(a) if a.len() >= 3 => a,
                    _ => { skipped += 1; continue; }
                };
                let from = match arr[0].as_u64() {
                    Some(v) => v as usize,
                    None => { skipped += 1; continue; }
                };
                let to = match arr[1].as_u64() {
                    Some(v) => v as usize,
                    None => { skipped += 1; continue; }
                };

                let (from_idx, to_idx) = match (index_map.get(from), index_map.get(to)) {
                    (Some(Some(f)), Some(Some(t))) => (*f, *t),
                    _ => { skipped += 1; continue; }
                };

                match serde_json::from_value::<KnowledgeEdge>(arr[2].clone()) {
                    Ok(mut edge) => {
                        edge.ensure_log_odds();
                        self.graph.add_edge(from_idx, to_idx, edge);
                        recovered += 1;
                    }
                    Err(e) => {
                        // Try minimal extraction — at least get the relation.
                        let relation = arr[2].get("relation")
                            .and_then(|v| v.as_str())
                            .unwrap_or("?");
                        let confidence = arr[2].get("confidence")
                            .and_then(|v| v.as_f64())
                            .unwrap_or(0.3);
                        let edge = KnowledgeEdge::new(
                            relation,
                            &format!("(recovered from parse error: {})", e),
                            "",
                            Basis::Assumed,
                        );
                        let mut edge = KnowledgeEdge { confidence, ..edge };
                        edge.ensure_log_odds();
                        self.graph.add_edge(from_idx, to_idx, edge);
                        recovered += 1;
                        log::warn!("Lenient: recovered edge '{}' with parse error: {}", relation, e);
                    }
                }
            }
        }

        (recovered, skipped)
    }

    /// Migrate existing edges to Thurisaz format.
    /// Computes log_odds from confidence, converts refutation_log to evidence_log,
    /// assigns decay categories. Safe to call multiple times (idempotent).
    #[allow(dead_code)]
    pub fn backfill_to_thurisaz(&mut self) -> usize {
        let edge_indices: Vec<_> = self.graph.edge_indices().collect();
        let mut migrated = 0;

        for eid in edge_indices {
            let edge = &mut self.graph[eid];
            let mut changed = false;

            // Ensure log_odds is set
            if edge.log_odds == 0.0 && (edge.confidence - 0.5).abs() > 0.001 {
                edge.log_odds = to_log_odds(edge.confidence);
                changed = true;
            }

            // Assign decay category if default and we can infer better
            if edge.decay_category == DecayCategory::Fact {
                let basis_name = format!("{:?}", edge.basis).to_lowercase();
                let inferred = DecayCategory::from_basis(&basis_name);
                if inferred != DecayCategory::Fact || basis_name != "told" {
                    edge.decay_category = inferred;
                    changed = true;
                }
            }

            // Convert refutation_log entries to evidence_log if empty
            if edge.evidence_log.is_empty() && !edge.refutation_log.is_empty() {
                for entry in &edge.refutation_log {
                    let evidence_type = match entry.outcome.as_str() {
                        "survived" => EvidenceType::RefutationSurvived,
                        "weakened" => EvidenceType::Inconsistency,
                        "contradicted" => EvidenceType::RefutationFailed,
                        _ => EvidenceType::Consistency, // "conjectured" entries
                    };
                    let bf = evidence_type.effective_bayes_factor(0.5);
                    edge.evidence_log.push(crate::epistemic::Evidence {
                        date: entry.date.clone(),
                        evidence_type,
                        test: entry.test.clone(),
                        detail: entry.evidence.clone(),
                        source_id: edge.source.clone(),
                        source_reputation: 0.5,
                        bayes_factor: bf,
                        log_odds_before: to_log_odds(entry.confidence_before.max(0.001)),
                        log_odds_after: to_log_odds(entry.confidence_after.max(0.001)),
                    });
                }
                changed = true;
            }

            // Build initial justificatory chain if empty
            if edge.justificatory_chain.is_empty() {
                let basis_name = format!("{:?}", edge.basis).to_lowercase();
                edge.justificatory_chain.push(JustificationStep {
                    step: 1,
                    process: format!("Initial conjecture (basis: {}, migrated to Thurisaz)", basis_name),
                    confidence: edge.basis.initial_confidence(),
                    source: edge.source.clone(),
                });
                if edge.tests > 0 {
                    edge.justificatory_chain.push(JustificationStep {
                        step: 2,
                        process: format!("Historical: {} tests, {} survived (migrated)", edge.tests, edge.survived),
                        confidence: edge.confidence,
                        source: edge.source.clone(),
                    });
                }
                changed = true;
            }

            // Set source_id from source if empty
            if edge.source_id.is_empty() && !edge.source.is_empty() {
                edge.source_id = edge.source.clone();
                changed = true;
            }

            if changed { migrated += 1; }
        }

        if migrated > 0 {
            log::info!("Migrated {} edges to Thurisaz format", migrated);
        }
        migrated
    }

    /// Save to JSON file (atomic write).
    #[allow(dead_code)]
    pub fn save(&self) {
        let data = self.to_serializable();
        if let Ok(json) = serde_json::to_string_pretty(&data) {
            let tmp = self.file_path.with_extension("json.tmp");
            // Write + fsync to ensure data hits disk before rename.
            // Prevents corruption from process kill between write and flush.
            let write_ok = (|| -> std::io::Result<()> {
                use std::io::Write;
                let mut f = std::fs::File::create(&tmp)?;
                f.write_all(json.as_bytes())?;
                f.sync_all()?; // fsync — data is on disk
                Ok(())
            })();
            if write_ok.is_ok() {
                let _ = std::fs::rename(&tmp, &self.file_path);
            }
        }
    }

    /// Convert the in-memory graph to the serializable format.
    pub fn to_graph_data(&self) -> GraphData {
        self.to_serializable()
    }

    fn to_serializable(&self) -> GraphData {
        let max_idx = self.graph.node_indices().map(|i| i.index()).max().unwrap_or(0);
        let mut nodes: Vec<Option<KnowledgeNode>> = vec![None; max_idx + 1];
        for idx in self.graph.node_indices() {
            nodes[idx.index()] = Some(self.graph[idx].clone());
        }
        let edges: Vec<(usize, usize, KnowledgeEdge)> = self
            .graph
            .edge_indices()
            .filter_map(|e| {
                let (a, b) = self.graph.edge_endpoints(e)?;
                Some((a.index(), b.index(), self.graph[e].clone()))
            })
            .collect();
        GraphData { nodes, edges, _extra: serde_json::Map::new() }
    }

    /// Rebuild the keyword inverted index from all nodes.
    pub fn rebuild_index(&mut self) {
        self.keyword_index.clear();
        for idx in self.graph.node_indices() {
            let node = &self.graph[idx];
            for keyword in Self::node_keywords(node) {
                self.keyword_index
                    .entry(keyword)
                    .or_default()
                    .insert(idx);
            }
        }
    }

    /// Extract keywords from a node for indexing.
    fn node_keywords(node: &KnowledgeNode) -> Vec<String> {
        let mut keywords = Vec::new();
        for word in tokenize(&node.label) {
            keywords.push(word);
        }
        for word in tokenize(&node.summary) {
            keywords.push(word);
        }
        for tag in &node.tags {
            for word in tokenize(tag) {
                keywords.push(word);
            }
        }
        keywords.push(node.kind.to_string());
        keywords
    }

    /// Find nodes matching a label (case-insensitive).
    #[allow(dead_code)]
    pub fn find_by_label(&self, label: &str) -> Option<NodeIndex> {
        let lower = label.to_lowercase();
        self.graph
            .node_indices()
            .find(|&idx| self.graph[idx].label.to_lowercase() == lower)
    }

    /// Total node count.
    pub fn node_count(&self) -> usize {
        self.graph.node_count()
    }

    /// All node labels (for cross-referencing between graphs).
    pub fn all_node_labels(&self) -> Vec<String> {
        self.graph.node_indices()
            .map(|idx| self.graph[idx].label.clone())
            .collect()
    }

    /// Add a topic node (for the meta-graph).
    pub fn add_topic_node(&mut self, topic: &str) -> NodeIndex {
        self.graph.add_node(KnowledgeNode {
            label: topic.into(),
            kind: NodeKind::Concept,
            summary: format!("Topic graph: memory/graphs/{}.json", topic),
            tags: vec!["graph".into(), "topic".into()],
            ..Default::default()
        })
    }

    /// Check if an edge with a given relation exists between two nodes.
    pub fn has_edge_between(&self, from: NodeIndex, to: NodeIndex, relation: &str) -> bool {
        self.graph.edges_directed(from, Direction::Outgoing)
            .any(|e| e.target() == to && e.weight().relation == relation)
    }

    /// Add a cross-reference edge between two topic nodes in the meta-graph.
    pub fn add_cross_link(&mut self, from: NodeIndex, to: NodeIndex, context: &str) {
        let edge = KnowledgeEdge {
            relation: "shares_entities".into(),
            context: context.into(),
            since: String::new(),
            confidence: 0.6,
            tests: 0,
            survived: 0,
            basis: Basis::Observed,
            last_tested: String::new(),
            importance: 0.5,
            references: 0,
            valid_from: String::new(),
            valid_until: String::new(),
            view: EdgeView::Semantic,
            source: "maintenance: cross-link".into(),
            refutation_log: Vec::new(),
            log_odds: to_log_odds(0.6),
            evidence_log: Vec::new(),
            justificatory_chain: Vec::new(),
            source_id: "maintenance:cross-link".into(),
            decay_category: DecayCategory::Fact,
            beneficial_impact: 0.0,
            corroboration_strength: 0.0,
            competition_group: String::new(),
        };
        self.graph.add_edge(from, to, edge);
    }

    /// Add an edge to the graph (used by synthesis and other direct-write operations).
    pub fn add_edge(&mut self, from: NodeIndex, to: NodeIndex, edge: KnowledgeEdge) {
        self.graph.add_edge(from, to, edge);
    }

    // ── Darwinian competition methods ────────────────────────────────

    /// Find competing hypotheses: edges between the same pair of nodes
    /// that offer alternative explanations (different relations).
    pub fn find_competitors(&self) -> Vec<CompetitorGroup> {
        type EdgeInfo = (petgraph::graph::EdgeIndex, String, f64, String);
        let mut pair_edges: HashMap<(usize, usize), Vec<EdgeInfo>> = HashMap::new();

        for edge_idx in self.graph.edge_indices() {
            let edge = &self.graph[edge_idx];
            if !Self::is_edge_current(edge) { continue; }
            if edge.confidence < MIN_PROMPT_CONFIDENCE { continue; }

            if let Some((src, tgt)) = self.graph.edge_endpoints(edge_idx) {
                let key = if src.index() <= tgt.index() {
                    (src.index(), tgt.index())
                } else {
                    (tgt.index(), src.index())
                };
                pair_edges.entry(key).or_default().push((
                    edge_idx,
                    edge.relation.clone(),
                    edge.confidence,
                    edge.context.clone(),
                ));
            }
        }

        let mut groups = Vec::new();
        for ((a, b), edges) in &pair_edges {
            let unique_relations: HashSet<&str> = edges.iter().map(|(_, r, _, _)| r.as_str()).collect();
            if unique_relations.len() < 2 { continue; }

            let node_a = self.graph.node_indices()
                .find(|&n| n.index() == *a)
                .map(|n| self.graph[n].label.clone())
                .unwrap_or_else(|| "?".into());
            let node_b = self.graph.node_indices()
                .find(|&n| n.index() == *b)
                .map(|n| self.graph[n].label.clone())
                .unwrap_or_else(|| "?".into());

            let competitors: Vec<Competitor> = edges.iter().map(|(_, rel, conf, ctx)| {
                Competitor {
                    relation: rel.clone(),
                    confidence: *conf,
                    context: ctx.clone(),
                }
            }).collect();

            groups.push(CompetitorGroup {
                node_a_label: node_a,
                node_b_label: node_b,
                competitors,
            });
        }

        groups
    }

    /// Compute corroboration strength for all edges.
    /// An edge's corroboration strength is the average confidence of edges
    /// that share a source or target node — the "neighbourhood strength".
    pub fn compute_corroboration_strength(&mut self) {
        let mut node_edge_confidences: HashMap<usize, Vec<f64>> = HashMap::new();
        for edge_idx in self.graph.edge_indices() {
            let edge = &self.graph[edge_idx];
            if let Some((src, tgt)) = self.graph.edge_endpoints(edge_idx) {
                node_edge_confidences.entry(src.index()).or_default().push(edge.confidence);
                node_edge_confidences.entry(tgt.index()).or_default().push(edge.confidence);
            }
        }

        let node_avg: HashMap<usize, f64> = node_edge_confidences.iter()
            .map(|(idx, confs)| {
                let avg = confs.iter().sum::<f64>() / confs.len() as f64;
                (*idx, avg)
            })
            .collect();

        // Collect updates first to avoid borrow conflict.
        let updates: Vec<(petgraph::graph::EdgeIndex, f64)> = self.graph.edge_indices()
            .filter_map(|edge_idx| {
                let (src, tgt) = self.graph.edge_endpoints(edge_idx)?;
                let src_avg = node_avg.get(&src.index()).copied().unwrap_or(0.5);
                let tgt_avg = node_avg.get(&tgt.index()).copied().unwrap_or(0.5);
                Some((edge_idx, (src_avg + tgt_avg) / 2.0))
            })
            .collect();

        for (edge_idx, strength) in updates {
            self.graph[edge_idx].corroboration_strength = strength;
        }
    }

    /// Find cross-domain patterns: edges in different topic areas with similar
    /// relations that might inform each other.
    pub fn find_cross_domain_patterns(&self, other: &KnowledgeGraph, limit: usize) -> Vec<PatternMatch> {
        let mut matches = Vec::new();

        for self_eid in self.graph.edge_indices() {
            let self_edge = &self.graph[self_eid];
            if self_edge.confidence < 0.5 { continue; }

            for other_eid in other.graph.edge_indices() {
                let other_edge = &other.graph[other_eid];
                if other_edge.confidence < 0.5 { continue; }

                let self_rel = self_edge.relation.to_lowercase();
                let other_rel = other_edge.relation.to_lowercase();
                if self_rel == other_rel || relations_similar(&self_rel, &other_rel) {
                    if let (Some((ss, st)), Some((os, ot))) = (
                        self.graph.edge_endpoints(self_eid),
                        other.graph.edge_endpoints(other_eid),
                    ) {
                        matches.push(PatternMatch {
                            source_from: self.graph[ss].label.clone(),
                            source_to: self.graph[st].label.clone(),
                            source_relation: self_edge.relation.clone(),
                            source_confidence: self_edge.confidence,
                            target_from: other.graph[os].label.clone(),
                            target_to: other.graph[ot].label.clone(),
                            target_relation: other_edge.relation.clone(),
                            target_confidence: other_edge.confidence,
                            similarity_reason: format!("Similar relation: '{}' ≈ '{}'",
                                self_edge.relation, other_edge.relation),
                        });
                    }
                }
            }
        }

        matches.truncate(limit);
        matches
    }

    // ── Rumination support methods ─────────────────────────────────

    /// Find undetermined connections — edges with relation "?" that need investigation.
    /// Returns (from_label, to_label) pairs.
    pub fn undetermined_connections(&self, limit: usize) -> Vec<(String, String)> {
        let mut results = Vec::new();
        for edge_idx in self.graph.edge_indices() {
            let edge = &self.graph[edge_idx];
            if edge.relation == "?" {
                if let Some((src, tgt)) = self.graph.edge_endpoints(edge_idx) {
                    results.push((
                        self.graph[src].label.clone(),
                        self.graph[tgt].label.clone(),
                    ));
                }
            }
            if results.len() >= limit { break; }
        }
        results
    }

    /// Find candidates for idea synthesis: A→B→C where both edges are strong
    /// and no direct A→C edge exists yet.
    /// Returns (A_index, C_index, B_label, relation_AB, relation_BC) tuples.
    pub fn synthesis_candidates(&self, limit: usize) -> Vec<(NodeIndex, NodeIndex, String, String, String)> {
        let mut candidates = Vec::new();

        for b_idx in self.graph.node_indices() {
            let b_label = self.graph[b_idx].label.clone();

            // Get strong incoming edges to B.
            let incoming: Vec<_> = self.graph.edges_directed(b_idx, Direction::Incoming)
                .filter(|e| e.weight().confidence >= 0.6 && Self::is_edge_current(e.weight()))
                .map(|e| (e.source(), e.weight().relation.clone()))
                .collect();

            // Get strong outgoing edges from B.
            let outgoing: Vec<_> = self.graph.edges_directed(b_idx, Direction::Outgoing)
                .filter(|e| e.weight().confidence >= 0.6 && Self::is_edge_current(e.weight()))
                .map(|e| (e.target(), e.weight().relation.clone()))
                .collect();

            for (a_idx, r1) in &incoming {
                for (c_idx, r2) in &outgoing {
                    // Skip self-loops and A==C.
                    if a_idx == c_idx || *a_idx == b_idx || *c_idx == b_idx { continue; }

                    // Check that no direct A→C edge exists.
                    let has_direct = self.graph.edges_directed(*a_idx, Direction::Outgoing)
                        .any(|e| e.target() == *c_idx);
                    if has_direct { continue; }

                    candidates.push((*a_idx, *c_idx, b_label.clone(), r1.clone(), r2.clone()));
                }
            }
        }

        // Sort by combined confidence (use labels for determinism).
        candidates.sort_by(|a, b| a.2.cmp(&b.2));
        candidates.truncate(limit);
        candidates
    }

    /// Find edges suitable for active refutation: important but uncertain beliefs.
    /// Returns (from_label, to_label, relation, confidence, importance) tuples.
    pub fn refutation_candidates(&self, limit: usize) -> Vec<(String, String, String, f64, f64)> {
        let mut candidates: Vec<(String, String, String, f64, f64)> = Vec::new();

        for edge_idx in self.graph.edge_indices() {
            let edge = &self.graph[edge_idx];

            // Select edges where confidence is moderate and importance is meaningful.
            if edge.confidence < 0.35 || edge.confidence > 0.80 { continue; }
            if edge.importance < 0.3 { continue; }
            if edge.basis == Basis::Assumed { continue; }
            if !Self::is_edge_current(edge) { continue; }

            if let Some((src, tgt)) = self.graph.edge_endpoints(edge_idx) {
                let from_label = self.graph[src].label.clone();
                let to_label = self.graph[tgt].label.clone();
                candidates.push((
                    from_label,
                    to_label,
                    edge.relation.clone(),
                    edge.confidence,
                    edge.importance,
                ));
            }
        }

        // Sort by priority: importance × (1 - confidence) descending.
        candidates.sort_by(|a, b| {
            let score_a = a.4 * (1.0 - a.3);
            let score_b = b.4 * (1.0 - b.3);
            score_b.partial_cmp(&score_a).unwrap_or(std::cmp::Ordering::Equal)
        });
        candidates.truncate(limit);
        candidates
    }

    /// Find pairs of contradictory edges (structured version of detect_contradictions).
    pub fn contradiction_pairs(&self) -> Vec<ContradictionPair> {
        let mut pair_edges: std::collections::HashMap<(usize, usize), Vec<(petgraph::graph::EdgeIndex, &KnowledgeEdge)>> =
            std::collections::HashMap::new();

        for edge_idx in self.graph.edge_indices() {
            if let Some((src, tgt)) = self.graph.edge_endpoints(edge_idx) {
                let key = if src.index() <= tgt.index() {
                    (src.index(), tgt.index())
                } else {
                    (tgt.index(), src.index())
                };
                pair_edges.entry(key).or_default().push((edge_idx, &self.graph[edge_idx]));
            }
        }

        let mut pairs = Vec::new();
        for ((a, b), edges) in &pair_edges {
            if edges.len() < 2 { continue; }
            for i in 0..edges.len() {
                for j in (i + 1)..edges.len() {
                    let (_, e1) = &edges[i];
                    let (_, e2) = &edges[j];
                    if (e1.confidence > 0.7 && e2.confidence < 0.3)
                        || (e2.confidence > 0.7 && e1.confidence < 0.3)
                    {
                        let node_a = self.graph.node_indices()
                            .find(|&n| n.index() == *a)
                            .map(|n| self.graph[n].label.clone())
                            .unwrap_or_else(|| "?".into());
                        let node_b = self.graph.node_indices()
                            .find(|&n| n.index() == *b)
                            .map(|n| self.graph[n].label.clone())
                            .unwrap_or_else(|| "?".into());
                        pairs.push(ContradictionPair {
                            node_a_label: node_a,
                            node_b_label: node_b,
                            edge_a_relation: e1.relation.clone(),
                            edge_a_confidence: e1.confidence,
                            edge_a_context: e1.context.clone(),
                            edge_b_relation: e2.relation.clone(),
                            edge_b_confidence: e2.confidence,
                            edge_b_context: e2.context.clone(),
                        });
                    }
                }
            }
        }

        pairs
    }

    /// Compute uncertainty statistics for this graph.
    pub fn uncertainty_stats(&self) -> UncertaintyStats {
        let mut edge_count = 0usize;
        let mut uncertain_count = 0usize;
        let mut total_confidence = 0.0f64;

        for edge_idx in self.graph.edge_indices() {
            let edge = &self.graph[edge_idx];
            edge_count += 1;
            total_confidence += edge.confidence;
            if edge.confidence < 0.6 {
                uncertain_count += 1;
            }
        }

        UncertaintyStats {
            edge_count,
            uncertain_edge_count: uncertain_count,
            avg_confidence: if edge_count > 0 { total_confidence / edge_count as f64 } else { 0.0 },
        }
    }

    /// Export the graph in a format suitable for 3D force-directed visualization.
    /// Returns { nodes: [...], links: [...] } compatible with 3d-force-graph.
    /// Link orphan nodes to a central hub. Call this to reprocess an existing graph.
    /// The hub node is named after the graph. Orphan edges use relation "?" with
    /// very low confidence (0.05) and are flagged as provisional.
    /// Backfill refutation logs on edges that predate the audit trail.
    /// Creates an initial entry from existing metadata (basis, confidence, source).
    pub fn backfill_refutation_logs(&mut self) {
        let edge_indices: Vec<_> = self.graph.edge_indices().collect();
        let mut backfilled = 0;
        for eid in edge_indices {
            let edge = &self.graph[eid];
            if !edge.refutation_log.is_empty() { continue; }

            // Build initial entry from what we know.
            let basis_name = format!("{:?}", edge.basis).to_lowercase();
            let date = if !edge.since.is_empty() { edge.since.clone() }
                else if !edge.valid_from.is_empty() { edge.valid_from.clone() }
                else { "unknown".into() };
            let source = if !edge.source.is_empty() {
                format!("Source: {}", edge.source)
            } else {
                "No source recorded (pre-audit edge)".into()
            };

            let mut log = vec![RefutationEntry {
                date: date.clone(),
                test: "Initial conjecture (backfilled from pre-audit edge)".into(),
                evidence: source,
                outcome: format!("conjectured (basis: {})", basis_name),
                confidence_before: 0.0,
                confidence_after: edge.basis.initial_confidence(),
            }];

            // If there have been tests, record a summary entry.
            if edge.tests > 0 {
                log.push(RefutationEntry {
                    date: if !edge.last_tested.is_empty() { edge.last_tested.clone() } else { date },
                    test: format!("Historical: {} tests, {} survived (backfilled)", edge.tests, edge.survived),
                    evidence: "Pre-audit testing history — individual test details not recorded".into(),
                    outcome: if edge.survived == edge.tests { "survived".into() }
                        else if edge.confidence < 0.2 { "weakened significantly".into() }
                        else { "partially survived".into() },
                    confidence_before: edge.basis.initial_confidence(),
                    confidence_after: edge.confidence,
                });
            }

            self.graph[eid].refutation_log = log;
            backfilled += 1;
        }
        if backfilled > 0 {
            log::info!("Backfilled refutation logs on {} edges", backfilled);
        }
    }

    pub fn link_orphans(&mut self, graph_name: &str) {
        // Find or create the hub node.
        let hub_idx = self.find_by_label(graph_name).unwrap_or_else(|| {
            self.graph.add_node(KnowledgeNode {
                label: graph_name.into(),
                kind: NodeKind::Concept,
                summary: format!("Central hub for {} graph", graph_name),
                created: String::new(),
                updated: String::new(),
                tags: vec!["hub".into(), "graph".into()],
                _extra: serde_json::Map::new(),
            })
        });

        // Find all nodes not reachable from the hub (disconnected components).
        // This catches both true orphans AND disconnected sub-clusters.
        let mut reachable = HashSet::new();
        let mut queue = std::collections::VecDeque::new();
        queue.push_back(hub_idx);
        reachable.insert(hub_idx);
        while let Some(current) = queue.pop_front() {
            for neighbor in self.graph.neighbors_undirected(current) {
                if reachable.insert(neighbor) {
                    queue.push_back(neighbor);
                }
            }
        }
        // Any node NOT reachable from hub needs a "?" link.
        let orphans: Vec<NodeIndex> = self.graph.node_indices()
            .filter(|idx| !reachable.contains(idx))
            .collect();

        // Group orphans into their connected components.
        // Link one representative from each component to the hub.
        let mut linked = HashSet::new();
        for &orphan in &orphans {
            if linked.contains(&orphan) { continue; }
            // BFS to find this orphan's component.
            let mut component = Vec::new();
            let mut q = std::collections::VecDeque::new();
            q.push_back(orphan);
            linked.insert(orphan);
            while let Some(n) = q.pop_front() {
                component.push(n);
                for neighbor in self.graph.neighbors_undirected(n) {
                    if !reachable.contains(&neighbor) && linked.insert(neighbor) {
                        q.push_back(neighbor);
                    }
                }
            }
            // Pick the most connected node as representative.
            let rep = component.iter()
                .max_by_key(|&&n| self.graph.edges(n).count())
                .copied()
                .unwrap_or(orphan);
            if !self.has_edge_between(hub_idx, rep, "?") {
                self.graph.add_edge(hub_idx, rep, KnowledgeEdge {
                    relation: "?".into(),
                    context: "Unlinked — connection not yet determined".into(),
                    since: String::new(),
                    confidence: 0.05,
                    tests: 0,
                    survived: 0,
                    basis: Basis::Assumed,
                    last_tested: String::new(),
                    importance: 0.1,
                    references: 0,
                    valid_from: String::new(),
                    valid_until: String::new(),
                    view: EdgeView::Entity,
                    source: "auto: orphan linking".into(),
                    refutation_log: Vec::new(),
                    log_odds: to_log_odds(0.05),
                    evidence_log: Vec::new(),
                    justificatory_chain: Vec::new(),
                    source_id: "auto:orphan-linking".into(),
                    decay_category: DecayCategory::Assumed,
                    beneficial_impact: 0.0,
                    corroboration_strength: 0.0,
                    competition_group: String::new(),
                });
            }
        }
    }

    /// Export the graph in a format suitable for 3D force-directed visualization.
    pub fn to_visualization(&self) -> serde_json::Value {

        let nodes: Vec<serde_json::Value> = self.graph.node_indices().map(|idx| {
            let node = &self.graph[idx];
            let is_hub = node.tags.iter().any(|t| t == "hub");
            // Derive node confidence from average of connected edge confidences.
            let edge_confs: Vec<f64> = self.graph.edges(idx)
                .map(|e| e.weight().confidence)
                .filter(|&c| c > 0.05)  // Exclude "?" orphan links from the average.
                .collect();
            let confidence = if edge_confs.is_empty() {
                if is_hub { 1.0 } else { 0.2 }  // Orphans are faint.
            } else {
                edge_confs.iter().sum::<f64>() / edge_confs.len() as f64
            };
            serde_json::json!({
                "id": idx.index(),
                "label": node.label,
                "kind": node.kind.to_string(),
                "summary": node.summary,
                "tags": node.tags,
                "created": node.created,
                "updated": node.updated,
                "confidence": confidence,
                "is_hub": is_hub,
            })
        }).collect();

        // Build a set of valid node IDs for filtering edges.
        let valid_nodes: HashSet<usize> = self.graph.node_indices().map(|i| i.index()).collect();

        let links: Vec<serde_json::Value> = self.graph.edge_indices().filter_map(|e| {
            let (src, tgt) = self.graph.edge_endpoints(e)?;
            // Skip self-loops and edges referencing missing nodes.
            if src == tgt { return None; }
            if !valid_nodes.contains(&src.index()) || !valid_nodes.contains(&tgt.index()) {
                return None;
            }
            let edge = &self.graph[e];
            let is_orphan_link = edge.relation == "?";
            Some(serde_json::json!({
                "source": src.index(),
                "target": tgt.index(),
                "relation": edge.relation,
                "confidence": edge.confidence,
                "log_odds": edge.log_odds,
                "importance": edge.importance,
                "basis": format!("{:?}", edge.basis).to_lowercase(),
                "view": format!("{:?}", edge.view).to_lowercase(),
                "tests": edge.tests,
                "survived": edge.survived,
                "valid_from": edge.valid_from,
                "valid_until": edge.valid_until,
                "source_doc": edge.source,
                "decay_category": format!("{:?}", edge.decay_category).to_lowercase(),
                "evidence_count": edge.evidence_log.len(),
                "is_orphan_link": is_orphan_link,
            }))
        }).collect();

        serde_json::json!({
            "nodes": nodes,
            "links": links,
        })
    }

    // --- Query API ---

    /// Query: "What do I know about X?"
    /// Finds the node, traverses outward to the given depth, returns the
    /// subgraph with all edges and cumulative confidence.
    pub fn query_about(&self, label: &str, max_depth: usize) -> QueryResult {
        let mut result = QueryResult::default();
        let root = match self.find_by_label(label) {
            Some(idx) => idx,
            None => {
                // Fuzzy: try substring match.
                let lower = label.to_lowercase();
                match self.graph.node_indices().find(|&idx| {
                    let l = self.graph[idx].label.to_lowercase();
                    l.contains(&lower) || lower.contains(&l)
                }) {
                    Some(idx) => idx,
                    None => return result,
                }
            }
        };

        // BFS traversal from root.
        let mut visited = HashSet::new();
        let mut frontier = vec![root];
        visited.insert(root);

        for _depth in 0..max_depth {
            let mut next_frontier = Vec::new();
            for &node in &frontier {
                // Outgoing edges.
                for edge_ref in self.graph.edges_directed(node, Direction::Outgoing) {
                    let target = edge_ref.target();
                    let edge = edge_ref.weight();
                    result.edges.push(WeightedEdge {
                        from: node,
                        to: target,
                        edge: edge.clone(),
                        path_confidence: edge.confidence,
                    });
                    if visited.insert(target) {
                        next_frontier.push(target);
                    }
                }
                // Incoming edges (what points TO this node).
                for edge_ref in self.graph.edges_directed(node, Direction::Incoming) {
                    let source = edge_ref.source();
                    let edge = edge_ref.weight();
                    result.edges.push(WeightedEdge {
                        from: source,
                        to: node,
                        edge: edge.clone(),
                        path_confidence: edge.confidence,
                    });
                    if visited.insert(source) {
                        next_frontier.push(source);
                    }
                }
            }
            frontier = next_frontier;
        }

        // Collect nodes with relevance scores (closer = higher).
        for &idx in &visited {
            let node = &self.graph[idx];
            // Score: 1.0 for root, diminishes with distance.
            let score = if idx == root { 1.0 } else {
                // Average confidence of edges connecting to this node in result.
                let connecting: Vec<f64> = result.edges.iter()
                    .filter(|e| e.from == idx || e.to == idx)
                    .map(|e| e.edge.relevance_score())
                    .collect();
                if connecting.is_empty() { 0.5 } else {
                    connecting.iter().sum::<f64>() / connecting.len() as f64
                }
            };
            result.nodes.push((idx, node.clone(), score));
        }
        result.nodes.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
        result
    }

    /// Query: "How is X connected to Y?"
    /// Finds shortest path(s) between two nodes, with cumulative confidence
    /// (product of edge confidences along the path — weakest link model).
    pub fn query_path(&self, from_label: &str, to_label: &str, max_paths: usize) -> QueryResult {
        let mut result = QueryResult::default();
        let from = match self.find_by_label(from_label) {
            Some(idx) => idx,
            None => return result,
        };
        let to = match self.find_by_label(to_label) {
            Some(idx) => idx,
            None => return result,
        };

        // BFS to find shortest paths (unweighted distance, but track confidence).
        let mut queue: std::collections::VecDeque<(NodeIndex, Vec<NodeIndex>, f64)> = std::collections::VecDeque::new();
        queue.push_back((from, vec![from], 1.0));
        let mut visited = HashSet::new();
        visited.insert(from);

        while let Some((current, path, cum_conf)) = queue.pop_front() {
            if result.paths.len() >= max_paths { break; }
            if path.len() > 10 { continue; } // max depth safety

            for edge_ref in self.graph.edges_directed(current, Direction::Outgoing) {
                let target = edge_ref.target();
                let edge_conf = edge_ref.weight().confidence;
                let new_conf = cum_conf * edge_conf; // product = weakest link chain

                if target == to {
                    let mut full_path = path.clone();
                    full_path.push(to);
                    result.paths.push(ConfidencePath {
                        nodes: full_path,
                        cumulative_confidence: new_conf,
                    });
                    continue;
                }

                if visited.insert(target) {
                    let mut new_path = path.clone();
                    new_path.push(target);
                    queue.push_back((target, new_path, new_conf));
                }
            }

            // Also traverse incoming edges (undirected search).
            for edge_ref in self.graph.edges_directed(current, Direction::Incoming) {
                let source = edge_ref.source();
                let edge_conf = edge_ref.weight().confidence;
                let new_conf = cum_conf * edge_conf;

                if source == to {
                    let mut full_path = path.clone();
                    full_path.push(to);
                    result.paths.push(ConfidencePath {
                        nodes: full_path,
                        cumulative_confidence: new_conf,
                    });
                    continue;
                }

                if visited.insert(source) {
                    let mut new_path = path.clone();
                    new_path.push(source);
                    queue.push_back((source, new_path, new_conf));
                }
            }
        }

        // Sort paths by confidence (highest first).
        result.paths.sort_by(|a, b| b.cumulative_confidence
            .partial_cmp(&a.cumulative_confidence).unwrap_or(std::cmp::Ordering::Equal));

        // Collect all nodes and edges from found paths.
        let mut node_set = HashSet::new();
        for path in &result.paths {
            for &idx in &path.nodes {
                node_set.insert(idx);
            }
        }
        for &idx in &node_set {
            let node = &self.graph[idx];
            result.nodes.push((idx, node.clone(), 1.0));
        }
        result
    }

    /// Query: "What decisions have we made?" — filter by node kind.
    pub fn query_by_kind(&self, kind: &NodeKind) -> QueryResult {
        let mut result = QueryResult::default();
        for idx in self.graph.node_indices() {
            if &self.graph[idx].kind == kind {
                let node = &self.graph[idx];
                // Score by average outgoing edge confidence.
                let avg_conf: f64 = {
                    let confs: Vec<f64> = self.graph.edges(idx)
                        .map(|e| e.weight().confidence)
                        .collect();
                    if confs.is_empty() { 0.5 } else { confs.iter().sum::<f64>() / confs.len() as f64 }
                };
                result.nodes.push((idx, node.clone(), avg_conf));

                // Include edges from these nodes.
                for edge_ref in self.graph.edges_directed(idx, Direction::Outgoing) {
                    result.edges.push(WeightedEdge {
                        from: idx,
                        to: edge_ref.target(),
                        edge: edge_ref.weight().clone(),
                        path_confidence: edge_ref.weight().confidence,
                    });
                }
            }
        }
        result.nodes.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
        result
    }

    /// Query: "What's uncertain?" — edges below a confidence threshold.
    pub fn query_uncertain(&self, threshold: f64) -> QueryResult {
        let mut result = QueryResult::default();
        let mut node_set = HashSet::new();
        for edge_idx in self.graph.edge_indices() {
            let edge = &self.graph[edge_idx];
            if edge.confidence < threshold && edge.confidence >= MIN_PROMPT_CONFIDENCE {
                if let Some((src, tgt)) = self.graph.edge_endpoints(edge_idx) {
                    result.edges.push(WeightedEdge {
                        from: src, to: tgt,
                        edge: edge.clone(),
                        path_confidence: edge.confidence,
                    });
                    node_set.insert(src);
                    node_set.insert(tgt);
                }
            }
        }
        for &idx in &node_set {
            result.nodes.push((idx, self.graph[idx].clone(), 0.5));
        }
        result
    }

    /// Query by edge view (MAGMA-inspired): "show me all causal relationships".
    #[allow(dead_code)]
    pub fn query_by_view(&self, view: &EdgeView) -> QueryResult {
        let mut result = QueryResult::default();
        let mut node_set = HashSet::new();
        for edge_idx in self.graph.edge_indices() {
            let edge = &self.graph[edge_idx];
            if &edge.view == view && edge.confidence >= MIN_PROMPT_CONFIDENCE {
                if let Some((src, tgt)) = self.graph.edge_endpoints(edge_idx) {
                    result.edges.push(WeightedEdge {
                        from: src, to: tgt,
                        edge: edge.clone(),
                        path_confidence: edge.confidence,
                    });
                    node_set.insert(src);
                    node_set.insert(tgt);
                }
            }
        }
        for &idx in &node_set {
            result.nodes.push((idx, self.graph[idx].clone(), 0.5));
        }
        result
    }

    /// Query: "what changed?" — edges with temporal validity (non-empty valid_until).
    #[allow(dead_code)]
    pub fn query_historical(&self) -> QueryResult {
        let mut result = QueryResult::default();
        let mut node_set = HashSet::new();
        for edge_idx in self.graph.edge_indices() {
            let edge = &self.graph[edge_idx];
            if !edge.valid_until.is_empty() {
                if let Some((src, tgt)) = self.graph.edge_endpoints(edge_idx) {
                    result.edges.push(WeightedEdge {
                        from: src, to: tgt,
                        edge: edge.clone(),
                        path_confidence: edge.confidence,
                    });
                    node_set.insert(src);
                    node_set.insert(tgt);
                }
            }
        }
        for &idx in &node_set {
            result.nodes.push((idx, self.graph[idx].clone(), 0.5));
        }
        result
    }

    /// Check if an edge is currently valid (no valid_until, or valid_until is in the future).
    fn is_edge_current(edge: &KnowledgeEdge) -> bool {
        edge.valid_until.is_empty()
    }

    /// Render a QueryResult as natural language.
    pub fn render_query_result(&self, result: &QueryResult, max_chars: usize) -> String {
        let mut output = String::new();

        // Render paths first (if any).
        if !result.paths.is_empty() {
            for path in &result.paths {
                let labels: Vec<&str> = path.nodes.iter()
                    .filter_map(|&idx| self.graph.node_weight(idx).map(|n| n.label.as_str()))
                    .collect();
                let conf_label = if path.cumulative_confidence >= 0.8 { "ESTABLISHED" }
                    else if path.cumulative_confidence >= 0.6 { "LIKELY" }
                    else if path.cumulative_confidence >= 0.4 { "POSSIBLE" }
                    else if path.cumulative_confidence >= 0.2 { "UNCERTAIN" }
                    else { "DOUBTFUL" };
                output.push_str(&format!("{} [{} {:.0}%]\n",
                    labels.join(" → "), conf_label, path.cumulative_confidence * 100.0));
            }
            output.push('\n');
        }

        // Render nodes with their edges.
        for (idx, node, score) in &result.nodes {
            let score_label = if *score >= 0.8 { "ESTABLISHED" }
                else if *score >= 0.6 { "LIKELY" }
                else if *score >= 0.4 { "POSSIBLE" }
                else if *score >= 0.2 { "UNCERTAIN" }
                else { "DOUBTFUL" };
            output.push_str(&format!("- {} ({}): {} [{} {:.0}%]\n",
                node.label, node.kind, node.summary, score_label, score * 100.0));
            for edge in &result.edges {
                if &edge.from == idx {
                    if let Some(target) = self.graph.node_weight(edge.to) {
                        let elabel = edge.edge.confidence_label().to_uppercase();
                        output.push_str(&format!("  → {} → {} [{} {:.0}%]\n",
                            edge.edge.relation, target.label,
                            elabel, edge.edge.confidence * 100.0));
                    }
                }
            }

            if output.len() > max_chars {
                output.push_str("...\n");
                break;
            }
        }

        output
    }

    // --- Layer 2: Context-aware retrieval (keyword-based fallback) ---

    /// Extract relevant node indices based on message keywords.
    /// Returns nodes sorted by relevance (most keyword hits first),
    /// expanded 1 hop to include neighbors.
    pub fn relevant_subgraph(&self, message: &str, max_nodes: usize) -> Vec<NodeIndex> {
        let keywords = extract_keywords(message);
        if keywords.is_empty() {
            // No useful keywords — return all (capped).
            let mut all: Vec<NodeIndex> = self.graph.node_indices().collect();
            all.truncate(max_nodes);
            return all;
        }

        let total_nodes = self.graph.node_count().max(1) as f64;

        // Score each node by TF-IDF weighted keyword hits.
        let mut scores: HashMap<NodeIndex, f64> = HashMap::new();
        for kw in &keywords {
            if let Some(nodes) = self.keyword_index.get(kw) {
                // IDF: log(total_nodes / docs_containing_term)
                let idf = (total_nodes / nodes.len().max(1) as f64).ln().max(0.1);
                for &idx in nodes {
                    *scores.entry(idx).or_default() += idf;
                }
            }
        }

        // Sort by TF-IDF score descending, take top N/2.
        let mut scored: Vec<(NodeIndex, f64)> = scores.into_iter().collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let direct_limit = max_nodes / 2;
        let direct: Vec<NodeIndex> = scored.iter().take(direct_limit).map(|(idx, _)| *idx).collect();

        // Expand 1 hop, but only through edges with meaningful confidence.
        let mut result: HashSet<NodeIndex> = direct.iter().copied().collect();
        for &idx in &direct {
            for edge in self.graph.edges(idx) {
                if edge.weight().confidence >= 0.3 {
                    result.insert(edge.target());
                }
            }
            // Also check incoming edges.
            for edge in self.graph.edges_directed(idx, petgraph::Direction::Incoming) {
                if edge.weight().confidence >= 0.3 {
                    result.insert(edge.source());
                }
            }
        }

        // Cap total.
        let mut result_vec: Vec<NodeIndex> = result.into_iter().collect();
        result_vec.truncate(max_nodes);
        result_vec
    }

    // --- Rendering ---

    /// Render the full graph as natural language.
    pub fn render_full(&self, max_chars: usize) -> String {
        let indices: Vec<NodeIndex> = self.graph.node_indices().collect();
        self.render_nodes(&indices, max_chars)
    }

    /// Render a subset of nodes as natural language.
    pub fn render_subgraph(&self, indices: &[NodeIndex], max_chars: usize) -> String {
        self.render_nodes(indices, max_chars)
    }

    /// Render query result as compact JSON — easier for AI to parse than prose.
    /// Each node with its edges, sorted by relevance. Truncated to max_chars.
    #[allow(dead_code)]
    pub fn render_query_json(&self, result: &QueryResult, max_chars: usize) -> String {
        let mut nodes_json: Vec<serde_json::Value> = Vec::new();

        for (idx, node, score) in &result.nodes {
            let edges: Vec<serde_json::Value> = self.graph.edges(*idx)
                .filter(|e| e.weight().confidence >= MIN_PROMPT_CONFIDENCE)
                .map(|e| {
                    let target = &self.graph[e.target()];
                    serde_json::json!({
                        "to": target.label,
                        "rel": e.weight().relation,
                        "conf": format!("{:.0}%", e.weight().confidence * 100.0),
                    })
                })
                .collect();

            nodes_json.push(serde_json::json!({
                "label": node.label,
                "kind": format!("{:?}", node.kind).to_lowercase(),
                "summary": node.summary,
                "score": format!("{:.2}", score),
                "edges": edges,
            }));

            // Check budget.
            let partial = serde_json::to_string(&nodes_json).unwrap_or_default();
            if partial.len() > max_chars {
                nodes_json.pop();
                break;
            }
        }

        serde_json::to_string_pretty(&nodes_json).unwrap_or_else(|_| "[]".into())
    }

    fn render_nodes(&self, indices: &[NodeIndex], max_chars: usize) -> String {
        if indices.is_empty() {
            return String::new();
        }

        // Group nodes by kind.
        let mut by_kind: HashMap<String, Vec<NodeIndex>> = HashMap::new();
        for &idx in indices {
            let kind = self.graph[idx].kind.to_string();
            by_kind.entry(kind).or_default().push(idx);
        }

        let idx_set: HashSet<NodeIndex> = indices.iter().copied().collect();
        let mut output = String::new();

        // Render each kind section.
        let kind_order = ["person", "project", "server", "tool", "concept", "decision", "event", "fact"];
        for kind in &kind_order {
            let Some(nodes) = by_kind.get(*kind) else { continue };
            output.push_str(&format!("## {}\n", capitalize(kind)));
            for &idx in nodes {
                let node = &self.graph[idx];
                output.push_str(&format!("- {} ({}): {}", node.label, kind, node.summary));
                if !node.updated.is_empty() {
                    output.push_str(&format!(" [{}]", node.updated));
                }
                output.push('\n');

                // Show edges to/from this node (within the subgraph).
                // Filter: current (not expired), above confidence threshold.
                // Sort by relevance (confidence × importance).
                let mut edges: Vec<_> = self.graph.edges_directed(idx, Direction::Outgoing)
                    .filter(|e| idx_set.contains(&e.target()))
                    .filter(|e| e.weight().confidence >= MIN_PROMPT_CONFIDENCE)
                    .filter(|e| Self::is_edge_current(e.weight()))
                    .collect();
                edges.sort_by(|a, b| b.weight().relevance_score()
                    .partial_cmp(&a.weight().relevance_score()).unwrap_or(std::cmp::Ordering::Equal));
                for edge_idx in edges {
                    let target = edge_idx.target();
                    let edge = edge_idx.weight();
                    let target_node = &self.graph[target];
                    let label = edge.confidence_label().to_uppercase();
                    let conf_str = format!(" [{} {:.0}%{}]",
                        label,
                        edge.confidence * 100.0,
                        if edge.tests > 0 { format!(" {}×", edge.tests) } else { String::new() });
                    output.push_str(&format!(
                        "  → {} → {}{}\n",
                        edge.relation, target_node.label, conf_str
                    ));
                }

                if output.len() > max_chars {
                    output.push_str("\n... (truncated — read knowledge.json for full graph)\n");
                    return output;
                }
            }
            output.push('\n');
        }

        output
    }
}

// --- Graph consolidation ---

/// Result of a consolidation pass.
#[derive(Debug, Default)]
pub struct ConsolidationReport {
    pub nodes_merged: usize,
    pub edges_merged: usize,
    pub chains_collapsed: usize,
    pub contradictions: Vec<String>,
    /// Disconnected clusters found (GraphRAG-inspired community detection).
    #[allow(dead_code)]
    pub clusters: Vec<Vec<String>>,
}

impl KnowledgeGraph {
    /// Run a full consolidation pass: dedup nodes, merge parallel edges,
    /// collapse chains, detect contradictions.
    pub fn consolidate(&mut self) -> ConsolidationReport {
        // 1. Deduplicate nodes.
        let nodes_merged = self.dedup_nodes();
        // 2. Merge parallel edges (same source, target, relation).
        let edges_merged = self.merge_parallel_edges();
        // 3. Collapse chains: A→B→C where B has degree 2 and is a Fact.
        let chains_collapsed = self.collapse_chains();
        // 4. Detect contradictions.
        let contradictions = self.detect_contradictions();
        // 5. Community detection (GraphRAG-inspired) — find disconnected clusters.
        let clusters = self.detect_communities();

        self.rebuild_index();
        ConsolidationReport { nodes_merged, edges_merged, chains_collapsed, contradictions, clusters }
    }

    /// Find and merge nodes with similar labels and same kind.
    fn dedup_nodes(&mut self) -> usize {
        let mut merged = 0;
        let indices: Vec<NodeIndex> = self.graph.node_indices().collect();

        // Build label→index map for matching.
        let mut to_merge: Vec<(NodeIndex, NodeIndex)> = Vec::new();
        for i in 0..indices.len() {
            for j in (i + 1)..indices.len() {
                let a = indices[i];
                let b = indices[j];
                if !self.graph.contains_node(a) || !self.graph.contains_node(b) {
                    continue;
                }
                if self.graph[a].kind != self.graph[b].kind {
                    continue;
                }
                if labels_match(&self.graph[a].label, &self.graph[b].label) {
                    to_merge.push((a, b));
                }
            }
        }

        for (keep, remove) in to_merge {
            if !self.graph.contains_node(keep) || !self.graph.contains_node(remove) {
                continue;
            }
            self.merge_nodes(keep, remove);
            merged += 1;
        }

        merged
    }

    /// Merge node `remove` into `keep`: move all edges, keep the better summary.
    fn merge_nodes(&mut self, keep: NodeIndex, remove: NodeIndex) {
        // Keep the longer summary.
        let remove_node = self.graph[remove].clone();
        let keep_node = &mut self.graph[keep];
        if remove_node.summary.len() > keep_node.summary.len() {
            keep_node.summary = remove_node.summary;
        }
        // Union tags.
        for tag in &remove_node.tags {
            if !keep_node.tags.contains(tag) {
                keep_node.tags.push(tag.clone());
            }
        }

        // Move edges from `remove` to `keep`.
        let edges_out: Vec<_> = self.graph.edges_directed(remove, Direction::Outgoing)
            .map(|e| (e.target(), e.weight().clone(), e.id()))
            .collect();
        for (target, weight, _) in edges_out {
            let actual_target = if target == remove { keep } else { target };
            self.graph.add_edge(keep, actual_target, weight);
        }
        let edges_in: Vec<_> = self.graph.edges_directed(remove, Direction::Incoming)
            .map(|e| (e.source(), e.weight().clone(), e.id()))
            .collect();
        for (source, weight, _) in edges_in {
            let actual_source = if source == remove { keep } else { source };
            self.graph.add_edge(actual_source, keep, weight);
        }

        self.graph.remove_node(remove);
    }

    /// Merge parallel edges (same source, target, and relation).
    fn merge_parallel_edges(&mut self) -> usize {
        use petgraph::graph::EdgeIndex;
        let mut merged = 0;

        // Collect all edge info first to avoid borrow conflicts.
        let edge_info: Vec<(EdgeIndex, usize, usize, String, KnowledgeEdge)> = self.graph
            .edge_indices()
            .filter_map(|e| {
                let (src, tgt) = self.graph.edge_endpoints(e)?;
                Some((e, src.index(), tgt.index(), self.graph[e].relation.clone(), self.graph[e].clone()))
            })
            .collect();

        let mut seen: HashMap<(usize, usize, String), (EdgeIndex, KnowledgeEdge)> = HashMap::new();
        let mut to_remove = Vec::new();
        let mut to_update: Vec<(EdgeIndex, KnowledgeEdge)> = Vec::new();

        for (idx, src, tgt, rel, mut edge) in edge_info {
            let key = (src, tgt, rel);
            edge.ensure_log_odds();
            if let Some((kept_idx, kept_edge)) = seen.get_mut(&key) {
                kept_edge.ensure_log_odds();
                // Bayesian merge: evidence-weighted average of log-odds.
                let k_weight = kept_edge.evidence_log.len().max(1) as f64;
                let e_weight = edge.evidence_log.len().max(1) as f64;
                kept_edge.log_odds = (kept_edge.log_odds * k_weight + edge.log_odds * e_weight)
                    / (k_weight + e_weight);
                kept_edge.confidence = to_probability(kept_edge.log_odds).clamp(0.01, 0.95);
                kept_edge.tests += edge.tests;
                kept_edge.survived += edge.survived;
                kept_edge.references += edge.references;
                kept_edge.importance = kept_edge.importance.max(edge.importance);
                // Combine evidence logs (deduplicate by date+test to avoid double-counting).
                let existing_keys: HashSet<(String, String)> = kept_edge.evidence_log.iter()
                    .map(|e| (e.date.clone(), e.test.clone()))
                    .collect();
                for entry in &edge.evidence_log {
                    let key = (entry.date.clone(), entry.test.clone());
                    if !existing_keys.contains(&key) {
                        kept_edge.evidence_log.push(entry.clone());
                    }
                }
                let existing_refutation_keys: HashSet<(String, String)> = kept_edge.refutation_log.iter()
                    .map(|e| (e.date.clone(), e.test.clone()))
                    .collect();
                for entry in &edge.refutation_log {
                    let key = (entry.date.clone(), entry.test.clone());
                    if !existing_refutation_keys.contains(&key) {
                        kept_edge.refutation_log.push(entry.clone());
                    }
                }
                // Combine context from both sources.
                if !edge.context.is_empty() && edge.context != kept_edge.context {
                    if kept_edge.context.is_empty() {
                        kept_edge.context = edge.context;
                    } else {
                        kept_edge.context = format!("{}; {}", kept_edge.context, edge.context);
                    }
                }
                to_update.push((*kept_idx, kept_edge.clone()));
                to_remove.push(idx);
                merged += 1;
            } else {
                seen.insert(key, (idx, edge));
            }
        }

        // Apply updates.
        for (idx, edge) in to_update {
            if self.graph.edge_weight_mut(idx).is_some() {
                self.graph[idx] = edge;
            }
        }
        for e in to_remove.into_iter().rev() {
            self.graph.remove_edge(e);
        }

        merged
    }

    /// Collapse chain nodes: A→B→C where B has exactly 1 incoming + 1 outgoing
    /// and is a Fact-type node with low importance.
    fn collapse_chains(&mut self) -> usize {
        let mut collapsed = 0;
        let candidates: Vec<NodeIndex> = self.graph.node_indices().filter(|&n| {
            self.graph[n].kind == NodeKind::Fact
                && self.graph.edges_directed(n, Direction::Incoming).count() == 1
                && self.graph.edges_directed(n, Direction::Outgoing).count() == 1
        }).collect();

        for mid in candidates {
            if !self.graph.contains_node(mid) { continue; }

            let in_edge = match self.graph.edges_directed(mid, Direction::Incoming).next() {
                Some(e) => (e.source(), e.weight().clone(), e.id()),
                None => continue,
            };
            let out_edge = match self.graph.edges_directed(mid, Direction::Outgoing).next() {
                Some(e) => (e.target(), e.weight().clone(), e.id()),
                None => continue,
            };

            let (src, mut in_w, _) = in_edge;
            let (tgt, mut out_w, _) = out_edge;
            in_w.ensure_log_odds();
            out_w.ensure_log_odds();

            // Don't collapse if either edge is high-importance.
            if in_w.importance > 0.7 || out_w.importance > 0.7 { continue; }
            // Don't collapse self-loops.
            if src == tgt { continue; }

            // Create combined edge.
            // Bayesian merging: use the weaker log_odds (conservative)
            let combined_lo = if in_w.log_odds.abs() < out_w.log_odds.abs() {
                in_w.log_odds
            } else {
                out_w.log_odds
            };
            let combined_conf = if combined_lo != 0.0 { to_probability(combined_lo) }
                else { in_w.confidence.min(out_w.confidence) };
            let combined = KnowledgeEdge {
                relation: format!("{} → {}", in_w.relation, out_w.relation),
                context: format!("{} (via {})", in_w.context, self.graph[mid].label),
                since: in_w.since.clone(),
                confidence: combined_conf,
                tests: in_w.tests + out_w.tests,
                survived: in_w.survived + out_w.survived,
                basis: in_w.basis.clone(),
                last_tested: in_w.last_tested.clone(),
                importance: in_w.importance.max(out_w.importance),
                references: in_w.references + out_w.references,
                valid_from: in_w.valid_from.clone(),
                valid_until: out_w.valid_until.clone(),
                view: in_w.view.clone(),
                source: in_w.source.clone(),
                refutation_log: {
                    let mut log = in_w.refutation_log.clone();
                    log.extend(out_w.refutation_log.iter().cloned());
                    log
                },
                log_odds: combined_lo,
                evidence_log: {
                    let mut log = in_w.evidence_log.clone();
                    log.extend(out_w.evidence_log.iter().cloned());
                    log
                },
                justificatory_chain: {
                    let mut chain = in_w.justificatory_chain.clone();
                    chain.extend(out_w.justificatory_chain.iter().cloned());
                    chain
                },
                source_id: in_w.source_id.clone(),
                decay_category: in_w.decay_category.clone(),
                beneficial_impact: in_w.beneficial_impact.max(out_w.beneficial_impact),
                corroboration_strength: (in_w.corroboration_strength + out_w.corroboration_strength) / 2.0,
                competition_group: in_w.competition_group.clone(),
            };

            self.graph.add_edge(src, tgt, combined);
            self.graph.remove_node(mid); // also removes its edges
            collapsed += 1;
        }

        collapsed
    }

    /// Detect potential contradictions: same node pair with edges whose
    /// confidence levels diverge significantly.
    fn detect_contradictions(&self) -> Vec<String> {
        let mut warnings = Vec::new();
        let mut pair_edges: HashMap<(usize, usize), Vec<&KnowledgeEdge>> = HashMap::new();

        for edge_idx in self.graph.edge_indices() {
            if let Some((src, tgt)) = self.graph.edge_endpoints(edge_idx) {
                // Normalize direction for undirected comparison.
                let key = if src.index() <= tgt.index() {
                    (src.index(), tgt.index())
                } else {
                    (tgt.index(), src.index())
                };
                pair_edges.entry(key).or_default().push(&self.graph[edge_idx]);
            }
        }

        for ((a, b), edges) in &pair_edges {
            if edges.len() < 2 { continue; }
            for i in 0..edges.len() {
                for j in (i + 1)..edges.len() {
                    let e1 = edges[i];
                    let e2 = edges[j];
                    // Flag if one is high-confidence and the other is low.
                    if (e1.confidence > 0.7 && e2.confidence < 0.3)
                        || (e2.confidence > 0.7 && e1.confidence < 0.3)
                    {
                        let node_a = self.graph.node_indices()
                            .find(|&n| n.index() == *a)
                            .map(|n| self.graph[n].label.as_str())
                            .unwrap_or("?");
                        let node_b = self.graph.node_indices()
                            .find(|&n| n.index() == *b)
                            .map(|n| self.graph[n].label.as_str())
                            .unwrap_or("?");
                        warnings.push(format!(
                            "{} ↔ {}: '{}' ({:.0}%) vs '{}' ({:.0}%) — possible contradiction",
                            node_a, node_b,
                            e1.relation, e1.confidence * 100.0,
                            e2.relation, e2.confidence * 100.0,
                        ));
                    }
                }
            }
        }

        warnings
    }

    /// Community detection (GraphRAG-inspired) — find connected components.
    /// Returns clusters of node labels. Clusters with 3+ nodes that lack
    /// a concept/theme node may need one.
    fn detect_communities(&self) -> Vec<Vec<String>> {
        use petgraph::algo::connected_components;
        use petgraph::graph::UnGraph;

        // Build an undirected view for component detection.
        let mut undirected = UnGraph::<(), ()>::new_undirected();
        let mut node_map: HashMap<NodeIndex, petgraph::graph::NodeIndex> = HashMap::new();

        for idx in self.graph.node_indices() {
            let u_idx = undirected.add_node(());
            node_map.insert(idx, u_idx);
        }
        for edge_idx in self.graph.edge_indices() {
            if let Some((src, tgt)) = self.graph.edge_endpoints(edge_idx) {
                if let (Some(&u_src), Some(&u_tgt)) = (node_map.get(&src), node_map.get(&tgt)) {
                    undirected.add_edge(u_src, u_tgt, ());
                }
            }
        }

        let num_components = connected_components(&undirected);
        if num_components <= 1 { return Vec::new(); }

        // BFS from each unvisited node to find connected components.
        let mut visited = HashSet::new();
        let mut clusters = Vec::new();
        for idx in self.graph.node_indices() {
            if visited.contains(&idx) { continue; }
            let mut cluster = Vec::new();
            let mut queue = std::collections::VecDeque::new();
            queue.push_back(idx);
            visited.insert(idx);
            while let Some(current) = queue.pop_front() {
                cluster.push(self.graph[current].label.clone());
                for neighbor in self.graph.neighbors_undirected(current) {
                    if visited.insert(neighbor) {
                        queue.push_back(neighbor);
                    }
                }
            }
            if cluster.len() >= 2 {
                clusters.push(cluster);
            }
        }
        // Only report if there are multiple clusters (fragmented graph).
        if clusters.len() > 1 { clusters } else { Vec::new() }
    }
}

/// Check if two labels refer to the same entity.
/// Case-insensitive, and one being a prefix/substring of the other.
fn labels_match(a: &str, b: &str) -> bool {
    let la = a.to_lowercase();
    let lb = b.to_lowercase();
    if la == lb { return true; }
    // One is a substring of the other (e.g. "Anthill" matches "Anthill project").
    if la.len() >= 3 && lb.contains(&la) { return true; }
    if lb.len() >= 3 && la.contains(&lb) { return true; }
    // Fuzzy match for longer labels: Levenshtein distance within 15% of length.
    if la.len() >= 6 && lb.len() >= 6 {
        let dist = levenshtein(&la, &lb);
        let max_len = la.len().max(lb.len());
        let threshold = (max_len as f64 * 0.15).ceil() as usize;
        if dist <= threshold.max(1) { return true; }
    }
    false
}

/// Levenshtein edit distance between two strings.
/// Check if two relation names are semantically similar.
/// Uses word overlap and Levenshtein distance for fuzzy matching.
fn relations_similar(a: &str, b: &str) -> bool {
    if a == b { return true; }
    // Check word overlap.
    let a_words: HashSet<&str> = a.split(|c: char| !c.is_alphanumeric()).filter(|w| w.len() > 2).collect();
    let b_words: HashSet<&str> = b.split(|c: char| !c.is_alphanumeric()).filter(|w| w.len() > 2).collect();
    if !a_words.is_empty() && !b_words.is_empty() {
        let overlap = a_words.intersection(&b_words).count();
        let total = a_words.len().max(b_words.len());
        if overlap as f64 / total as f64 >= 0.5 { return true; }
    }
    // Fallback: Levenshtein distance relative to length.
    let max_len = a.len().max(b.len());
    if max_len == 0 { return true; }
    let dist = levenshtein(a, b);
    (dist as f64 / max_len as f64) < 0.3
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (m, n) = (a.len(), b.len());
    // Use single-row optimization: O(min(m,n)) space.
    let mut prev = vec![0usize; n + 1];
    let mut curr = vec![0usize; n + 1];
    for (j, slot) in prev.iter_mut().enumerate() { *slot = j; }
    for i in 1..=m {
        curr[0] = i;
        for j in 1..=n {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            curr[j] = (prev[j] + 1)
                .min(curr[j - 1] + 1)
                .min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[n]
}

// --- Cached graph (avoids re-parsing JSON on every request) ---

use std::sync::Mutex;
use std::time::SystemTime;

/// A cached knowledge graph that reloads from disk only when the file changes.
/// Cached topic graph with modification tracking.
struct CachedTopicGraph {
    graph: KnowledgeGraph,
    mtime: Option<SystemTime>,
}

pub struct CachedGraph {
    graph: Mutex<KnowledgeGraph>,
    file_path: PathBuf,
    last_mtime: Mutex<Option<SystemTime>>,
    /// Cached topic graphs from memory/graphs/, keyed by filename.
    topic_cache: Mutex<HashMap<PathBuf, CachedTopicGraph>>,
}

impl CachedGraph {
    /// Create a new cached graph from a file path.
    pub fn new(path: &Path) -> Self {
        let mtime = std::fs::metadata(path).ok().and_then(|m| m.modified().ok());
        let graph = KnowledgeGraph::load(path);
        Self {
            graph: Mutex::new(graph),
            file_path: path.to_path_buf(),
            last_mtime: Mutex::new(mtime),
            topic_cache: Mutex::new(HashMap::new()),
        }
    }

    /// Get the graph, reloading if the file changed on disk.
    /// Returns a result string (rendered context) for the given message.
    pub fn render_for_prompt(&self, message: &str, max_chars: usize) -> String {
        self.maybe_reload();
        let graph = match self.graph.lock() {
            Ok(g) => g,
            Err(_) => return String::new(),
        };
        if graph.node_count() == 0 {
            return String::new();
        }
        if graph.node_count() <= 30 {
            return graph.render_full(max_chars);
        }

        // For larger graphs: try structured query first (extract entity names
        // from the message and do graph traversal), fall back to keyword subgraph.
        // Adaptive depth: small graphs get deeper traversal, large graphs stay shallow.
        let keywords = extract_keywords(message);
        let depth = if graph.node_count() < 100 { 2 } else { 1 };
        let mut result = QueryResult::default();
        let mut existing: HashSet<usize> = HashSet::new();

        // Try each keyword as a potential entity label.
        for kw in &keywords {
            let about = graph.query_about(kw, depth);
            for (idx, node, score) in about.nodes {
                if existing.insert(idx.index()) {
                    result.nodes.push((idx, node, score));
                }
            }
            result.edges.extend(about.edges);
        }

        // Sort nodes by relevance (confidence × importance) so most useful context comes first.
        result.nodes.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));

        let mut meta_result = if !result.nodes.is_empty() {
            let mut r = graph.render_query_result(&result, max_chars);
            r.push_str("\n(Query-based context. Read knowledge.json for full graph.)\n");
            r
        } else {
            // Fallback to keyword-based subgraph.
            let relevant = graph.relevant_subgraph(message, 50);
            let mut r = graph.render_subgraph(&relevant, max_chars);
            r.push_str("\n(Keyword-based context. Read knowledge.json for full graph.)\n");
            r
        };

        // Use the META-GRAPH to find which topic graphs are relevant — don't scan all files.
        // The meta-graph has nodes tagged "topic" that represent each graph file.
        // Query the meta-graph for topics matching the message, then load only those.
        let graphs_dir = self.file_path.parent()
            .map(|p| p.join("graphs"));
        if let Some(dir) = graphs_dir {
            if dir.exists() {
                let remaining = max_chars.saturating_sub(meta_result.len());
                if remaining > 200 {
                    // Find relevant topic names from the meta-graph.
                    let relevant_topics: Vec<String> = {
                        let graph = match self.graph.lock() {
                            Ok(g) => g,
                            Err(_) => return meta_result,
                        };
                        // Find topic nodes that match the message keywords.
                        let keywords = extract_keywords(message);
                        graph.graph.node_indices()
                            .filter(|&idx| {
                                let n = &graph.graph[idx];
                                n.tags.iter().any(|t| t == "topic" || t == "graph")
                            })
                            .filter(|&idx| {
                                let n = &graph.graph[idx];
                                let label_lower = n.label.to_lowercase();
                                // Match if any keyword appears in the topic label or summary.
                                keywords.iter().any(|kw| label_lower.contains(kw)
                                    || n.summary.to_lowercase().contains(kw))
                                // Also always include "conversation" graph.
                                || n.label == "conversation"
                            })
                            .map(|idx| graph.graph[idx].label.clone())
                            .collect()
                    };

                    // Load only the relevant topic graphs (cached).
                    let mut topic_context = String::new();
                    let mut cache = self.topic_cache.lock().unwrap_or_else(|e| e.into_inner());
                    for topic_name in &relevant_topics {
                        let path = dir.join(format!("{}.json", topic_name));
                        if !path.exists() { continue; }

                        let current_mtime = std::fs::metadata(&path)
                            .ok().and_then(|m| m.modified().ok());

                        let needs_load = match cache.get(&path) {
                            Some(cached) => cached.mtime != current_mtime,
                            None => true,
                        };
                        if needs_load {
                            let tg = KnowledgeGraph::load(&path);
                            cache.insert(path.clone(), CachedTopicGraph {
                                graph: tg,
                                mtime: current_mtime,
                            });
                        }

                        let topic = match cache.get(&path) {
                            Some(c) => &c.graph,
                            None => continue,
                        };
                        if topic.node_count() == 0 { continue; }

                        let relevant = topic.relevant_subgraph(message, 10);
                        if !relevant.is_empty() {
                            topic_context.push_str(&format!("\n### {}\n", topic_name));
                            topic_context.push_str(&topic.render_subgraph(
                                &relevant,
                                remaining.saturating_sub(topic_context.len()) / 2,
                            ));
                        }
                        if topic_context.len() > remaining { break; }
                    }
                    if !topic_context.is_empty() {
                        meta_result.push_str(&topic_context);
                    }
                }
            }
        }

        meta_result
    }

    /// Run a structured query against the cached graph.
    #[allow(dead_code)]
    pub fn query_about(&self, label: &str, depth: usize) -> QueryResult {
        self.maybe_reload();
        match self.graph.lock() {
            Ok(g) => g.query_about(label, depth),
            Err(_) => QueryResult::default(),
        }
    }

    /// Find paths between two entities.
    #[allow(dead_code)]
    pub fn query_path(&self, from: &str, to: &str, max_paths: usize) -> QueryResult {
        self.maybe_reload();
        match self.graph.lock() {
            Ok(g) => g.query_path(from, to, max_paths),
            Err(_) => QueryResult::default(),
        }
    }

    /// Query by node kind.
    #[allow(dead_code)]
    pub fn query_by_kind(&self, kind: &NodeKind) -> QueryResult {
        self.maybe_reload();
        match self.graph.lock() {
            Ok(g) => g.query_by_kind(kind),
            Err(_) => QueryResult::default(),
        }
    }

    /// Query uncertain edges.
    #[allow(dead_code)]
    pub fn query_uncertain(&self, threshold: f64) -> QueryResult {
        self.maybe_reload();
        match self.graph.lock() {
            Ok(g) => g.query_uncertain(threshold),
            Err(_) => QueryResult::default(),
        }
    }

    /// Reload from disk if the file's mtime has changed.
    fn maybe_reload(&self) {
        let current_mtime = std::fs::metadata(&self.file_path)
            .ok()
            .and_then(|m| m.modified().ok());

        // Also check topic graph mtimes in the graphs/ sibling directory.
        let latest_topic_mtime = self.file_path.parent()
            .map(|p| p.join("graphs"))
            .filter(|d| d.is_dir())
            .and_then(|d| {
                std::fs::read_dir(d).ok().and_then(|entries| {
                    entries.flatten()
                        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("json"))
                        .filter_map(|e| std::fs::metadata(e.path()).ok()?.modified().ok())
                        .max()
                })
            });

        let effective_mtime = match (current_mtime, latest_topic_mtime) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        };

        let needs_reload = {
            let last = self.last_mtime.lock().ok();
            match (last.as_deref(), &effective_mtime) {
                (Some(Some(prev)), Some(curr)) => curr != prev,
                (Some(None), Some(_)) => true, // file appeared
                _ => false,
            }
        };

        if needs_reload {
            let new_graph = KnowledgeGraph::load(&self.file_path);
            log::debug!("Knowledge graph reloaded: {} nodes", new_graph.node_count());
            if let Ok(mut g) = self.graph.lock() {
                *g = new_graph;
            }
            if let Ok(mut m) = self.last_mtime.lock() {
                *m = effective_mtime;
            }
        }
    }

    /// Archive low-confidence edges to a separate file.
    /// Returns the number of edges archived.
    pub fn archive_stale(&self) -> usize {
        self.maybe_reload();
        let mut graph = match self.graph.lock() {
            Ok(g) => g,
            Err(_) => return 0,
        };

        let archive_path = self.file_path.with_file_name("knowledge-archive.json");
        let mut archive = KnowledgeGraph::load(&archive_path);

        // Find edges to archive.
        let stale_edges: Vec<_> = graph.graph.edge_indices().filter(|&e| {
            graph.graph[e].confidence < ARCHIVE_CONFIDENCE
        }).collect();

        if stale_edges.is_empty() { return 0; }

        let mut archived = 0;
        for edge_idx in stale_edges {
            if let Some((from, to)) = graph.graph.edge_endpoints(edge_idx) {
                let edge = graph.graph[edge_idx].clone();
                let from_node = graph.graph[from].clone();
                let to_node = graph.graph[to].clone();

                // Ensure nodes exist in archive.
                let a_from = archive.find_by_label(&from_node.label)
                    .unwrap_or_else(|| archive.graph.add_node(from_node));
                let a_to = archive.find_by_label(&to_node.label)
                    .unwrap_or_else(|| archive.graph.add_node(to_node));
                archive.graph.add_edge(a_from, a_to, edge);
                archived += 1;
            }
        }

        // Remove archived edges from active graph.
        // (Collect indices first, remove in reverse to avoid invalidation.)
        let to_remove: Vec<_> = graph.graph.edge_indices().filter(|&e| {
            graph.graph[e].confidence < ARCHIVE_CONFIDENCE
        }).collect();
        for e in to_remove.into_iter().rev() {
            graph.graph.remove_edge(e);
        }

        // Remove orphan nodes (no edges).
        let orphans: Vec<_> = graph.graph.node_indices().filter(|&n| {
            graph.graph.edges(n).next().is_none()
                && graph.graph.neighbors_undirected(n).next().is_none()
        }).collect();
        for n in orphans.into_iter().rev() {
            graph.graph.remove_node(n);
        }

        // Save archive FIRST — if power dies here, duplicates are safe.
        // Then save the trimmed active graph.
        archive.rebuild_index();
        archive.save();
        graph.rebuild_index();
        graph.save();

        if archived > 0 {
            log::info!("Archived {} low-confidence edges to {}", archived, archive_path.display());
        }
        archived
    }

    /// Run graph consolidation: dedup nodes, merge edges, collapse chains.
    /// Returns a report of what was done.
    pub fn consolidate(&self) -> ConsolidationReport {
        self.maybe_reload();
        let mut graph = match self.graph.lock() {
            Ok(g) => g,
            Err(_) => return ConsolidationReport::default(),
        };
        let report = graph.consolidate();
        if report.nodes_merged > 0 || report.edges_merged > 0 || report.chains_collapsed > 0 {
            graph.save();
            log::info!(
                "Graph consolidated: {} nodes merged, {} edges merged, {} chains collapsed, {} contradictions, {} clusters",
                report.nodes_merged, report.edges_merged, report.chains_collapsed, report.contradictions.len(), report.clusters.len()
            );
            for warning in &report.contradictions {
                log::warn!("Contradiction: {}", warning);
            }
            if report.clusters.len() > 1 {
                log::info!("Graph has {} disconnected clusters — consider linking related topics", report.clusters.len());
            }
        }
        report
    }

    /// Apply time-based confidence decay to all edges.
    /// Called when the ANT has been idle — decays untested conjectures.
    pub fn apply_decay(&self, days: u32) {
        self.maybe_reload();
        let mut graph = match self.graph.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        let edge_indices: Vec<_> = graph.graph.edge_indices().collect();
        let mut decayed = 0u32;
        for edge_idx in edge_indices {
            let before = graph.graph[edge_idx].confidence;
            graph.graph[edge_idx].decay(days);
            if (before - graph.graph[edge_idx].confidence).abs() > 0.001 {
                decayed += 1;
            }
        }
        if decayed > 0 {
            graph.save();
            log::info!("Decayed {} edges by {} days of inactivity", decayed, days);
        }
    }

    /// Semantic search using Ollama embeddings.
    /// Collects node data synchronously, then embeds asynchronously.
    pub async fn semantic_search(
        &self,
        ollama: &crate::ollama::OllamaClient,
        message: &str,
        top_n: usize,
    ) -> Vec<(String, f32)> {
        self.maybe_reload();

        // Collect node data under the lock, then drop it before async work.
        let node_data: Vec<(String, String)> = {
            let graph = match self.graph.lock() {
                Ok(g) => g,
                Err(_) => return Vec::new(),
            };
            graph.graph.node_indices()
                .map(|idx| {
                    let n = &graph.graph[idx];
                    (n.label.clone(), format!("{}: {}", n.label, n.summary))
                })
                .collect()
        };

        if node_data.is_empty() { return Vec::new(); }

        // Embed all nodes (with caching) — async, no lock held.
        let mut node_embeddings: Vec<(String, Vec<f32>)> = Vec::new();
        for (label, text) in &node_data {
            match ollama.embed_cached(label, text).await {
                Ok(vec) => node_embeddings.push((label.clone(), vec)),
                Err(e) => log::debug!("Embed failed for '{}': {}", label, e),
            }
        }

        // Embed the query.
        let query_vec = match ollama.embed(&[message]).await {
            Ok(vecs) if !vecs.is_empty() => vecs.into_iter().next().unwrap(),
            _ => return Vec::new(),
        };

        crate::ollama::OllamaClient::top_similar(&query_vec, &node_embeddings, top_n)
    }

    /// Enhanced render_for_prompt that uses embeddings when Ollama is available.
    /// Hybrid semantic + keyword search for maximum retrieval quality.
    /// Uses embeddings to find semantically similar nodes, then supplements
    /// with keyword matches to catch entities that embeddings miss.
    pub async fn render_for_prompt_semantic(
        &self,
        ollama: &crate::ollama::OllamaClient,
        message: &str,
        max_chars: usize,
    ) -> String {
        // Phase 1: Semantic search (async, no graph lock held).
        let similar = self.semantic_search(ollama, message, 10).await;
        if similar.is_empty() {
            log::debug!("Semantic search returned no results — falling back to keyword only");
        }

        self.maybe_reload();
        let graph = match self.graph.lock() {
            Ok(g) => g,
            Err(_) => return self.render_for_prompt(message, max_chars),
        };

        if graph.node_count() == 0 {
            return String::new();
        }

        let depth = if graph.node_count() < 100 { 2 } else { 1 };
        let mut result = QueryResult::default();
        let mut existing = std::collections::HashSet::new();

        // Add semantic matches first (higher quality).
        for (label, _score) in &similar {
            let about = graph.query_about(label, depth);
            for (idx, node, score) in about.nodes {
                if existing.insert(idx.index()) {
                    result.nodes.push((idx, node, score));
                }
            }
            result.edges.extend(about.edges);
        }

        // Phase 2: Keyword search to fill gaps.
        let keywords = extract_keywords(message);
        for kw in &keywords {
            let about = graph.query_about(kw, depth);
            for (idx, node, score) in about.nodes {
                if existing.insert(idx.index()) {
                    result.nodes.push((idx, node, score));
                }
            }
            result.edges.extend(about.edges);
        }

        if result.nodes.is_empty() {
            // Final fallback: relevance-scored keyword subgraph.
            let relevant = graph.relevant_subgraph(message, 50);
            let mut r = graph.render_subgraph(&relevant, max_chars);
            r.push_str("\n(Keyword-based context. Read knowledge.json for full graph.)\n");
            return r;
        }

        // Sort by relevance so most useful context comes first.
        result.nodes.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));

        let method = if !similar.is_empty() { "Hybrid (semantic + keyword)" } else { "Keyword" };
        // Use compact JSON for structured results — AI parses it more accurately.
        let mut r = graph.render_query_json(&result, max_chars);
        r.push_str(&format!("\n({} search. Read knowledge.json for full graph.)\n", method));
        r
    }

    /// Node count (for logging).
    #[allow(dead_code)]
    pub fn node_count(&self) -> usize {
        self.graph.lock().map(|g| g.node_count()).unwrap_or(0)
    }
}

// --- Episodic memory ---

/// A conversation episode — a summary of what happened in a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Episode {
    /// When this episode occurred.
    pub date: String,
    /// Who was involved (user identifier or name).
    #[serde(default)]
    pub participants: Vec<String>,
    /// 2-3 sentence summary of what happened.
    pub summary: String,
    /// Key outcomes or decisions.
    #[serde(default)]
    pub outcomes: Vec<String>,
    /// Tags for searchability.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Entities mentioned in this episode (links to knowledge graph nodes).
    /// Enables "what conversations involved this entity?" queries.
    #[serde(default)]
    pub entities: Vec<String>,
}

/// Episodic memory store — append-only log of conversation summaries.
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct EpisodicMemory {
    pub episodes: Vec<Episode>,
}

impl EpisodicMemory {
    /// Load from JSON file, or create empty.
    pub fn load(path: &Path) -> Self {
        if path.exists() {
            if let Ok(contents) = std::fs::read_to_string(path) {
                if let Ok(mem) = serde_json::from_str(&contents) {
                    return mem;
                }
            }
        }
        Self::default()
    }

    /// Save to JSON file (atomic write).
    #[allow(dead_code)]
    pub fn save(&self, path: &Path) {
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let tmp = path.with_extension("json.tmp");
            if std::fs::write(&tmp, &json).is_ok() {
                let _ = std::fs::rename(&tmp, path);
            }
        }
    }

    /// Retrieve recent episodes (last N).
    pub fn recent(&self, n: usize) -> &[Episode] {
        let start = self.episodes.len().saturating_sub(n);
        &self.episodes[start..]
    }

    /// Search episodes by keywords in summary, outcomes, and tags.
    /// Results are weighted by both keyword relevance and recency (exponential decay).
    pub fn search(&self, message: &str, max_results: usize) -> Vec<&Episode> {
        let keywords = extract_keywords(message);
        if keywords.is_empty() {
            return self.recent(max_results).iter().collect();
        }

        let today = today_date();

        let mut scored: Vec<(&Episode, f64)> = self.episodes.iter().map(|ep| {
            let text = format!("{} {} {}",
                ep.summary,
                ep.outcomes.join(" "),
                ep.tags.join(" "));
            let tokens = extract_keywords(&text);
            let keyword_score: f64 = keywords.iter()
                .filter(|kw| tokens.contains(kw))
                .count() as f64;

            // Recency boost: episodes from today score 1.0, decaying with half-life of 30 days.
            let days_ago = days_between(&ep.date, &today).unwrap_or(90) as f64;
            let recency = (-days_ago / 30.0_f64).exp();

            (ep, keyword_score * (1.0 + recency))
        }).filter(|(_, s)| *s > 0.0).collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.into_iter().take(max_results).map(|(ep, _)| ep).collect()
    }

    /// Render episodes as natural language for the prompt.
    pub fn render(&self, episodes: &[&Episode], max_chars: usize) -> String {
        if episodes.is_empty() { return String::new(); }
        let mut output = String::new();
        for ep in episodes {
            output.push_str(&format!("- [{}] {}", ep.date, ep.summary));
            for outcome in &ep.outcomes {
                output.push_str(&format!("\n  → {}", outcome));
            }
            output.push('\n');
            if output.len() > max_chars {
                output.push_str("... (more episodes in episodes.json)\n");
                break;
            }
        }
        output
    }
}

// --- Keyword extraction (Layer 2) ---
// Language-agnostic: uses word length filtering instead of language-specific
// stop words. Works for English, French, German, Māori, etc.

/// Return today's date as "YYYY-MM-DD".
fn today_date() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Simple UTC date calculation (no timezone library needed).
    let days = now / 86400;
    let (y, m, d) = days_to_ymd(days);
    format!("{:04}-{:02}-{:02}", y, m, d)
}

/// Convert days-since-epoch to (year, month, day).
fn days_to_ymd(days: u64) -> (u64, u64, u64) {
    // Algorithm from Howard Hinnant's date library.
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

/// Approximate days between two "YYYY-MM-DD" dates. Returns None on parse failure.
fn days_between(a: &str, b: &str) -> Option<u64> {
    let parse = |s: &str| -> Option<u64> {
        let parts: Vec<&str> = s.split('-').collect();
        if parts.len() != 3 { return None; }
        let y: u64 = parts[0].parse().ok()?;
        let m: u64 = parts[1].parse().ok()?;
        let d: u64 = parts[2].parse().ok()?;
        // Approximate: 365.25 * y + 30.44 * m + d
        Some((365.25 * y as f64 + 30.44 * m as f64 + d as f64) as u64)
    };
    let da = parse(a)?;
    let db = parse(b)?;
    Some(db.saturating_sub(da))
}

/// Extract meaningful keywords from a message.
/// Language-agnostic: filters by length and produces both the original
/// word and common suffix-stripped variants for fuzzy matching.
pub fn extract_keywords(text: &str) -> Vec<String> {
    let mut keywords = Vec::new();
    for word in tokenize(text) {
        // Add the word itself.
        keywords.push(word.clone());
        // Skip suffix variants for acronyms/abbreviations (all-uppercase words
        // like "api", "aws", "sql" — already lowercased by tokenize).
        // Also skip short words where suffix stripping would leave too little.
        let is_acronym = word.len() <= 5 && word.chars().all(|c| c.is_ascii_alphanumeric());
        if is_acronym && word.len() <= 4 {
            continue;
        }
        // Add suffix-stripped variants for fuzzy matching (language-agnostic).
        // This catches plurals, conjugations, etc. across many languages.
        // Use char boundaries (not byte positions) to handle multi-byte UTF-8.
        let char_indices: Vec<usize> = word.char_indices().map(|(i, _)| i).collect();
        let char_count = char_indices.len();
        for trim in [1, 2, 3] {
            if char_count > trim + 3 {
                let end_byte = char_indices[char_count - trim];
                keywords.push(word[..end_byte].to_string());
            }
        }
    }
    keywords.sort();
    keywords.dedup();
    keywords
}

/// Tokenize text into lowercase words.
/// Language-agnostic: splits on non-alphanumeric, filters short words.
fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '-' && c != '_')
        .filter(|w| w.chars().count() > 2) // char count, not byte count (for unicode)
        .filter(|w| !is_function_word(w))
        .map(|w| w.to_string())
        .collect()
}

/// Filter common function words across multiple languages.
/// Short list: only the most universal, high-frequency words.
fn is_function_word(w: &str) -> bool {
    matches!(
        w,
        // English
        "the" | "and" | "for" | "are" | "was" | "not" | "but" | "you" | "has" | "had"
        | "his" | "her" | "its" | "our" | "can" | "did" | "all" | "will" | "been"
        // French
        | "les" | "des" | "une" | "est" | "pas" | "que" | "qui" | "dans" | "pour" | "avec"
        // German
        | "die" | "der" | "das" | "und" | "ein" | "ist" | "den" | "von" | "mit" | "auf"
        // Spanish
        | "los" | "las" | "del" | "por" | "con" | "una"
        // Common across romance languages
        | "non"
    )
}

/// Visual confidence bar (language-agnostic).
/// ●●●●○ for 80%, ●●○○○ for 40%, etc.
#[allow(dead_code)]
fn confidence_bar(confidence: f64) -> String {
    let filled = (confidence * 5.0).round() as usize;
    let empty = 5 - filled.min(5);
    format!("{}{}", "●".repeat(filled.min(5)), "○".repeat(empty))
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().to_string() + c.as_str(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_graph_loads_and_saves() {
        let dir = std::env::temp_dir().join("anthill-test-kg-empty");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("knowledge.json");

        let kg = KnowledgeGraph::load(&path);
        assert_eq!(kg.node_count(), 0);
        kg.save();
        assert!(path.exists());

        // Reload.
        let kg2 = KnowledgeGraph::load(&path);
        assert_eq!(kg2.node_count(), 0);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn add_nodes_and_edges_roundtrip() {
        let dir = std::env::temp_dir().join("anthill-test-kg-rt");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("knowledge.json");

        let mut kg = KnowledgeGraph::load(&path);

        let roy = kg.graph.add_node(KnowledgeNode {
            label: "Roy".into(),
            kind: NodeKind::Person,
            summary: "Project lead".into(),
            created: "2026-03-20".into(),
            updated: "2026-03-20".into(),
            tags: vec!["architect".into()],
            ..Default::default()
        });
        let anthill = kg.graph.add_node(KnowledgeNode {
            label: "Anthill".into(),
            kind: NodeKind::Project,
            summary: "AI colony platform".into(),
            created: "2026-03-10".into(),
            updated: "2026-03-20".into(),
            tags: vec!["rust".into(), "ai".into()],
            ..Default::default()
        });
        kg.graph.add_edge(roy, anthill, KnowledgeEdge::new(
            "works_on", "Lead developer", "2026-03-10", Basis::Observed,
        ));
        kg.rebuild_index();
        kg.save();

        // Reload.
        let kg2 = KnowledgeGraph::load(&path);
        assert_eq!(kg2.node_count(), 2);
        assert!(kg2.find_by_label("Roy").is_some());
        assert!(kg2.find_by_label("Anthill").is_some());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn keyword_extraction() {
        let kw = extract_keywords("What is the Anthill project architecture?");
        assert!(kw.contains(&"anthill".to_string()));
        assert!(kw.contains(&"project".to_string()));
        assert!(kw.contains(&"architecture".to_string()));
        assert!(!kw.contains(&"the".to_string())); // function word
    }

    #[test]
    fn relevant_subgraph_extraction() {
        let dir = std::env::temp_dir().join("anthill-test-kg-subgraph");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("knowledge.json");

        let mut kg = KnowledgeGraph::load(&path);

        let roy = kg.graph.add_node(KnowledgeNode {
            label: "Roy".into(),
            kind: NodeKind::Person,
            summary: "Architect".into(),
            created: "2026-03-20".into(),
            updated: "2026-03-20".into(),
            tags: vec![],
            ..Default::default()
        });
        let anthill = kg.graph.add_node(KnowledgeNode {
            label: "Anthill".into(),
            kind: NodeKind::Project,
            summary: "AI colony".into(),
            created: "2026-03-10".into(),
            updated: "2026-03-20".into(),
            tags: vec!["rust".into()],
            ..Default::default()
        });
        let _unrelated = kg.graph.add_node(KnowledgeNode {
            label: "Weather".into(),
            kind: NodeKind::Fact,
            summary: "It rains in Auckland".into(),
            created: "2026-03-20".into(),
            updated: "2026-03-20".into(),
            tags: vec![],
            ..Default::default()
        });
        kg.graph.add_edge(roy, anthill, KnowledgeEdge::new(
            "works_on", "", "", Basis::Observed,
        ));
        kg.rebuild_index();

        // Asking about Anthill should pull Roy (neighbor) but not Weather.
        let relevant = kg.relevant_subgraph("Tell me about Anthill", 50);
        let labels: Vec<String> = relevant
            .iter()
            .map(|&idx| kg.graph[idx].label.clone())
            .collect();
        assert!(labels.contains(&"Anthill".to_string()));
        assert!(labels.contains(&"Roy".to_string())); // 1-hop neighbor
        assert!(!labels.contains(&"Weather".to_string()));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn render_produces_readable_output() {
        let dir = std::env::temp_dir().join("anthill-test-kg-render");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("knowledge.json");

        let mut kg = KnowledgeGraph::load(&path);
        let roy = kg.graph.add_node(KnowledgeNode {
            label: "Roy".into(),
            kind: NodeKind::Person,
            summary: "Project lead".into(),
            created: "2026-03-20".into(),
            updated: "2026-03-20".into(),
            tags: vec![],
            ..Default::default()
        });
        let anthill = kg.graph.add_node(KnowledgeNode {
            label: "Anthill".into(),
            kind: NodeKind::Project,
            summary: "AI colony platform".into(),
            created: "2026-03-10".into(),
            updated: "2026-03-20".into(),
            tags: vec![],
            ..Default::default()
        });
        kg.graph.add_edge(roy, anthill, KnowledgeEdge::new(
            "works_on", "", "", Basis::Observed,
        ));
        kg.rebuild_index();

        let rendered = kg.render_full(4096);
        assert!(rendered.contains("## Person"));
        assert!(rendered.contains("Roy (person): Project lead"));
        assert!(rendered.contains("→ works_on → Anthill"));
        assert!(rendered.contains("## Project"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn popperian_confidence_dynamics() {
        let mut edge = KnowledgeEdge::new("works_on", "Dev lead", "2026-03-10", Basis::Told);
        assert!((edge.confidence - 0.6).abs() < 0.01); // told = 0.6

        // Survive 5 tests — confidence should increase.
        for _ in 0..5 {
            edge.strengthen("2026-03-20");
        }
        assert!(edge.confidence > 0.6, "After 5 strengthen: {}", edge.confidence);
        assert_eq!(edge.tests, 5);
        assert_eq!(edge.survived, 5);

        // Fail 2 tests — confidence should decrease.
        let before_weaken = edge.confidence;
        edge.weaken("2026-03-20");
        edge.weaken("2026-03-20");
        assert!(edge.confidence < before_weaken, "Weaken should decrease confidence");
        assert_eq!(edge.tests, 7);
        assert_eq!(edge.survived, 5);

        // Direct contradiction — significant drop (Bayesian with BF≈0.18).
        let before = edge.confidence;
        edge.contradict("2026-03-20");
        assert!(edge.confidence < before, "Contradict should decrease confidence: {} < {}", edge.confidence, before);

        // Time decay.
        let mut fresh = KnowledgeEdge::new("uses", "Rust", "2026-03-01", Basis::Observed);
        let initial = fresh.confidence;
        fresh.decay(90); // 3 months untested
        assert!(fresh.confidence < initial);

        // Basis initial confidence ordering.
        assert!(Basis::Observed.initial_confidence() > Basis::Told.initial_confidence());
        assert!(Basis::Told.initial_confidence() > Basis::Inferred.initial_confidence());
        assert!(Basis::Inferred.initial_confidence() > Basis::Assumed.initial_confidence());
    }

    #[test]
    fn confidence_labels() {
        let mut edge = KnowledgeEdge::new("test", "", "", Basis::Observed);
        edge.confidence = 0.9;
        assert_eq!(edge.confidence_label(), "established");
        edge.confidence = 0.65;
        assert_eq!(edge.confidence_label(), "likely");
        edge.confidence = 0.45;
        assert_eq!(edge.confidence_label(), "possible");
        edge.confidence = 0.25;
        assert_eq!(edge.confidence_label(), "uncertain");
        edge.confidence = 0.1;
        assert_eq!(edge.confidence_label(), "doubtful");
    }

    #[test]
    fn low_confidence_edges_hidden_in_render() {
        let dir = std::env::temp_dir().join("anthill-test-kg-lowconf");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("knowledge.json");

        let mut kg = KnowledgeGraph::load(&path);
        let a = kg.graph.add_node(KnowledgeNode {
            label: "A".into(), kind: NodeKind::Fact, summary: "Node A".into(),
            created: String::new(), updated: String::new(), tags: vec![],
            ..Default::default()
        });
        let b = kg.graph.add_node(KnowledgeNode {
            label: "B".into(), kind: NodeKind::Fact, summary: "Node B".into(),
            created: String::new(), updated: String::new(), tags: vec![],
            ..Default::default()
        });

        // High confidence edge — should render.
        let mut strong = KnowledgeEdge::new("strong_link", "", "", Basis::Observed);
        strong.confidence = 0.9;
        kg.graph.add_edge(a, b, strong);

        // Very low confidence edge — should be hidden.
        let mut weak = KnowledgeEdge::new("weak_link", "", "", Basis::Assumed);
        weak.confidence = 0.05;
        kg.graph.add_edge(a, b, weak);

        kg.rebuild_index();
        let rendered = kg.render_full(4096);
        assert!(rendered.contains("strong_link"));
        assert!(!rendered.contains("weak_link"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn fuzzy_matching_via_suffix_variants() {
        // "projects" should generate "project" (suffix strip) for matching.
        let kw = extract_keywords("Tell me about the Anthill projects");
        assert!(kw.contains(&"anthill".to_string()));
        assert!(kw.contains(&"project".to_string())); // "projects" minus 1 char
    }

    #[test]
    fn multilingual_keywords() {
        // French: function words filtered, content words kept.
        let kw = extract_keywords("Quel est le projet Anthill?");
        assert!(kw.contains(&"projet".to_string()));
        assert!(kw.contains(&"anthill".to_string()));
        assert!(!kw.contains(&"est".to_string())); // French function word

        // German.
        let kw = extract_keywords("Was ist das Anthill Projekt?");
        assert!(kw.contains(&"anthill".to_string()));
        assert!(kw.contains(&"projekt".to_string()));
        assert!(!kw.contains(&"das".to_string())); // German function word
    }

    #[test]
    fn confidence_bar_rendering() {
        assert_eq!(confidence_bar(1.0), "●●●●●");
        assert_eq!(confidence_bar(0.6), "●●●○○");
        assert_eq!(confidence_bar(0.0), "○○○○○");
    }

    #[test]
    fn node_deduplication() {
        let dir = std::env::temp_dir().join("anthill-test-kg-dedup");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("knowledge.json");

        let mut kg = KnowledgeGraph::load(&path);
        // Two nodes with matching labels (case-insensitive).
        kg.graph.add_node(KnowledgeNode {
            label: "Anthill".into(), kind: NodeKind::Project, summary: "Short".into(),
            created: String::new(), updated: String::new(), tags: vec![],
            ..Default::default()
        });
        kg.graph.add_node(KnowledgeNode {
            label: "anthill".into(), kind: NodeKind::Project, summary: "AI colony platform, detailed".into(),
            created: String::new(), updated: String::new(), tags: vec!["rust".into()],
            ..Default::default()
        });
        assert_eq!(kg.node_count(), 2);

        let report = kg.consolidate();
        assert_eq!(report.nodes_merged, 1);
        assert_eq!(kg.node_count(), 1);
        // Should keep the longer summary.
        let remaining = kg.graph.node_indices().next().unwrap();
        assert!(kg.graph[remaining].summary.contains("detailed"));
        // Tags merged.
        assert!(kg.graph[remaining].tags.contains(&"rust".to_string()));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn parallel_edge_merging() {
        let dir = std::env::temp_dir().join("anthill-test-kg-parallel");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("knowledge.json");

        let mut kg = KnowledgeGraph::load(&path);
        let a = kg.graph.add_node(KnowledgeNode {
            label: "A".into(), kind: NodeKind::Person, summary: "".into(),
            created: String::new(), updated: String::new(), tags: vec![],
            ..Default::default()
        });
        let b = kg.graph.add_node(KnowledgeNode {
            label: "B".into(), kind: NodeKind::Project, summary: "".into(),
            created: String::new(), updated: String::new(), tags: vec![],
            ..Default::default()
        });
        // Two edges with same relation.
        let mut e1 = KnowledgeEdge::new("works_on", "context 1", "", Basis::Told);
        e1.confidence = 0.5;
        e1.tests = 3;
        e1.survived = 2;
        let mut e2 = KnowledgeEdge::new("works_on", "context 2, more detailed", "", Basis::Observed);
        e2.confidence = 0.6;
        e2.tests = 5;
        e2.survived = 4;
        kg.graph.add_edge(a, b, e1);
        kg.graph.add_edge(a, b, e2);

        let report = kg.consolidate();
        assert_eq!(report.edges_merged, 1);

        // Remaining edge should have evidence-weighted average confidence.
        let remaining = kg.graph.edges(a).next().unwrap();
        let edge = remaining.weight();
        // With log-odds weighted average, result is between the two inputs.
        assert!(edge.confidence > 0.4 && edge.confidence < 0.7,
            "Expected merged confidence between inputs, got {}", edge.confidence);
        assert_eq!(edge.tests, 8); // 3 + 5
        assert_eq!(edge.survived, 6); // 2 + 4
        // Context should combine both sources.
        assert!(edge.context.contains("context 1"));
        assert!(edge.context.contains("context 2"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn chain_collapsing() {
        let dir = std::env::temp_dir().join("anthill-test-kg-chain");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("knowledge.json");

        let mut kg = KnowledgeGraph::load(&path);
        let a = kg.graph.add_node(KnowledgeNode {
            label: "Roy".into(), kind: NodeKind::Person, summary: "".into(),
            created: String::new(), updated: String::new(), tags: vec![],
            ..Default::default()
        });
        let mid = kg.graph.add_node(KnowledgeNode {
            label: "uses Rust".into(), kind: NodeKind::Fact, summary: "intermediate".into(),
            created: String::new(), updated: String::new(), tags: vec![],
            ..Default::default()
        });
        let c = kg.graph.add_node(KnowledgeNode {
            label: "Anthill".into(), kind: NodeKind::Project, summary: "".into(),
            created: String::new(), updated: String::new(), tags: vec![],
            ..Default::default()
        });
        let mut e1 = KnowledgeEdge::new("knows", "", "", Basis::Observed);
        e1.confidence = 0.8;
        let mut e2 = KnowledgeEdge::new("applied_to", "", "", Basis::Inferred);
        e2.confidence = 0.6;
        kg.graph.add_edge(a, mid, e1);
        kg.graph.add_edge(mid, c, e2);

        assert_eq!(kg.node_count(), 3);
        let report = kg.consolidate();
        assert_eq!(report.chains_collapsed, 1);
        assert_eq!(kg.node_count(), 2); // mid removed

        // Combined edge: confidence = min(0.8, 0.6) = 0.6.
        let combined = kg.graph.edges(a).next().unwrap();
        assert!((combined.weight().confidence - 0.6).abs() < 0.01);
        assert!(combined.weight().relation.contains("knows"));
        assert!(combined.weight().relation.contains("applied_to"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn contradiction_detection() {
        let dir = std::env::temp_dir().join("anthill-test-kg-contra");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("knowledge.json");

        let mut kg = KnowledgeGraph::load(&path);
        let a = kg.graph.add_node(KnowledgeNode {
            label: "Team".into(), kind: NodeKind::Concept, summary: "".into(),
            created: String::new(), updated: String::new(), tags: vec![],
            ..Default::default()
        });
        let b = kg.graph.add_node(KnowledgeNode {
            label: "Python".into(), kind: NodeKind::Tool, summary: "".into(),
            created: String::new(), updated: String::new(), tags: vec![],
            ..Default::default()
        });
        let mut e1 = KnowledgeEdge::new("uses", "main language", "", Basis::Told);
        e1.confidence = 0.85;
        let mut e2 = KnowledgeEdge::new("avoids", "too slow", "", Basis::Inferred);
        e2.confidence = 0.2;
        kg.graph.add_edge(a, b, e1);
        kg.graph.add_edge(a, b, e2);

        let report = kg.consolidate();
        assert!(!report.contradictions.is_empty());
        assert!(report.contradictions[0].contains("contradiction"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn labels_match_cases() {
        assert!(labels_match("Anthill", "anthill"));
        assert!(labels_match("Anthill", "Anthill project"));
        assert!(labels_match("Roy", "roy"));
        assert!(!labels_match("Anthill", "Beehive"));
        assert!(labels_match("AI", "ai")); // exact case-insensitive
    }

    fn build_test_graph(path: &Path) -> KnowledgeGraph {
        let mut kg = KnowledgeGraph::load(path);
        let roy = kg.graph.add_node(KnowledgeNode {
            label: "Roy".into(), kind: NodeKind::Person, summary: "Architect".into(),
            created: "2026-03-20".into(), updated: "2026-03-20".into(), tags: vec![],
            ..Default::default()
        });
        let anthill = kg.graph.add_node(KnowledgeNode {
            label: "Anthill".into(), kind: NodeKind::Project, summary: "AI colony".into(),
            created: "2026-03-10".into(), updated: "2026-03-20".into(), tags: vec!["rust".into()],
            ..Default::default()
        });
        let alfred = kg.graph.add_node(KnowledgeNode {
            label: "Alfred".into(), kind: NodeKind::Server, summary: "Production server".into(),
            created: "2026-03-15".into(), updated: "2026-03-20".into(), tags: vec!["linux".into()],
            ..Default::default()
        });
        let rust = kg.graph.add_node(KnowledgeNode {
            label: "Rust".into(), kind: NodeKind::Tool, summary: "Programming language".into(),
            created: "2026-03-10".into(), updated: "2026-03-20".into(), tags: vec![],
            ..Default::default()
        });
        let mut e1 = KnowledgeEdge::new("works_on", "Lead dev", "2026-03-10", Basis::Observed);
        e1.confidence = 0.85;
        e1.tests = 10;
        e1.survived = 9;
        kg.graph.add_edge(roy, anthill, e1);
        let mut e2 = KnowledgeEdge::new("deployed_on", "", "2026-03-15", Basis::Told);
        e2.confidence = 0.72;
        kg.graph.add_edge(anthill, alfred, e2);
        let mut e3 = KnowledgeEdge::new("written_in", "", "2026-03-10", Basis::Observed);
        e3.confidence = 0.9;
        kg.graph.add_edge(anthill, rust, e3);
        kg.rebuild_index();
        kg
    }

    #[test]
    fn query_about_traverses_from_node() {
        let dir = std::env::temp_dir().join("anthill-test-kg-qabout");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let kg = build_test_graph(&dir.join("knowledge.json"));

        // Query about Roy — should find Roy + his connections.
        let result = kg.query_about("Roy", 2);
        let labels: Vec<&str> = result.nodes.iter().map(|(_, n, _)| n.label.as_str()).collect();
        assert!(labels.contains(&"Roy"));
        assert!(labels.contains(&"Anthill")); // 1 hop
        assert!(labels.contains(&"Alfred")); // 2 hops
        assert!(!result.edges.is_empty());

        // Root node should have score 1.0.
        let roy_score = result.nodes.iter().find(|(_, n, _)| n.label == "Roy").unwrap().2;
        assert!((roy_score - 1.0).abs() < 0.01);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn query_about_fuzzy_match() {
        let dir = std::env::temp_dir().join("anthill-test-kg-qfuzzy");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let kg = build_test_graph(&dir.join("knowledge.json"));

        // "ant" should fuzzy-match "Anthill".
        let result = kg.query_about("ant", 1);
        assert!(!result.nodes.is_empty());
        let labels: Vec<&str> = result.nodes.iter().map(|(_, n, _)| n.label.as_str()).collect();
        assert!(labels.contains(&"Anthill"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn query_path_finds_connections() {
        let dir = std::env::temp_dir().join("anthill-test-kg-qpath");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let kg = build_test_graph(&dir.join("knowledge.json"));

        // Path from Roy to Alfred: Roy → Anthill → Alfred.
        let result = kg.query_path("Roy", "Alfred", 3);
        assert!(!result.paths.is_empty());
        let path = &result.paths[0];
        assert_eq!(path.nodes.len(), 3); // Roy, Anthill, Alfred
        // Cumulative confidence: 0.85 * 0.72 = 0.612.
        assert!((path.cumulative_confidence - 0.612).abs() < 0.01);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn query_path_no_connection() {
        let dir = std::env::temp_dir().join("anthill-test-kg-qnopath");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut kg = build_test_graph(&dir.join("knowledge.json"));
        // Add an isolated node.
        kg.graph.add_node(KnowledgeNode {
            label: "Unrelated".into(), kind: NodeKind::Fact, summary: "No connections".into(),
            created: String::new(), updated: String::new(), tags: vec![],
            ..Default::default()
        });
        kg.rebuild_index();

        let result = kg.query_path("Roy", "Unrelated", 3);
        assert!(result.paths.is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn query_by_kind_filters() {
        let dir = std::env::temp_dir().join("anthill-test-kg-qkind");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let kg = build_test_graph(&dir.join("knowledge.json"));

        let result = kg.query_by_kind(&NodeKind::Server);
        assert_eq!(result.nodes.len(), 1);
        assert_eq!(result.nodes[0].1.label, "Alfred");

        let result = kg.query_by_kind(&NodeKind::Decision);
        assert_eq!(result.nodes.len(), 0);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn query_uncertain_finds_weak_edges() {
        let dir = std::env::temp_dir().join("anthill-test-kg-quncertain");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut kg = build_test_graph(&dir.join("knowledge.json"));

        // Add a weak edge.
        let roy = kg.find_by_label("Roy").unwrap();
        let alfred = kg.find_by_label("Alfred").unwrap();
        let mut weak = KnowledgeEdge::new("may_admin", "uncertain", "", Basis::Assumed);
        weak.confidence = 0.3;
        kg.graph.add_edge(roy, alfred, weak);

        let result = kg.query_uncertain(0.5);
        assert!(!result.edges.is_empty());
        // Should include the weak edge but not the strong ones.
        let weak_edges: Vec<&WeightedEdge> = result.edges.iter()
            .filter(|e| e.edge.relation == "may_admin")
            .collect();
        assert_eq!(weak_edges.len(), 1);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn render_query_result_includes_confidence() {
        let dir = std::env::temp_dir().join("anthill-test-kg-qrender");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let kg = build_test_graph(&dir.join("knowledge.json"));

        let result = kg.query_about("Roy", 1);
        let rendered = kg.render_query_result(&result, 4096);
        // Should contain graduated trust labels and confidence percentages.
        assert!(rendered.contains("ESTABLISHED") || rendered.contains("LIKELY")
            || rendered.contains("POSSIBLE") || rendered.contains("UNCERTAIN")
            || rendered.contains("DOUBTFUL"), "rendered should contain a trust label: {}", rendered);
        assert!(rendered.contains("Roy"));
        assert!(rendered.contains("works_on"));
        assert!(rendered.contains("%"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn levenshtein_distance() {
        assert_eq!(super::levenshtein("kitten", "sitting"), 3);
        assert_eq!(super::levenshtein("", "abc"), 3);
        assert_eq!(super::levenshtein("abc", "abc"), 0);
        assert_eq!(super::levenshtein("abc", "abd"), 1);
    }

    #[test]
    fn labels_match_fuzzy() {
        // Exact (case-insensitive)
        assert!(super::labels_match("Anthill", "anthill"));
        // Substring
        assert!(super::labels_match("Anthill", "Anthill project"));
        assert!(super::labels_match("Redis", "Redistribution")); // substring match
        // Levenshtein: "anthill" vs "anthil_" (1 edit, ~14% of 7 chars)
        assert!(super::labels_match("anthill", "anthil_"));
        // Too different for Levenshtein
        assert!(!super::labels_match("anthill", "beehive"));
        // Short labels skip fuzzy (< 6 chars)
        assert!(!super::labels_match("abc", "abd"));
    }

    #[test]
    fn days_between_dates() {
        // Same date
        assert_eq!(super::days_between("2026-03-20", "2026-03-20"), Some(0));
        // One day apart (approximate)
        let d = super::days_between("2026-03-19", "2026-03-20").unwrap();
        assert!(d <= 2, "Expected ~1, got {}", d);
        // Invalid
        assert!(super::days_between("bad", "date").is_none());
    }

    #[test]
    fn corruption_recovery_loads_archive() {
        let dir = std::env::temp_dir().join("anthill-test-kg-corrupt");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // Create a valid archive with one node.
        let archive_path = dir.join("knowledge-archive.json");
        let mut archive = KnowledgeGraph::load(&archive_path);
        archive.graph.add_node(KnowledgeNode {
            label: "Recovered".into(), kind: NodeKind::Fact,
            summary: "From archive".into(),
            created: String::new(), updated: String::new(), tags: vec![],
            ..Default::default()
        });
        archive.save();

        // Write corrupted main file.
        let main_path = dir.join("knowledge.json");
        std::fs::write(&main_path, "{ invalid json !!!").unwrap();

        // Load should recover from archive.
        let kg = KnowledgeGraph::load(&main_path);
        assert_eq!(kg.node_count(), 1);
        assert!(kg.find_by_label("Recovered").is_some());

        // Corrupted file should be preserved.
        assert!(dir.join("knowledge.json.corrupted").exists());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn edge_merge_uses_max_confidence() {
        let dir = std::env::temp_dir().join("anthill-test-kg-merge-max");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("knowledge.json");

        let mut kg = KnowledgeGraph::load(&path);
        let a = kg.graph.add_node(KnowledgeNode {
            label: "A".into(), kind: NodeKind::Concept, summary: "".into(),
            created: String::new(), updated: String::new(), tags: vec![],
            ..Default::default()
        });
        let b = kg.graph.add_node(KnowledgeNode {
            label: "B".into(), kind: NodeKind::Concept, summary: "".into(),
            created: String::new(), updated: String::new(), tags: vec![],
            ..Default::default()
        });

        let mut e1 = KnowledgeEdge::new("uses", "ctx1", "2026-01-01", Basis::Observed);
        e1.confidence = 0.5;
        let mut e2 = KnowledgeEdge::new("uses", "ctx2", "2026-01-01", Basis::Told);
        e2.confidence = 0.8;

        kg.graph.add_edge(a, b, e1);
        kg.graph.add_edge(a, b, e2);

        let merged = kg.merge_parallel_edges();
        assert_eq!(merged, 1);

        // Merged edge uses evidence-weighted log-odds average.
        let edge = kg.graph.edges(a).next().unwrap();
        assert!(edge.weight().confidence > 0.5 && edge.weight().confidence < 0.85,
            "Expected merged confidence between inputs, got {}", edge.weight().confidence);
        // Context should combine both.
        assert!(edge.weight().context.contains("ctx1"));
        assert!(edge.weight().context.contains("ctx2"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn relevant_subgraph_filters_low_confidence() {
        let dir = std::env::temp_dir().join("anthill-test-kg-subgraph-conf");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("knowledge.json");

        let mut kg = KnowledgeGraph::load(&path);
        let a = kg.graph.add_node(KnowledgeNode {
            label: "Rust".into(), kind: NodeKind::Concept, summary: "programming language".into(),
            created: String::new(), updated: String::new(), tags: vec!["rust".into()],
            ..Default::default()
        });
        let b = kg.graph.add_node(KnowledgeNode {
            label: "HighConf".into(), kind: NodeKind::Fact, summary: "well-known".into(),
            created: String::new(), updated: String::new(), tags: vec![],
            ..Default::default()
        });
        let c = kg.graph.add_node(KnowledgeNode {
            label: "LowConf".into(), kind: NodeKind::Fact, summary: "uncertain".into(),
            created: String::new(), updated: String::new(), tags: vec![],
            ..Default::default()
        });

        let mut high = KnowledgeEdge::new("uses", "", "2026-01-01", Basis::Observed);
        high.confidence = 0.9;
        let mut low = KnowledgeEdge::new("maybe", "", "2026-01-01", Basis::Assumed);
        low.confidence = 0.1; // Below 0.3 threshold

        kg.graph.add_edge(a, b, high);
        kg.graph.add_edge(a, c, low);
        kg.rebuild_index();

        let result = kg.relevant_subgraph("rust programming", 50);
        // Should include Rust and HighConf, but NOT LowConf (low confidence edge).
        let labels: Vec<&str> = result.iter()
            .map(|&idx| kg.graph[idx].label.as_str())
            .collect();
        assert!(labels.contains(&"Rust"));
        assert!(labels.contains(&"HighConf"));
        assert!(!labels.contains(&"LowConf"), "Low-confidence neighbor should be filtered out");

        std::fs::remove_dir_all(&dir).ok();
    }
}
