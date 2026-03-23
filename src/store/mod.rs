//! Knowledge store — validated, trait-based access to the knowledge graph.
//!
//! All graph access goes through the `KnowledgeStore` trait. The AI cannot
//! edit graph files directly — all mutations are validated at the API boundary.
//!
//! Architecture:
//!   Consumers (MCP, Web, Maintenance) → KnowledgeStore trait
//!     → GraphEngine (petgraph, queries, consolidation)
//!     → StorageBackend (JSON files, later CBOR/git)

pub mod validated;
pub mod engine;
pub mod json_backend;
pub mod cbor_backend;
pub mod changelog;
pub mod live;
pub mod migration;

use std::fmt;

// Re-export key types for consumers.
pub use validated::{ValidatedNode, ValidatedEdge, ValidatedEvidence};
#[allow(unused_imports)]
pub use live::LiveKnowledgeStore;

// ── Error types ────────────────────────────────────────────────────

/// Result type for store operations.
pub type StoreResult<T> = Result<T, StoreError>;

/// Errors from the knowledge store.
#[derive(Debug)]
pub enum StoreError {
    /// A field value failed validation.
    Validation(String),
    /// The requested entity was not found.
    NotFound(String),
    /// A duplicate entity already exists.
    Duplicate(String),
    /// I/O or serialization error.
    Storage(String),
    /// Git operation failed.
    #[allow(dead_code)]
    Git(String),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Validation(msg) => write!(f, "Validation error: {}", msg),
            Self::NotFound(msg) => write!(f, "Not found: {}", msg),
            Self::Duplicate(msg) => write!(f, "Duplicate: {}", msg),
            Self::Storage(msg) => write!(f, "Storage error: {}", msg),
            Self::Git(msg) => write!(f, "Git error: {}", msg),
        }
    }
}

impl std::error::Error for StoreError {}

// ── ID types ───────────────────────────────────────────────────────

/// Opaque node identifier (wraps petgraph NodeIndex).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId(pub(crate) petgraph::stable_graph::NodeIndex);

/// Opaque edge identifier (wraps petgraph EdgeIndex).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EdgeId(pub(crate) petgraph::graph::EdgeIndex);

// ── Info types ─────────────────────────────────────────────────────

/// Summary info about a graph.
#[derive(Debug, Clone, serde::Serialize)]
pub struct GraphInfo {
    pub name: String,
    pub node_count: usize,
    pub edge_count: usize,
}

/// Detailed stats about a graph.
#[derive(Debug, Clone, serde::Serialize)]
#[allow(dead_code)]
pub struct GraphStats {
    pub name: String,
    pub node_count: usize,
    pub edge_count: usize,
    pub avg_confidence: f64,
    pub uncertain_edges: usize,
    pub orphan_nodes: usize,
}

/// Info about a git commit.
#[derive(Debug, Clone, serde::Serialize)]
#[allow(dead_code)]
pub struct CommitInfo {
    pub hash: String,
    pub message: String,
    pub timestamp: String,
}

/// Result of an edge update (evidence, strengthen, weaken, etc.).
#[derive(Debug, Clone, serde::Serialize)]
pub struct EdgeUpdate {
    pub confidence_before: f64,
    pub confidence_after: f64,
    pub log_odds_before: f64,
    pub log_odds_after: f64,
    pub evidence_type: String,
    pub bayes_factor: f64,
    /// Warning message if the update pattern looks like confirmation bias.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confirmation_bias_warning: Option<String>,
}

// ── Consolidation report ───────────────────────────────────────────

/// Report from a consolidation pass.
#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct ConsolidationReport {
    pub nodes_merged: usize,
    pub edges_merged: usize,
    pub chains_collapsed: usize,
    pub contradictions: Vec<String>,
    pub clusters: Vec<Vec<String>>,
}

// ── KnowledgeStore trait ───────────────────────────────────────────

/// The primary interface to the knowledge system.
/// All consumers (MCP, web, maintenance, AI worker) go through this.
///
/// Writes are validated — invalid data is rejected with StoreError::Validation.
/// The AI interacts through MCP tools that call these methods.
/// Direct file editing is not supported.
#[allow(dead_code)]
pub trait KnowledgeStore: Send + Sync {
    // ── Graph management ──

