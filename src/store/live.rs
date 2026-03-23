//! LiveKnowledgeStore — the primary KnowledgeStore implementation.
//!
//! Manages multiple named graphs (meta + topics), caches them in memory,
//! validates all writes, and delegates to KnowledgeGraph methods.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::RwLock;

use crate::knowledge::{
    self, KnowledgeGraph, KnowledgeNode, CompetitorGroup, ContradictionPair,
    PatternMatch, UncertaintyStats, QueryResult, NodeKind,
};
use crate::store::{
    KnowledgeStore, StorageBackend, StoreError, StoreResult,
    NodeId, EdgeId, EdgeUpdate, GraphInfo, GraphStats, CommitInfo,
    ConsolidationReport,
    ValidatedNode, ValidatedEdge, ValidatedEvidence,
};
use crate::store::cbor_backend::CborGitBackend;
use crate::store::changelog::{Changelog, ChangeEntry, ChangeKind};

/// The primary implementation of KnowledgeStore.
/// Uses CBOR+Git backend for persistence, KnowledgeGraph for in-memory operations.
pub struct LiveKnowledgeStore {
    backend: CborGitBackend,
    memory_dir: PathBuf,
    /// Cached graphs, keyed by name ("meta", "anthill", etc.).
    graphs: RwLock<HashMap<String, KnowledgeGraph>>,
}

impl LiveKnowledgeStore {
    /// Create a new store backed by CBOR files with git auto-commit.
    pub fn new(memory_dir: PathBuf) -> Self {
        let backend = CborGitBackend::new(memory_dir.clone());
        Self {
            backend,
            memory_dir,
            graphs: RwLock::new(HashMap::new()),
        }
    }

    /// Get or load a graph by name. Caller must hold the write lock.
    /// When both CBOR and JSON exist, loads the NEWER one (by mtime).
    fn ensure_loaded(graphs: &mut HashMap<String, KnowledgeGraph>, name: &str, memory_dir: &std::path::Path) -> StoreResult<()> {
        if !graphs.contains_key(name) {
            let cbor_path = if name == "meta" || name.is_empty() {
                memory_dir.join("knowledge.cbor")
            } else {
                memory_dir.join("graphs").join(format!("{}.cbor", name))
            };
            let json_path = graph_path(memory_dir, name);

            let cbor_exists = cbor_path.exists();
            let json_exists = json_path.exists();

            // When both exist, prefer the newer one (the AI might have edited JSON directly).
            let use_cbor = if cbor_exists && json_exists {
                let cbor_mtime = std::fs::metadata(&cbor_path).ok().and_then(|m| m.modified().ok());
                let json_mtime = std::fs::metadata(&json_path).ok().and_then(|m| m.modified().ok());
                match (cbor_mtime, json_mtime) {
                    (Some(c), Some(j)) => c >= j,
                    _ => true, // Default to CBOR if mtime unavailable.
                }
            } else {
                cbor_exists
            };

            // If CBOR exists but has no citations, and JSON does, prefer JSON
            // (the AI may have added citations directly to JSON).
            let prefer_json = if use_cbor && json_exists {
                let cbor_has_cites = std::fs::read(&cbor_path).ok()
                    .and_then(|b| ciborium::de::from_reader::<crate::knowledge::GraphData, _>(&b[..]).ok())
                    .map(|d| d.edges.iter().any(|(_, _, e)| !e.citations.is_empty()))
                    .unwrap_or(false);
                let json_has_cites = std::fs::read_to_string(&json_path).ok()
                    .and_then(|c| serde_json::from_str::<crate::knowledge::GraphData>(&c).ok())
                    .map(|d| d.edges.iter().any(|(_, _, e)| !e.citations.is_empty()))
                    .unwrap_or(false);
                !cbor_has_cites && json_has_cites
            } else {
                false
            };

            let load_from_cbor = use_cbor && !prefer_json;

            let kg = if load_from_cbor {
                match std::fs::read(&cbor_path) {
                    Ok(bytes) => {
                        match ciborium::de::from_reader::<crate::knowledge::GraphData, _>(&bytes[..]) {
                            Ok(data) => {
                                let mut kg = KnowledgeGraph::empty(cbor_path);
                                kg.load_from_data(&data);
                                kg
                            }
                            Err(e) => {
                                log::warn!("CBOR parse failed for {}, falling back to JSON: {}", name, e);
                                KnowledgeGraph::load(&json_path)
                            }
                        }
                    }
                    Err(e) => {
                        log::warn!("Failed to read {}: {}", cbor_path.display(), e);
                        KnowledgeGraph::load(&json_path)
                    }
                }
            } else if json_exists {
                log::info!("Loading '{}' from JSON (has citations that CBOR lacks)", name);
                KnowledgeGraph::load(&json_path)
            } else if cbor_exists {
                // CBOR exists but no JSON — just load CBOR.
                match std::fs::read(&cbor_path) {
                    Ok(bytes) => {
                        match ciborium::de::from_reader::<crate::knowledge::GraphData, _>(&bytes[..]) {
                            Ok(data) => {
                                let mut kg = KnowledgeGraph::empty(cbor_path.clone());
                                kg.load_from_data(&data);
                                kg
                            }
                            Err(_) => KnowledgeGraph::empty(cbor_path)
                        }
                    }
                    Err(_) => KnowledgeGraph::empty(cbor_path)
                }
            } else {
                // Neither exists — empty graph.
                KnowledgeGraph::empty(cbor_path)
            };

            // If we loaded from JSON (not CBOR), auto-save as CBOR to keep formats in sync.
            // This handles the case where the AI edited JSON directly.
            if !use_cbor && json_exists && kg.node_count() > 0 {
                let data = kg.to_graph_data();
                let cbor_path_for_save = if name == "meta" || name.is_empty() {
                    memory_dir.join("knowledge.cbor")
                } else {
                    memory_dir.join("graphs").join(format!("{}.cbor", name))
                };
                if let Some(parent) = cbor_path_for_save.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let mut buf = Vec::new();
                if ciborium::ser::into_writer(&data, &mut buf).is_ok() {
                    use std::io::Write;
                    let tmp = cbor_path_for_save.with_extension("cbor.tmp");
                    if let Ok(mut f) = std::fs::File::create(&tmp) {
                        if f.write_all(&buf).is_ok() && f.sync_all().is_ok() {
                            let _ = std::fs::rename(&tmp, &cbor_path_for_save);
                            log::info!("Auto-converted {} JSON → CBOR ({} nodes)", name, kg.node_count());
                        }
                    }
                }
            }

            graphs.insert(name.to_string(), kg);
        }
        Ok(())
    }

