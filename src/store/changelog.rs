//! Semantic changelog — structured record of knowledge graph changes.
//!
//! Every graph mutation logs what actually changed (which nodes, edges,
//! confidence shifts) in a human and machine-readable format. This sits
//! alongside git commits — git tracks file-level changes, the changelog
//! tracks semantic changes.
//!
//! The AI can query this to answer: "when did I change my mind about X?"
//! "what was my confidence in Y three days ago?" "what did rumination
//! discover last cycle?"

use serde::{Deserialize, Serialize};
use std::path::Path;

/// A single change entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeEntry {
    pub timestamp: String,
    pub graph: String,
    pub kind: ChangeKind,
    pub description: String,
}

/// What kind of change.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    NodeAdded { label: String, node_kind: String },
    EdgeAdded { from: String, to: String, relation: String, confidence: f64 },
    EvidenceUpdated { from: String, to: String, relation: String, evidence_type: String, confidence_before: f64, confidence_after: f64 },
    EdgeStrengthened { from: String, to: String, relation: String, confidence_before: f64, confidence_after: f64 },
    EdgeWeakened { from: String, to: String, relation: String, confidence_before: f64, confidence_after: f64 },
    Consolidated { nodes_merged: usize, edges_merged: usize },
    BranchCreated { name: String },
    BranchMerged { name: String },
    BranchAbandoned { name: String },
    ColonyQuery { from_ant: String, entity: String },
    ColonyResponse { to_ant: String },
}

/// The changelog — append-only log of semantic changes.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Changelog {
    pub entries: Vec<ChangeEntry>,
}

#[allow(dead_code)]
impl Changelog {
    const MAX_ENTRIES: usize = 500;

    pub fn load(memory_dir: &Path) -> Self {
        let path = memory_dir.join("changelog.cbor");
        if path.exists() {
            if let Ok(bytes) = std::fs::read(&path) {
                if let Ok(log) = ciborium::de::from_reader::<Changelog, _>(&bytes[..]) {
                    return log;
                }
            }
            // Fall back to JSON if CBOR fails.
            let json_path = memory_dir.join("changelog.json");
            if json_path.exists() {
                if let Ok(contents) = std::fs::read_to_string(&json_path) {
                    if let Ok(log) = serde_json::from_str(&contents) {
                        return log;
                    }
                }
            }
        }
        Self::default()
    }

    pub fn save(&self, memory_dir: &Path) {
        let path = memory_dir.join("changelog.cbor");
        let mut buf = Vec::new();
        if ciborium::ser::into_writer(self, &mut buf).is_ok() {
            let tmp = path.with_extension("cbor.tmp");
            if let Ok(()) = std::fs::write(&tmp, &buf) {
                let _ = std::fs::rename(&tmp, &path);
            }
        }
    }

    pub fn append(&mut self, entry: ChangeEntry, memory_dir: &Path) {
        self.entries.push(entry);
        if self.entries.len() > Self::MAX_ENTRIES {
            let excess = self.entries.len() - Self::MAX_ENTRIES;
            self.entries.drain(..excess);
        }
        self.save(memory_dir);
    }

    /// Search the changelog for entries matching a query string.
    pub fn search(&self, query: &str, limit: usize) -> Vec<&ChangeEntry> {
        let query_lower = query.to_lowercase();
        self.entries.iter().rev()
            .filter(|e| {
                e.description.to_lowercase().contains(&query_lower)
                    || e.graph.to_lowercase().contains(&query_lower)
                    || match &e.kind {
                        ChangeKind::NodeAdded { label, .. } => label.to_lowercase().contains(&query_lower),
                        ChangeKind::EdgeAdded { from, to, relation, .. } |
                        ChangeKind::EvidenceUpdated { from, to, relation, .. } |
                        ChangeKind::EdgeStrengthened { from, to, relation, .. } |
                        ChangeKind::EdgeWeakened { from, to, relation, .. } => {
                            from.to_lowercase().contains(&query_lower)
                                || to.to_lowercase().contains(&query_lower)
                                || relation.to_lowercase().contains(&query_lower)
                        }
                        _ => false,
                    }
            })
            .take(limit)
            .collect()
    }

    /// Get recent entries.
    pub fn recent(&self, limit: usize) -> Vec<&ChangeEntry> {
        self.entries.iter().rev().take(limit).collect()
    }

    /// Render entries as text for the AI.
    pub fn render(entries: &[&ChangeEntry]) -> String {
        if entries.is_empty() { return "No changes found.".into(); }

        let mut text = String::new();
        for e in entries {
            text.push_str(&format!("{} [{}] {}\n", e.timestamp, e.graph, e.description));
        }
        text
    }
}
