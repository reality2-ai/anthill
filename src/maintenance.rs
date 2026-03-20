//! Background knowledge graph maintenance daemon.
//!
//! Runs periodically per ANT, independent of user requests:
//! - Consolidate each topic graph (dedup, merge, collapse)
//! - Cross-link topic graphs in the meta-graph
//! - Migrate misplaced nodes from meta-graph to topic graphs
//! - Decay untested conjectures
//! - Detect contradictions and clusters
//! - Log all changes

use std::path::PathBuf;
use std::time::Duration;

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
}

/// Run the background maintenance loop. Spawned as a tokio task per ANT.
pub async fn maintenance_loop(config: MaintenanceConfig) {
    let mut last_consolidation = std::time::Instant::now();
    let mut last_cross_link = std::time::Instant::now();

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
    }
}

/// Consolidate all topic graphs and the meta-graph.
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
                    if report.nodes_merged > 0 || report.edges_merged > 0 {
                        kg.save();
                        log::info!("[{}] Topic '{}' consolidated: {} merged, {} edges merged",
                            config.ant_name, topic, report.nodes_merged, report.edges_merged);
                    }
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
        log::info!("[{}] Cross-linked {} topic pairs in meta-graph", config.ant_name, added);
    }
}