    /// Get a read-only reference to a graph, loading if needed.
    fn with_graph<F, R>(&self, name: &str, f: F) -> StoreResult<R>
    where
        F: FnOnce(&KnowledgeGraph) -> StoreResult<R>,
    {
        // Try read lock first.
        {
            let graphs = self.graphs.read().map_err(|_| StoreError::Storage("lock poisoned".into()))?;
            if let Some(kg) = graphs.get(name) {
                return f(kg);
            }
        }
        // Need to load — acquire write lock.
        let mut graphs = self.graphs.write().map_err(|_| StoreError::Storage("lock poisoned".into()))?;
        Self::ensure_loaded(&mut graphs, name, &self.memory_dir)?;
        let kg = graphs.get(name).ok_or_else(|| StoreError::NotFound(format!("graph '{}'", name)))?;
        f(kg)
    }

    /// Get a mutable reference to a graph, loading if needed. Saves after mutation via CBOR backend.
    fn with_graph_mut<F, R>(&self, name: &str, f: F) -> StoreResult<R>
    where
        F: FnOnce(&mut KnowledgeGraph) -> StoreResult<R>,
    {
        let mut graphs = self.graphs.write().map_err(|_| StoreError::Storage("lock poisoned".into()))?;
        Self::ensure_loaded(&mut graphs, name, &self.memory_dir)?;
        let kg = graphs.get_mut(name).ok_or_else(|| StoreError::NotFound(format!("graph '{}'", name)))?;
        let result = f(kg)?;
        // Save after mutation — through CBOR backend (auto-commits to git).
        let data = kg.to_graph_data();
        self.backend.save_graph(name, &data)?;
        Ok(result)
    }

    /// Get the memory directory path (for components that need it during migration).
    #[allow(dead_code)]
    pub fn memory_dir(&self) -> &std::path::Path {
        &self.memory_dir
    }

    /// Add an edge by NodeId (for synthesis and other internal operations).
    pub fn add_edge_by_id(&self, graph: &str, from: NodeId, to: NodeId, edge: crate::knowledge::KnowledgeEdge) -> StoreResult<EdgeId> {
        self.with_graph_mut(graph, |kg| {
            let eid = kg.graph.add_edge(from.0, to.0, edge);
            Ok(EdgeId(eid))
        })
    }

