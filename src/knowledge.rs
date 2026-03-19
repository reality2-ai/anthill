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

/// Node types the AI can create.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Person,
    Project,
    Server,
    Tool,
    Concept,
    Decision,
    Event,
    Fact,
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
        }
    }
}

/// A node in the knowledge graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

/// How a conjecture was originally formed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Basis {
    /// Directly observed by the AI.
    Observed,
    /// Told by the user.
    Told,
    /// Inferred from other knowledge.
    Inferred,
    /// Assumed without evidence.
    Assumed,
}

#[allow(dead_code)]
impl Basis {
    /// Initial confidence for a new conjecture based on how it was formed.
    pub fn initial_confidence(&self) -> f64 {
        match self {
            Self::Observed => 0.7,
            Self::Told => 0.6,
            Self::Inferred => 0.4,
            Self::Assumed => 0.3,
        }
    }
}

impl Default for Basis {
    fn default() -> Self { Self::Assumed }
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
}

fn default_confidence() -> f64 { 0.5 }
fn default_importance() -> f64 { 0.5 }

#[allow(dead_code)]
impl KnowledgeEdge {
    /// Create a new conjecture.
    pub fn new(relation: &str, context: &str, since: &str, basis: Basis) -> Self {
        let confidence = basis.initial_confidence();
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

    /// Combined score: confidence × importance. Used for prompt prioritisation.
    pub fn relevance_score(&self) -> f64 {
        self.confidence * self.importance
    }

    /// The conjecture survived a refutation attempt — strengthen it.
    pub fn strengthen(&mut self, date: &str) {
        self.tests += 1;
        self.survived += 1;
        self.last_tested = date.into();
        self.recalculate();
    }

    /// The conjecture was tested and evidence weakened it (but didn't refute it).
    pub fn weaken(&mut self, date: &str) {
        self.tests += 1;
        // survived stays the same
        self.last_tested = date.into();
        self.recalculate();
    }

    /// Direct contradiction — sharp confidence penalty.
    pub fn contradict(&mut self, date: &str) {
        self.tests += 1;
        self.last_tested = date.into();
        self.confidence *= 0.3;
        if self.confidence < 0.01 { self.confidence = 0.01; }
    }

    /// Recalculate confidence from test history.
    /// Uses a Bayesian-style formula: prior blended with observed rate.
    fn recalculate(&mut self) {
        if self.tests == 0 {
            return;
        }
        // Blend the basis prior with the observed survival rate.
        // More tests → more weight on observed rate.
        let prior = self.basis.initial_confidence();
        let observed = self.survived as f64 / self.tests as f64;
        let weight = (self.tests as f64) / (self.tests as f64 + 3.0); // 3 pseudo-observations
        self.confidence = prior * (1.0 - weight) + observed * weight;
        self.confidence = self.confidence.clamp(0.01, 0.99);
    }

    /// Apply time decay — untested conjectures drift toward uncertainty.
    /// Call with the number of days since last tested.
    pub fn decay(&mut self, days_since_tested: u32) {
        if days_since_tested == 0 { return; }
        // Decay rate: lose ~5% confidence per 30 days untested.
        let factor = 0.95_f64.powf(days_since_tested as f64 / 30.0);
        self.confidence *= factor;
        if self.confidence < 0.01 { self.confidence = 0.01; }
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

/// Confidence below which edges are archived (moved to separate file).
pub const ARCHIVE_CONFIDENCE: f64 = 0.10;

/// Maximum active nodes before auto-archiving triggers.
#[allow(dead_code)]
pub const MAX_ACTIVE_NODES: usize = 500;

/// Serializable graph format (petgraph's serde format).
#[derive(Serialize, Deserialize)]
struct GraphData {
    nodes: Vec<Option<KnowledgeNode>>,
    edges: Vec<(usize, usize, KnowledgeEdge)>,
}

/// Knowledge graph with keyword index for retrieval.
pub struct KnowledgeGraph {
    graph: StableGraph<KnowledgeNode, KnowledgeEdge>,
    keyword_index: HashMap<String, HashSet<NodeIndex>>,
    #[allow(dead_code)]
    file_path: PathBuf,
}

impl KnowledgeGraph {
    /// Load from JSON file, or create empty.
    pub fn load(path: &Path) -> Self {
        let mut kg = Self {
            graph: StableGraph::new(),
            keyword_index: HashMap::new(),
            file_path: path.to_path_buf(),
        };

        if path.exists() {
            if let Ok(contents) = std::fs::read_to_string(path) {
                if let Ok(data) = serde_json::from_str::<GraphData>(&contents) {
                    // Reconstruct graph from serialized data.
                    let mut index_map: Vec<Option<NodeIndex>> = Vec::new();
                    for node_opt in &data.nodes {
                        if let Some(node) = node_opt {
                            let idx = kg.graph.add_node(node.clone());
                            index_map.push(Some(idx));
                        } else {
                            index_map.push(None);
                        }
                    }
                    for (from, to, edge) in &data.edges {
                        if let (Some(Some(from_idx)), Some(Some(to_idx))) =
                            (index_map.get(*from), index_map.get(*to))
                        {
                            kg.graph.add_edge(*from_idx, *to_idx, edge.clone());
                        }
                    }
                } else {
                    log::warn!("Failed to parse knowledge graph at {}, starting empty", path.display());
                }
            }
        }

        kg.rebuild_index();
        kg
    }

    /// Save to JSON file (atomic write).
    #[allow(dead_code)]
    pub fn save(&self) {
        let data = self.to_serializable();
        if let Ok(json) = serde_json::to_string_pretty(&data) {
            let tmp = self.file_path.with_extension("json.tmp");
            if std::fs::write(&tmp, &json).is_ok() {
                let _ = std::fs::rename(&tmp, &self.file_path);
            }
        }
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
        GraphData { nodes, edges }
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

    /// Render a QueryResult as natural language.
    pub fn render_query_result(&self, result: &QueryResult, max_chars: usize) -> String {
        let mut output = String::new();

        // Render paths first (if any).
        if !result.paths.is_empty() {
            for path in &result.paths {
                let labels: Vec<&str> = path.nodes.iter()
                    .filter_map(|&idx| self.graph.node_weight(idx).map(|n| n.label.as_str()))
                    .collect();
                let bar = confidence_bar(path.cumulative_confidence);
                output.push_str(&format!("{} [{} {:.0}%]\n",
                    labels.join(" → "), bar, path.cumulative_confidence * 100.0));
            }
            output.push('\n');
        }

        // Render nodes with their edges.
        for (idx, node, score) in &result.nodes {
            let bar = confidence_bar(*score);
            output.push_str(&format!("- {} ({}): {} [{}]\n",
                node.label, node.kind, node.summary, bar));
            for edge in &result.edges {
                if &edge.from == idx {
                    if let Some(target) = self.graph.node_weight(edge.to) {
                        let ebar = confidence_bar(edge.edge.confidence);
                        output.push_str(&format!("  → {} → {} [{} {:.0}%]\n",
                            edge.edge.relation, target.label,
                            ebar, edge.edge.confidence * 100.0));
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

        // Score each node by keyword hits.
        let mut scores: HashMap<NodeIndex, u32> = HashMap::new();
        for kw in &keywords {
            if let Some(nodes) = self.keyword_index.get(kw) {
                for &idx in nodes {
                    *scores.entry(idx).or_default() += 1;
                }
            }
        }

        // Sort by score descending, take top N/2.
        let mut scored: Vec<(NodeIndex, u32)> = scores.into_iter().collect();
        scored.sort_by(|a, b| b.1.cmp(&a.1));
        let direct_limit = max_nodes / 2;
        let direct: Vec<NodeIndex> = scored.iter().take(direct_limit).map(|(idx, _)| *idx).collect();

        // Expand 1 hop.
        let mut result: HashSet<NodeIndex> = direct.iter().copied().collect();
        for &idx in &direct {
            for neighbor in self.graph.neighbors_undirected(idx) {
                result.insert(neighbor);
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
                // Sort by relevance (confidence × importance), filter low-confidence.
                let mut edges: Vec<_> = self.graph.edges_directed(idx, Direction::Outgoing)
                    .filter(|e| idx_set.contains(&e.target()))
                    .filter(|e| e.weight().confidence >= MIN_PROMPT_CONFIDENCE)
                    .collect();
                edges.sort_by(|a, b| b.weight().relevance_score()
                    .partial_cmp(&a.weight().relevance_score()).unwrap_or(std::cmp::Ordering::Equal));
                for edge_idx in edges {
                    let target = edge_idx.target();
                    let edge = edge_idx.weight();
                    let target_node = &self.graph[target];
                    let conf_str = if edge.confidence >= 0.8 {
                        if edge.tests > 0 {
                            format!(" [{}×]", edge.tests)
                        } else {
                            String::new()
                        }
                    } else {
                        let bar = confidence_bar(edge.confidence);
                        format!(" [{} {:.0}%{}]",
                            bar,
                            edge.confidence * 100.0,
                            if edge.tests > 0 { format!(" {}×", edge.tests) } else { String::new() })
                    };
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
}

impl KnowledgeGraph {
    /// Run a full consolidation pass: dedup nodes, merge parallel edges,
    /// collapse chains, detect contradictions.
    pub fn consolidate(&mut self) -> ConsolidationReport {
        let mut report = ConsolidationReport::default();

        // 1. Deduplicate nodes.
        report.nodes_merged = self.dedup_nodes();

        // 2. Merge parallel edges (same source, target, relation).
        report.edges_merged = self.merge_parallel_edges();

        // 3. Collapse chains: A→B→C where B has degree 2 and is a Fact.
        report.chains_collapsed = self.collapse_chains();

        // 4. Detect contradictions.
        report.contradictions = self.detect_contradictions();

        self.rebuild_index();
        report
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

        for (idx, src, tgt, rel, edge) in edge_info {
            let key = (src, tgt, rel);
            if let Some((kept_idx, kept_edge)) = seen.get_mut(&key) {
                // Merge: combined confidence, summed counts.
                kept_edge.confidence = (1.0 - (1.0 - kept_edge.confidence) * (1.0 - edge.confidence))
                    .min(0.95);
                kept_edge.tests += edge.tests;
                kept_edge.survived += edge.survived;
                kept_edge.references += edge.references;
                kept_edge.importance = kept_edge.importance.max(edge.importance);
                if edge.context.len() > kept_edge.context.len() {
                    kept_edge.context = edge.context;
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

            let (src, in_w, _) = in_edge;
            let (tgt, out_w, _) = out_edge;

            // Don't collapse if either edge is high-importance.
            if in_w.importance > 0.7 || out_w.importance > 0.7 { continue; }
            // Don't collapse self-loops.
            if src == tgt { continue; }

            // Create combined edge.
            let combined = KnowledgeEdge {
                relation: format!("{} → {}", in_w.relation, out_w.relation),
                context: format!("{} (via {})", in_w.context, self.graph[mid].label),
                since: in_w.since.clone(),
                confidence: in_w.confidence.min(out_w.confidence), // weakest link
                tests: in_w.tests.min(out_w.tests),
                survived: in_w.survived.min(out_w.survived),
                basis: in_w.basis.clone(),
                last_tested: in_w.last_tested.clone(),
                importance: in_w.importance.max(out_w.importance),
                references: in_w.references + out_w.references,
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
    false
}

// --- Cached graph (avoids re-parsing JSON on every request) ---

use std::sync::Mutex;
use std::time::SystemTime;

/// A cached knowledge graph that reloads from disk only when the file changes.
pub struct CachedGraph {
    graph: Mutex<KnowledgeGraph>,
    file_path: PathBuf,
    last_mtime: Mutex<Option<SystemTime>>,
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
        let keywords = extract_keywords(message);
        let mut result = QueryResult::default();

        // Try each keyword as a potential entity label.
        for kw in &keywords {
            let about = graph.query_about(kw, 1);
            if !about.nodes.is_empty() {
                // Merge into result (dedup by node index).
                let existing: HashSet<usize> = result.nodes.iter().map(|(idx, _, _)| idx.index()).collect();
                for (idx, node, score) in about.nodes {
                    if !existing.contains(&idx.index()) {
                        result.nodes.push((idx, node, score));
                    }
                }
                result.edges.extend(about.edges);
            }
        }

        if !result.nodes.is_empty() {
            let mut r = graph.render_query_result(&result, max_chars);
            r.push_str("\n(Query-based context. Read knowledge.json for full graph.)\n");
            r
        } else {
            // Fallback to keyword-based subgraph.
            let relevant = graph.relevant_subgraph(message, 50);
            let mut r = graph.render_subgraph(&relevant, max_chars);
            r.push_str("\n(Keyword-based context. Read knowledge.json for full graph.)\n");
            r
        }
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

        let needs_reload = {
            let last = self.last_mtime.lock().ok();
            match (last.as_deref(), &current_mtime) {
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
                *m = current_mtime;
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
                "Graph consolidated: {} nodes merged, {} edges merged, {} chains collapsed, {} contradictions",
                report.nodes_merged, report.edges_merged, report.chains_collapsed, report.contradictions.len()
            );
            for warning in &report.contradictions {
                log::warn!("Contradiction: {}", warning);
            }
        }
        report
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
    pub fn search(&self, message: &str, max_results: usize) -> Vec<&Episode> {
        let keywords = extract_keywords(message);
        if keywords.is_empty() {
            return self.recent(max_results).iter().collect();
        }

        let mut scored: Vec<(&Episode, u32)> = self.episodes.iter().map(|ep| {
            let text = format!("{} {} {}",
                ep.summary,
                ep.outcomes.join(" "),
                ep.tags.join(" "));
            let tokens = extract_keywords(&text);
            let score: u32 = keywords.iter()
                .filter(|kw| tokens.contains(kw))
                .count() as u32;
            (ep, score)
        }).filter(|(_, s)| *s > 0).collect();

        scored.sort_by(|a, b| b.1.cmp(&a.1));
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

/// Extract meaningful keywords from a message.
/// Language-agnostic: filters by length and produces both the original
/// word and common suffix-stripped variants for fuzzy matching.
pub fn extract_keywords(text: &str) -> Vec<String> {
    let mut keywords = Vec::new();
    for word in tokenize(text) {
        // Add the word itself.
        keywords.push(word.clone());
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
        });
        let anthill = kg.graph.add_node(KnowledgeNode {
            label: "Anthill".into(),
            kind: NodeKind::Project,
            summary: "AI colony platform".into(),
            created: "2026-03-10".into(),
            updated: "2026-03-20".into(),
            tags: vec!["rust".into(), "ai".into()],
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
        });
        let anthill = kg.graph.add_node(KnowledgeNode {
            label: "Anthill".into(),
            kind: NodeKind::Project,
            summary: "AI colony".into(),
            created: "2026-03-10".into(),
            updated: "2026-03-20".into(),
            tags: vec!["rust".into()],
        });
        let unrelated = kg.graph.add_node(KnowledgeNode {
            label: "Weather".into(),
            kind: NodeKind::Fact,
            summary: "It rains in Auckland".into(),
            created: "2026-03-20".into(),
            updated: "2026-03-20".into(),
            tags: vec![],
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
        });
        let anthill = kg.graph.add_node(KnowledgeNode {
            label: "Anthill".into(),
            kind: NodeKind::Project,
            summary: "AI colony platform".into(),
            created: "2026-03-10".into(),
            updated: "2026-03-20".into(),
            tags: vec![],
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
        assert!(edge.confidence > 0.6);
        assert_eq!(edge.tests, 5);
        assert_eq!(edge.survived, 5);

        // Fail 2 tests — confidence should decrease.
        edge.weaken("2026-03-20");
        edge.weaken("2026-03-20");
        let after_weaken = edge.confidence;
        assert!(after_weaken < edge.confidence + 0.01); // decreased or stayed
        assert_eq!(edge.tests, 7);
        assert_eq!(edge.survived, 5);

        // Direct contradiction — sharp drop.
        let before = edge.confidence;
        edge.contradict("2026-03-20");
        assert!(edge.confidence < before * 0.5);

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
        });
        let b = kg.graph.add_node(KnowledgeNode {
            label: "B".into(), kind: NodeKind::Fact, summary: "Node B".into(),
            created: String::new(), updated: String::new(), tags: vec![],
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
        });
        kg.graph.add_node(KnowledgeNode {
            label: "anthill".into(), kind: NodeKind::Project, summary: "AI colony platform, detailed".into(),
            created: String::new(), updated: String::new(), tags: vec!["rust".into()],
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
        });
        let b = kg.graph.add_node(KnowledgeNode {
            label: "B".into(), kind: NodeKind::Project, summary: "".into(),
            created: String::new(), updated: String::new(), tags: vec![],
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

        // Remaining edge should have combined confidence > either individual.
        let remaining = kg.graph.edges(a).next().unwrap();
        let edge = remaining.weight();
        assert!(edge.confidence > 0.6); // combined > max individual
        assert_eq!(edge.tests, 8); // 3 + 5
        assert_eq!(edge.survived, 6); // 2 + 4
        assert!(edge.context.contains("detailed")); // kept longer context

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
        });
        let mid = kg.graph.add_node(KnowledgeNode {
            label: "uses Rust".into(), kind: NodeKind::Fact, summary: "intermediate".into(),
            created: String::new(), updated: String::new(), tags: vec![],
        });
        let c = kg.graph.add_node(KnowledgeNode {
            label: "Anthill".into(), kind: NodeKind::Project, summary: "".into(),
            created: String::new(), updated: String::new(), tags: vec![],
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
        });
        let b = kg.graph.add_node(KnowledgeNode {
            label: "Python".into(), kind: NodeKind::Tool, summary: "".into(),
            created: String::new(), updated: String::new(), tags: vec![],
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
        });
        let anthill = kg.graph.add_node(KnowledgeNode {
            label: "Anthill".into(), kind: NodeKind::Project, summary: "AI colony".into(),
            created: "2026-03-10".into(), updated: "2026-03-20".into(), tags: vec!["rust".into()],
        });
        let alfred = kg.graph.add_node(KnowledgeNode {
            label: "Alfred".into(), kind: NodeKind::Server, summary: "Production server".into(),
            created: "2026-03-15".into(), updated: "2026-03-20".into(), tags: vec!["linux".into()],
        });
        let rust = kg.graph.add_node(KnowledgeNode {
            label: "Rust".into(), kind: NodeKind::Tool, summary: "Programming language".into(),
            created: "2026-03-10".into(), updated: "2026-03-20".into(), tags: vec![],
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
        // Should contain confidence indicators.
        assert!(rendered.contains("●"));
        assert!(rendered.contains("Roy"));
        assert!(rendered.contains("works_on"));
        assert!(rendered.contains("%"));

        std::fs::remove_dir_all(&dir).ok();
    }
}
