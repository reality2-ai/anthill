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

/// An edge in the knowledge graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeEdge {
    pub relation: String,
    #[serde(default)]
    pub context: String,
    #[serde(default)]
    pub since: String,
}

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
                for edge_idx in self.graph.edges_directed(idx, Direction::Outgoing) {
                    let target = edge_idx.target();
                    if idx_set.contains(&target) {
                        let edge = edge_idx.weight();
                        let target_node = &self.graph[target];
                        output.push_str(&format!(
                            "  → {} → {}\n",
                            edge.relation, target_node.label
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

// --- Keyword extraction (Layer 2) ---

/// Extract meaningful keywords from a message.
pub fn extract_keywords(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '-' && c != '_')
        .filter(|w| w.len() > 2)
        .filter(|w| !is_stop_word(w))
        .map(|w| stem(w))
        .collect()
}

/// Tokenize text into lowercase words.
fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '-' && c != '_')
        .filter(|w| w.len() > 2)
        .filter(|w| !is_stop_word(w))
        .map(|w| stem(w))
        .collect()
}

/// Minimal stemming: strip common English suffixes.
fn stem(word: &str) -> String {
    let w = word.to_lowercase();
    if w.len() > 5 {
        if let Some(base) = w.strip_suffix("ing") {
            return base.to_string();
        }
        if let Some(base) = w.strip_suffix("tion") {
            return base.to_string();
        }
        if let Some(base) = w.strip_suffix("ed") {
            return base.to_string();
        }
    }
    if w.len() > 3 {
        if let Some(base) = w.strip_suffix('s') {
            return base.to_string();
        }
    }
    w
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().to_string() + c.as_str(),
    }
}

fn is_stop_word(w: &str) -> bool {
    matches!(
        w,
        "the" | "this" | "that" | "these" | "those"
            | "and" | "but" | "for" | "nor" | "not" | "yet" | "also"
            | "are" | "was" | "were" | "been" | "being" | "have" | "has" | "had"
            | "does" | "did" | "will" | "would" | "could" | "should" | "may" | "might"
            | "can" | "shall" | "must"
            | "its" | "his" | "her" | "our" | "your" | "their" | "who" | "whom"
            | "what" | "which" | "when" | "where" | "how" | "why"
            | "with" | "from" | "into" | "about" | "between" | "through"
            | "during" | "before" | "after" | "above" | "below" | "over" | "under"
            | "then" | "than" | "some" | "such" | "each" | "every" | "all" | "any"
            | "both" | "few" | "more" | "most" | "other" | "very" | "just" | "too"
            | "here" | "there" | "now" | "well" | "only"
    )
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
        kg.graph.add_edge(roy, anthill, KnowledgeEdge {
            relation: "works_on".into(),
            context: "Lead developer".into(),
            since: "2026-03-10".into(),
        });
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
        assert!(!kw.contains(&"the".to_string())); // stop word
        assert!(!kw.contains(&"what".to_string())); // stop word
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
        kg.graph.add_edge(roy, anthill, KnowledgeEdge {
            relation: "works_on".into(),
            context: "".into(),
            since: "".into(),
        });
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
        kg.graph.add_edge(roy, anthill, KnowledgeEdge {
            relation: "works_on".into(),
            context: "".into(),
            since: "".into(),
        });
        kg.rebuild_index();

        let rendered = kg.render_full(4096);
        assert!(rendered.contains("## Person"));
        assert!(rendered.contains("Roy (person): Project lead"));
        assert!(rendered.contains("→ works_on → Anthill"));
        assert!(rendered.contains("## Project"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn stemming() {
        assert_eq!(stem("running"), "runn");
        assert_eq!(stem("projects"), "project");
        assert_eq!(stem("deployed"), "deploy");
        assert_eq!(stem("configuration"), "configura");
        assert_eq!(stem("architecture"), "architecture"); // no matching suffix
        assert_eq!(stem("ai"), "ai"); // too short to stem
    }
}