    /// Log a semantic change to the changelog.
    fn log_change(&self, graph: &str, kind: ChangeKind, description: &str) {
        let mut changelog = Changelog::load(&self.memory_dir);
        changelog.append(ChangeEntry {
            timestamp: crate::dateutil::datetime_now(),
            graph: graph.into(),
            kind,
            description: description.into(),
        }, &self.memory_dir);
    }

    /// Search the semantic changelog.
    #[allow(dead_code)]
    pub fn search_changelog(&self, query: &str, limit: usize) -> Vec<String> {
        let changelog = Changelog::load(&self.memory_dir);
        let entries = changelog.search(query, limit);
        entries.iter().map(|e| format!("{} [{}] {}", e.timestamp, e.graph, e.description)).collect()
    }

    /// Render a query result using the graph's render method.
    /// This is a convenience for MCP/web consumers that need formatted output.
    pub fn with_graph_render(&self, graph: &str, result: &QueryResult) -> Option<String> {
        let graphs = self.graphs.read().ok()?;
        let kg = graphs.get(graph)?;
        Some(kg.render_query_result(result, 8000))
    }

    /// Invalidate a cached graph so it's reloaded from disk next time.
    #[allow(dead_code)]
    pub fn invalidate(&self, name: &str) {
        if let Ok(mut graphs) = self.graphs.write() {
            graphs.remove(name);
        }
    }

    /// Invalidate all cached graphs.
    #[allow(dead_code)]
    pub fn invalidate_all(&self) {
        if let Ok(mut graphs) = self.graphs.write() {
            graphs.clear();
        }
    }
}

impl KnowledgeStore for LiveKnowledgeStore {
    // ── Graph management ──

    fn list_graphs(&self) -> StoreResult<Vec<GraphInfo>> {
        let names = self.backend.list_graphs()?;
        let mut result = Vec::new();
        for name in names {
            let info = self.with_graph(&name, |kg| {
                Ok(GraphInfo {
                    name: name.clone(),
                    node_count: kg.node_count(),
                    edge_count: kg.edge_count(),
                })
            })?;
            result.push(info);
        }
        Ok(result)
    }

    fn graph_stats(&self, graph: &str) -> StoreResult<GraphStats> {
        self.with_graph(graph, |kg| {
            let stats = kg.uncertainty_stats();
            Ok(GraphStats {
                name: graph.into(),
                node_count: kg.node_count(),
                edge_count: stats.edge_count,
                avg_confidence: stats.avg_confidence,
                uncertain_edges: stats.uncertain_edge_count,
                orphan_nodes: 0, // TODO
            })
        })
    }

    // ── Node operations ──

    fn add_node(&self, graph: &str, node: ValidatedNode) -> StoreResult<NodeId> {
        let label = node.inner.label.clone();
        let kind = node.inner.kind.to_string();
        let result = self.with_graph_mut(graph, |kg| {
            if kg.find_by_label(&label).is_some() {
                return Err(StoreError::Duplicate(format!("node '{}' already exists", label)));
            }
            let idx = kg.graph.add_node(node.inner);
            kg.rebuild_index();
            Ok(NodeId(idx))
        })?;
        self.log_change(graph,
            ChangeKind::NodeAdded { label: label.clone(), node_kind: kind.clone() },
            &format!("Added node '{}' ({})", label, kind));
        Ok(result)
    }

    fn get_node(&self, graph: &str, label: &str) -> StoreResult<KnowledgeNode> {
        self.with_graph(graph, |kg| {
            let idx = kg.find_by_label(label)
                .ok_or_else(|| StoreError::NotFound(format!("node '{}' in graph '{}'", label, graph)))?;
            Ok(kg.graph[idx].clone())
        })
    }

    fn list_nodes(&self, graph: &str) -> StoreResult<Vec<String>> {
        self.with_graph(graph, |kg| {
            Ok(kg.all_node_labels())
        })
    }

    // ── Edge operations ──

    fn add_edge(&self, graph: &str, edge: ValidatedEdge) -> StoreResult<EdgeId> {
        self.with_graph_mut(graph, |kg| {
            let from_idx = kg.find_by_label(&edge.from_label)
                .ok_or_else(|| StoreError::NotFound(format!("node '{}' not found", edge.from_label)))?;
            let to_idx = kg.find_by_label(&edge.to_label)
                .ok_or_else(|| StoreError::NotFound(format!("node '{}' not found", edge.to_label)))?;
            let eid = kg.graph.add_edge(from_idx, to_idx, edge.inner);
            Ok(EdgeId(eid))
        })
    }

