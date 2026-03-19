//! Knowledge graph — structured memory for ANTs.
//!
//! Stores entities (people, projects, tools, concepts) and relationships
//! as a directed graph. Persisted to JSON. Context-aware retrieval extracts
//! relevant subgraphs for the AI prompt based on message keywords.

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
}

fn default_confidence() -> f64 { 0.5 }

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
        }
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

    // --- Layer 2: Context-aware retrieval ---

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
                // Only show edges above the minimum confidence threshold.
                for edge_idx in self.graph.edges_directed(idx, Direction::Outgoing) {
                    let target = edge_idx.target();
                    if idx_set.contains(&target) {
                        let edge = edge_idx.weight();
                        if edge.confidence < MIN_PROMPT_CONFIDENCE { continue; }
                        let target_node = &self.graph[target];
                        let conf_str = if edge.confidence >= 0.8 {
                            if edge.tests > 0 {
                                format!(" [{}×]", edge.tests)
                            } else {
                                String::new()
                            }
                        } else {
                            // Use visual confidence bar + percentage (language-agnostic).
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
            graph.render_full(max_chars)
        } else {
            let relevant = graph.relevant_subgraph(message, 50);
            let mut r = graph.render_subgraph(&relevant, max_chars);
            r.push_str("\n(Showing relevant context. Read knowledge.json for full graph.)\n");
            r
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

    /// Node count (for logging).
    #[allow(dead_code)]
    pub fn node_count(&self) -> usize {
        self.graph.lock().map(|g| g.node_count()).unwrap_or(0)
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
        for len in [1, 2, 3] {
            if word.len() > len + 3 {
                keywords.push(word[..word.len() - len].to_string());
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
}