    /// List all available graphs (meta + topic graphs).
    fn list_graphs(&self) -> StoreResult<Vec<GraphInfo>>;

    /// Get detailed stats for a graph.
    fn graph_stats(&self, graph: &str) -> StoreResult<GraphStats>;

    // ── Node operations (validated) ──

    /// Add a node to a graph. Returns the node ID.
    fn add_node(&self, graph: &str, node: ValidatedNode) -> StoreResult<NodeId>;

    /// Get a node by label.
    fn get_node(&self, graph: &str, label: &str) -> StoreResult<crate::knowledge::KnowledgeNode>;

    /// List all node labels in a graph.
    fn list_nodes(&self, graph: &str) -> StoreResult<Vec<String>>;

    // ── Edge operations (validated) ──

    /// Add an edge to a graph. Returns the edge ID.
    fn add_edge(&self, graph: &str, edge: ValidatedEdge) -> StoreResult<EdgeId>;

    /// Update an edge with typed evidence (primary Thurisaz update path).
    fn update_evidence(
        &self, graph: &str, from: &str, to: &str, relation: &str,
        evidence: ValidatedEvidence,
    ) -> StoreResult<EdgeUpdate>;

    /// Strengthen an edge (refutation survived).
    fn strengthen(
        &self, graph: &str, from: &str, to: &str, relation: &str,
        test: &str, evidence: &str,
    ) -> StoreResult<EdgeUpdate>;

    /// Weaken an edge (inconsistency found).
    fn weaken(
        &self, graph: &str, from: &str, to: &str, relation: &str,
        test: &str, evidence: &str,
    ) -> StoreResult<EdgeUpdate>;

    /// Contradict an edge (refutation failed — sharp penalty).
    fn contradict(
        &self, graph: &str, from: &str, to: &str, relation: &str,
        test: &str, evidence: &str,
    ) -> StoreResult<EdgeUpdate>;

    // ── Queries (read-only) ──

    /// Query the subgraph around an entity.
    fn query_about(&self, graph: &str, entity: &str, depth: usize)
        -> StoreResult<crate::knowledge::QueryResult>;

    /// Find paths between two entities.
    fn query_path(&self, graph: &str, from: &str, to: &str, max_paths: usize)
        -> StoreResult<crate::knowledge::QueryResult>;

    /// Query by node kind.
    fn query_by_kind(&self, graph: &str, kind: &str)
        -> StoreResult<crate::knowledge::QueryResult>;

    /// Query edges below a confidence threshold.
    fn query_uncertain(&self, graph: &str, threshold: f64)
        -> StoreResult<crate::knowledge::QueryResult>;

    /// Get the justification chain for an edge.
    fn query_justification(&self, graph: &str, from: &str, to: &str, relation: &str)
        -> StoreResult<String>;

    /// Add a citation to an existing edge.
    fn add_citation(
        &self, graph: &str, from: &str, to: &str, relation: &str,
        citation: crate::knowledge::Reference,
    ) -> StoreResult<()>;

    /// Extract misplaced nodes from the meta-graph (non-topic nodes) and relocate
    /// them to a holding topic graph. Returns number of nodes relocated.
    fn extract_misplaced_meta_nodes(&self) -> StoreResult<usize>;

    /// List orphan nodes (nodes with only '?' connections).
    fn list_orphans(&self, graph: &str) -> StoreResult<Vec<String>>;

    // ── Rendering ──

    /// Render relevant knowledge for the AI system prompt.
    fn render_for_prompt(&self, message: &str, max_chars: usize) -> String;

    /// Render the graph as JSON for 3D visualization.
    fn to_visualization(&self, graph: &str) -> StoreResult<serde_json::Value>;

    // ── Maintenance ──

    /// Run consolidation: dedup nodes, merge parallel edges, collapse chains.
    fn consolidate(&self, graph: &str) -> StoreResult<ConsolidationReport>;

    /// Apply time-based decay to all edges.
    fn apply_decay(&self, graph: &str, days: u32) -> StoreResult<u32>;