    fn update_evidence(
        &self, graph: &str, from: &str, to: &str, relation: &str,
        evidence: ValidatedEvidence,
    ) -> StoreResult<EdgeUpdate> {
        let from_s = from.to_string();
        let to_s = to.to_string();
        let rel_s = relation.to_string();
        let ev_type = format!("{:?}", evidence.evidence_type);

        let result = self.with_graph_mut(graph, |kg| {
            let (_eid, edge) = find_edge_mut(kg, from, to, relation)?;
            let before_conf = edge.confidence;
            let before_lo = edge.log_odds;
            edge.update_with_evidence(
                evidence.evidence_type.clone(),
                &evidence.date,
                &evidence.test,
                &evidence.detail,
                &evidence.source_id,
                evidence.source_reputation,
            );

            // Check for confirmation bias patterns.
            let warning = detect_confirmation_bias(edge);

            Ok(EdgeUpdate {
                confidence_before: before_conf,
                confidence_after: edge.confidence,
                log_odds_before: before_lo,
                log_odds_after: edge.log_odds,
                evidence_type: format!("{:?}", evidence.evidence_type),
                bayes_factor: evidence.evidence_type.effective_bayes_factor(evidence.source_reputation),
                confirmation_bias_warning: warning,
            })
        })?;
        self.log_change(graph,
            ChangeKind::EvidenceUpdated {
                from: from_s, to: to_s, relation: rel_s, evidence_type: ev_type,
                confidence_before: result.confidence_before, confidence_after: result.confidence_after,
            },
            &format!("Evidence: {:.0}% → {:.0}%", result.confidence_before * 100.0, result.confidence_after * 100.0));
        Ok(result)
    }

    fn strengthen(
        &self, graph: &str, from: &str, to: &str, relation: &str,
        test: &str, evidence: &str,
    ) -> StoreResult<EdgeUpdate> {
        self.with_graph_mut(graph, |kg| {
            let (_eid, edge) = find_edge_mut(kg, from, to, relation)?;
            let before = (edge.confidence, edge.log_odds);
            edge.strengthen_with(&today(), test, evidence);
            let warning = detect_confirmation_bias(edge);
            Ok(EdgeUpdate {
                confidence_before: before.0,
                confidence_after: edge.confidence,
                log_odds_before: before.1,
                log_odds_after: edge.log_odds,
                evidence_type: "refutation_survived".into(),
                bayes_factor: 2.5,
                confirmation_bias_warning: warning,
            })
        })
    }

    fn weaken(
        &self, graph: &str, from: &str, to: &str, relation: &str,
        test: &str, evidence: &str,
    ) -> StoreResult<EdgeUpdate> {
        self.with_graph_mut(graph, |kg| {
            let (_eid, edge) = find_edge_mut(kg, from, to, relation)?;
            let before = (edge.confidence, edge.log_odds);
            edge.weaken_with(&today(), test, evidence);
            Ok(EdgeUpdate {
                confidence_before: before.0,
                confidence_after: edge.confidence,
                log_odds_before: before.1,
                log_odds_after: edge.log_odds,
                evidence_type: "inconsistency".into(),
                bayes_factor: 0.4,
                confirmation_bias_warning: None,
            })
        })
    }

    fn contradict(
        &self, graph: &str, from: &str, to: &str, relation: &str,
        test: &str, evidence: &str,
    ) -> StoreResult<EdgeUpdate> {
        self.with_graph_mut(graph, |kg| {
            let (_eid, edge) = find_edge_mut(kg, from, to, relation)?;
            let before = (edge.confidence, edge.log_odds);
            edge.contradict_with(&today(), test, evidence);
            Ok(EdgeUpdate {
                confidence_before: before.0,
                confidence_after: edge.confidence,
                log_odds_before: before.1,
                log_odds_after: edge.log_odds,
                evidence_type: "refutation_failed".into(),
                bayes_factor: 0.1,
                confirmation_bias_warning: None,
            })
        })
    }

    fn add_citation(
        &self, graph: &str, from: &str, to: &str, relation: &str,
        citation: crate::knowledge::Reference,
    ) -> StoreResult<()> {
        self.with_graph_mut(graph, |kg| {
            let (_eid, edge) = find_edge_mut(kg, from, to, relation)?;
            // Avoid duplicate citations (same cite_id or same URL).
            let dominated = edge.citations.iter().any(|c|
                (!c.cite_id.is_empty() && c.cite_id == citation.cite_id) ||
                (!c.url.is_empty() && c.url == citation.url)
            );
            if !dominated {
                edge.citations.push(citation);
            }
            Ok(())
        })
    }

    // ── Queries ──

    fn query_about(&self, graph: &str, entity: &str, depth: usize) -> StoreResult<QueryResult> {
        self.with_graph(graph, |kg| Ok(kg.query_about(entity, depth)))
    }

    fn query_path(&self, graph: &str, from: &str, to: &str, max_paths: usize) -> StoreResult<QueryResult> {
        self.with_graph(graph, |kg| Ok(kg.query_path(from, to, max_paths)))
    }

    fn query_by_kind(&self, graph: &str, kind: &str) -> StoreResult<QueryResult> {
        let node_kind: NodeKind = serde_json::from_value(serde_json::Value::String(kind.into()))
            .unwrap_or(NodeKind::Other);
        self.with_graph(graph, |kg| Ok(kg.query_by_kind(&node_kind)))
    }

    fn query_uncertain(&self, graph: &str, threshold: f64) -> StoreResult<QueryResult> {
        self.with_graph(graph, |kg| Ok(kg.query_uncertain(threshold)))
    }

    fn query_justification(&self, graph: &str, from: &str, to: &str, relation: &str) -> StoreResult<String> {
        self.with_graph(graph, |kg| {
            let (_eid, edge) = find_edge(kg, from, to, relation)?;
            let mut text = String::new();
            for step in &edge.justificatory_chain {
                text.push_str(&format!("{}. {} (conf: {:.0}%, source: {})\n",
                    step.step, step.process, step.confidence * 100.0, step.source));
            }
            if text.is_empty() {
                text = "No justificatory chain recorded.".into();
            }
            Ok(text)
        })
    }

    fn list_orphans(&self, graph: &str) -> StoreResult<Vec<String>> {
        self.with_graph(graph, |kg| {
            Ok(kg.undetermined_connections(100).into_iter()
                .flat_map(|(a, b)| vec![a, b])
                .collect())
        })
    }

    // ── Rendering ──

    fn render_for_prompt(&self, message: &str, max_chars: usize) -> String {
        // Load meta graph + relevant topic graphs.
        let meta_path = self.memory_dir.join("knowledge.json");
        let cached = knowledge::CachedGraph::new(&meta_path);
        cached.render_for_prompt(message, max_chars)
    }

    fn to_visualization(&self, graph: &str) -> StoreResult<serde_json::Value> {
        self.with_graph(graph, |kg| Ok(kg.to_visualization()))
    }

    // ── Maintenance ──

    fn consolidate(&self, graph: &str) -> StoreResult<ConsolidationReport> {
        self.with_graph_mut(graph, |kg| {
            kg.backfill_refutation_logs();
            kg.backfill_to_thurisaz();
            let report = kg.consolidate();
            kg.link_orphans(graph);
            Ok(ConsolidationReport {
                nodes_merged: report.nodes_merged,
                edges_merged: report.edges_merged,
                chains_collapsed: report.chains_collapsed,
                contradictions: report.contradictions,
                clusters: report.clusters,
            })
        })
    }

    fn apply_decay(&self, graph: &str, days: u32) -> StoreResult<u32> {
        self.with_graph_mut(graph, |kg| {
            let mut decayed = 0u32;
            let edge_indices: Vec<_> = kg.graph.edge_indices().collect();
            for eid in edge_indices {
                let before = kg.graph[eid].confidence;
                kg.graph[eid].decay(days);
                if (before - kg.graph[eid].confidence).abs() > 0.001 {
                    decayed += 1;
                }
            }
            Ok(decayed)
        })
    }

    fn compute_corroboration_strength(&self, graph: &str) -> StoreResult<()> {
        self.with_graph_mut(graph, |kg| {
            kg.compute_corroboration_strength();
            Ok(())
        })
    }