    /// Compute corroboration strength for all edges.
    fn compute_corroboration_strength(&self, graph: &str) -> StoreResult<()>;

    /// Link orphan nodes to a hub.
    fn link_orphans(&self, graph: &str) -> StoreResult<u32>;

    /// Backfill Thurisaz format on legacy edges.
    fn backfill_thurisaz(&self, graph: &str) -> StoreResult<u32>;

    // ── Rumination support ──

    /// Find edges suitable for refutation (important but uncertain).
    fn refutation_candidates(&self, graph: &str, limit: usize)
        -> StoreResult<Vec<(String, String, String, f64, f64)>>;

    /// Find synthesis candidates (A→B→C where no A→C exists).
    fn synthesis_candidates(&self, graph: &str, limit: usize)
        -> StoreResult<Vec<(NodeId, NodeId, String, String, String)>>;

    /// Find undetermined connections ('?' edges).
    fn undetermined_connections(&self, graph: &str, limit: usize)
        -> StoreResult<Vec<(String, String)>>;

    /// Find competing hypotheses.
    fn find_competitors(&self, graph: &str)
        -> StoreResult<Vec<crate::knowledge::CompetitorGroup>>;

    /// Find contradiction pairs.
    fn contradiction_pairs(&self, graph: &str)
        -> StoreResult<Vec<crate::knowledge::ContradictionPair>>;

    /// Get uncertainty stats.
    fn uncertainty_stats(&self, graph: &str)
        -> StoreResult<crate::knowledge::UncertaintyStats>;

    /// Find cross-domain patterns between two graphs.
    fn cross_domain_patterns(&self, graph_a: &str, graph_b: &str, limit: usize)
        -> StoreResult<Vec<crate::knowledge::PatternMatch>>;

    // ── Git integration ──

    /// Commit current state with a message. Returns commit hash.
    fn commit(&self, message: &str) -> StoreResult<String>;

    /// Get recent commit history for a graph.
    fn history(&self, graph: &str, limit: usize) -> StoreResult<Vec<CommitInfo>>;

    // ── Thought branches ──

    /// Begin a thought — batch subsequent changes into one atomic commit.
    fn begin_thought(&self);

    /// End a thought — commit all batched changes with a descriptive message.
    fn end_thought(&self, message: &str) -> StoreResult<String>;

    /// Show what changed since a specific commit.
    fn diff_since(&self, commit: &str) -> StoreResult<String>;

    /// Show what's different on the current branch vs main.
    fn diff_from_main(&self) -> StoreResult<String>;

    /// Create a thought branch for speculative exploration.
    fn create_thought_branch(&self, name: &str) -> StoreResult<String>;

    /// Merge a thought branch into main — the ideas survived evaluation.
    fn merge_thought_branch(&self, branch: &str) -> StoreResult<bool>;

    /// Abandon a thought branch — the exploration was a dead end.
    fn abandon_thought_branch(&self, branch: &str) -> StoreResult<()>;

    /// List all thought branches.
    fn list_thought_branches(&self) -> StoreResult<Vec<String>>;

    /// Get the current branch name.
    fn current_branch(&self) -> StoreResult<String>;
}

// ── StorageBackend trait ───────────────────────────────────────────

/// Low-level storage operations. Implementations handle file I/O and git.
#[allow(dead_code)]
pub(crate) trait StorageBackend: Send + Sync {
    /// Load a graph by name. Returns None if it doesn't exist.
    fn load_graph(&self, name: &str) -> StoreResult<Option<crate::knowledge::GraphData>>;

    /// Save a graph. The backend handles atomicity.
    fn save_graph(&self, name: &str, data: &crate::knowledge::GraphData) -> StoreResult<()>;

    /// List all available graph names.
    fn list_graphs(&self) -> StoreResult<Vec<String>>;

    /// Delete a graph.
    fn delete_graph(&self, name: &str) -> StoreResult<()>;

    /// Commit current state to git. Returns commit hash.
    fn commit(&self, message: &str) -> StoreResult<String>;

    /// Get recent commit history for a graph file.
    fn history(&self, name: &str, limit: usize) -> StoreResult<Vec<CommitInfo>>;
}