    fn link_orphans(&self, graph: &str) -> StoreResult<u32> {
        self.with_graph_mut(graph, |kg| {
            // Count orphans before.
            let before = kg.undetermined_connections(1000).len();
            kg.link_orphans(graph);
            let after = kg.undetermined_connections(1000).len();
            // after can be larger than before if linking creates new '?' edges.
            Ok(after.saturating_sub(before) as u32)
        })
    }

    fn backfill_thurisaz(&self, graph: &str) -> StoreResult<u32> {
        self.with_graph_mut(graph, |kg| {
            let count = kg.backfill_to_thurisaz();
            Ok(count as u32)
        })
    }

    // ── Rumination support ──

    fn refutation_candidates(&self, graph: &str, limit: usize) -> StoreResult<Vec<(String, String, String, f64, f64)>> {
        self.with_graph(graph, |kg| Ok(kg.refutation_candidates(limit)))
    }

    fn synthesis_candidates(&self, graph: &str, limit: usize) -> StoreResult<Vec<(NodeId, NodeId, String, String, String)>> {
        self.with_graph(graph, |kg| {
            Ok(kg.synthesis_candidates(limit).into_iter()
                .map(|(a, c, b, r1, r2)| (NodeId(a), NodeId(c), b, r1, r2))
                .collect())
        })
    }

    fn undetermined_connections(&self, graph: &str, limit: usize) -> StoreResult<Vec<(String, String)>> {
        self.with_graph(graph, |kg| Ok(kg.undetermined_connections(limit)))
    }

    fn find_competitors(&self, graph: &str) -> StoreResult<Vec<CompetitorGroup>> {
        self.with_graph(graph, |kg| Ok(kg.find_competitors()))
    }

    fn contradiction_pairs(&self, graph: &str) -> StoreResult<Vec<ContradictionPair>> {
        self.with_graph(graph, |kg| Ok(kg.contradiction_pairs()))
    }

    fn uncertainty_stats(&self, graph: &str) -> StoreResult<UncertaintyStats> {
        self.with_graph(graph, |kg| Ok(kg.uncertainty_stats()))
    }

    fn cross_domain_patterns(&self, graph_a: &str, graph_b: &str, limit: usize) -> StoreResult<Vec<PatternMatch>> {
        // Need both graphs loaded. Use the with_graph helper for each.
        let mut graphs = self.graphs.write().map_err(|_| StoreError::Storage("lock poisoned".into()))?;
        Self::ensure_loaded(&mut graphs, graph_a, &self.memory_dir)?;
        Self::ensure_loaded(&mut graphs, graph_b, &self.memory_dir)?;
        let kg_a = graphs.get(graph_a).ok_or_else(|| StoreError::NotFound(graph_a.into()))?;
        let kg_b = graphs.get(graph_b).ok_or_else(|| StoreError::NotFound(graph_b.into()))?;
        Ok(kg_a.find_cross_domain_patterns(kg_b, limit))
    }

    // ── Git ──

    fn commit(&self, message: &str) -> StoreResult<String> {
        self.backend.commit(message)
    }

    fn history(&self, graph: &str, limit: usize) -> StoreResult<Vec<CommitInfo>> {
        self.backend.history(graph, limit)
    }

    // ── Thought branches ──

    fn begin_thought(&self) {
        self.backend.begin_thought();
    }

    fn end_thought(&self, message: &str) -> StoreResult<String> {
        self.backend.end_thought(message)
    }

    fn diff_since(&self, commit: &str) -> StoreResult<String> {
        self.backend.diff_since(commit)
    }

    fn diff_from_main(&self) -> StoreResult<String> {
        self.backend.diff_from_main()
    }

    fn create_thought_branch(&self, name: &str) -> StoreResult<String> {
        // Invalidate all cached graphs — we're switching branches.
        self.invalidate_all();
        self.backend.create_branch(name)
    }

    fn merge_thought_branch(&self, branch: &str) -> StoreResult<bool> {
        self.invalidate_all();
        let result = self.backend.merge_branch(branch)?;
        self.invalidate_all(); // Reload after merge.
        Ok(result)
    }

    fn abandon_thought_branch(&self, branch: &str) -> StoreResult<()> {
        self.invalidate_all();
        self.backend.abandon_branch(branch)
    }

    fn list_thought_branches(&self) -> StoreResult<Vec<String>> {
        self.backend.list_branches()
    }

    fn current_branch(&self) -> StoreResult<String> {
        self.backend.current_branch()
    }
}

// ── Helpers ────────────────────────────────────────────────────────

fn graph_path(memory_dir: &std::path::Path, name: &str) -> PathBuf {
    if name == "meta" || name.is_empty() {
        memory_dir.join("knowledge.json")
    } else {
        memory_dir.join("graphs").join(format!("{}.json", name))
    }
}

/// Find an edge by from/to labels and relation. Returns edge index and mutable reference.
fn find_edge_mut<'a>(
    kg: &'a mut KnowledgeGraph,
    from: &str, to: &str, relation: &str,
) -> StoreResult<(petgraph::graph::EdgeIndex, &'a mut knowledge::KnowledgeEdge)> {
    use petgraph::visit::EdgeRef;
    use petgraph::Direction;

    let from_idx = kg.find_by_label(from)
        .ok_or_else(|| StoreError::NotFound(format!("node '{}'", from)))?;
    let to_idx = kg.find_by_label(to)
        .ok_or_else(|| StoreError::NotFound(format!("node '{}'", to)))?;

    let eid = kg.graph.edges_directed(from_idx, Direction::Outgoing)
        .find(|e| e.target() == to_idx && e.weight().relation == relation)
        .map(|e| e.id())
        .ok_or_else(|| StoreError::NotFound(format!(
            "edge '{}' -> '{}' via '{}'", from, to, relation
        )))?;

    let edge = &mut kg.graph[eid];
    Ok((eid, edge))
}

/// Find an edge by from/to labels and relation (immutable).
fn find_edge<'a>(
    kg: &'a KnowledgeGraph,
    from: &str, to: &str, relation: &str,
) -> StoreResult<(petgraph::graph::EdgeIndex, &'a knowledge::KnowledgeEdge)> {
    use petgraph::visit::EdgeRef;
    use petgraph::Direction;

    let from_idx = kg.find_by_label(from)
        .ok_or_else(|| StoreError::NotFound(format!("node '{}'", from)))?;
    let to_idx = kg.find_by_label(to)
        .ok_or_else(|| StoreError::NotFound(format!("node '{}'", to)))?;

    let edge_ref = kg.graph.edges_directed(from_idx, Direction::Outgoing)
        .find(|e| e.target() == to_idx && e.weight().relation == relation)
        .ok_or_else(|| StoreError::NotFound(format!(
            "edge '{}' -> '{}' via '{}'", from, to, relation
        )))?;

    Ok((edge_ref.id(), edge_ref.weight()))
}

fn today() -> String {
    crate::dateutil::today_string()
}

/// Detect confirmation bias patterns in an edge's evidence history.
/// Returns a warning message if the pattern is suspicious.
fn detect_confirmation_bias(edge: &knowledge::KnowledgeEdge) -> Option<String> {
    let log = &edge.evidence_log;
    if log.len() < 3 { return None; }

    // Count evidence types.
    let mut positive = 0u32;
    let mut negative = 0u32;
    let mut _neutral = 0u32;
    let mut types_seen = std::collections::HashSet::new();

    for entry in log {
        types_seen.insert(std::mem::discriminant(&entry.evidence_type));
        if entry.bayes_factor > 1.0 {
            positive += 1;
        } else if entry.bayes_factor < 1.0 {
            negative += 1;
        } else {
            _neutral += 1;
        }
    }

    let total = log.len() as f64;
    let positive_rate = positive as f64 / total;

    let mut warnings = Vec::new();

    // All positive, no negative — suspicious.
    if positive >= 5 && negative == 0 {
        warnings.push(format!(
            "{} consecutive positive updates with zero negative — are you genuinely testing or just confirming?",
            positive
        ));
    }

    // Very high positive rate with low diversity.
    if positive_rate > 0.85 && types_seen.len() <= 2 && log.len() >= 5 {
        warnings.push(format!(
            "{:.0}% positive rate with only {} evidence type(s) — diversity of testing needed. \
             Try different kinds of refutation, not just the same approach.",
            positive_rate * 100.0, types_seen.len()
        ));
    }

    // Confidence above ceiling — the structural limit kicked in.
    if edge.confidence >= 0.69 && types_seen.len() <= 1 {
        warnings.push(
            "Confidence capped at 70% — only one type of evidence present. \
             To increase further, the edge needs different kinds of evidence \
             (e.g. corroboration AND refutation_survived, not just repeated corroborations).".into()
        );
    }

    if warnings.is_empty() {
        None
    } else {
        Some(format!("CONFIRMATION BIAS WARNING: {}", warnings.join(" | ")))
    }
}
