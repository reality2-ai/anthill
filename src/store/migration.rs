//! Migration tool — fix and clean up existing knowledge graphs.
//!
//! Handles:
//! - Invalid enum values (e.g. "basis": "research") → fixed to valid values
//! - Non-graph JSON files (research notes) → converted to proper graphs or removed
//! - Corrupted/tmp files → cleaned up
//! - Backfill Thurisaz format on legacy edges
//! - Move stray graph files into graphs/ directory
//!
//! Run: anthill --migrate-graphs <memory-dir>
//! Or:  anthill --migrate-graphs ~/.config/anthill/ants/<name>/working/memory

use std::path::Path;

/// Migrate all graphs in a directory tree.
/// If given an ants/ parent directory, recurse into each ANT's memory.
pub fn migrate_all(dir: &Path) {
    println!("Migrating knowledge graphs in {}...", dir.display());

    // Check if this is the ants/ parent directory.
    let ants_dir = dir.join("ants");
    if ants_dir.exists() {
        // Recurse into each ANT.
        if let Ok(entries) = std::fs::read_dir(&ants_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let memory_dir = path.join("working").join("memory");
                    if memory_dir.exists() {
                        let name = path.file_name()
                            .map(|f| f.to_string_lossy().to_string())
                            .unwrap_or_default();
                        println!("\n=== ANT: {} ===", name);
                        migrate_memory_dir(&memory_dir);
                    }
                }
            }
        }
        return;
    }

    // Check if this is a memory/ directory directly.
    if dir.join("knowledge.json").exists() || dir.join("graphs").exists() {
        migrate_memory_dir(dir);
        return;
    }

    // Check if this is a working/ directory.
    let memory_sub = dir.join("memory");
    if memory_sub.exists() {
        migrate_memory_dir(&memory_sub);
        return;
    }

    println!("Could not find knowledge graphs in {}.", dir.display());
    println!("Expected: <path>/memory/knowledge.json or <path>/ants/*/working/memory/");
}

/// Migrate a single ANT's memory directory.
fn migrate_memory_dir(memory_dir: &Path) {
    let graphs_dir = memory_dir.join("graphs");
    let _ = std::fs::create_dir_all(&graphs_dir);

    let mut stats = MigrationStats::default();

    // 1. Clean up corrupted and tmp files.
    clean_temp_files(memory_dir, &mut stats);
    clean_temp_files(&graphs_dir, &mut stats);

    // 2. Move stray graph files from memory/ to memory/graphs/.
    move_stray_graphs(memory_dir, &graphs_dir, &mut stats);

    // 3. Fix or remove non-graph JSON files in graphs/.
    fix_non_graph_files(&graphs_dir, &mut stats);

    // 4. Load, validate, backfill, and re-save each graph.
    fix_graph_files(&graphs_dir, &mut stats);

    // 5. Fix the meta-graph too.
    let meta_path = memory_dir.join("knowledge.json");
    if meta_path.exists() {
        fix_single_graph(&meta_path, "meta", &mut stats);
    }

    // Report.
    println!("  Cleaned: {} temp/corrupted files", stats.cleaned);
    println!("  Moved:   {} stray files → graphs/", stats.moved);
    println!("  Fixed:   {} graphs with invalid data", stats.fixed);
    println!("  Removed: {} non-graph files", stats.removed);
    println!("  OK:      {} graphs already valid", stats.ok);
}

#[derive(Default)]
struct MigrationStats {
    cleaned: u32,
    moved: u32,
    fixed: u32,
    removed: u32,
    ok: u32,
}

fn clean_temp_files(dir: &Path, stats: &mut MigrationStats) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() { continue; }
        let name = path.file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default();
        if name.ends_with(".corrupted") || name.ends_with(".json.tmp") || name.ends_with(".tmp") {
            if std::fs::remove_file(&path).is_ok() {
                println!("  Cleaned: {}", name);
                stats.cleaned += 1;
            }
        }
    }
}

fn move_stray_graphs(memory_dir: &Path, graphs_dir: &Path, stats: &mut MigrationStats) {
    let skip = [
        "knowledge.json", "knowledge-archive.json",
        "episodes.json", "embeddings.json",
        "reputation.json", "questions.json",
        "rumination_log.json",
    ];

    let entries = match std::fs::read_dir(memory_dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() { continue; }
        let name = path.file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default();

        if !name.ends_with(".json") { continue; }
        if skip.iter().any(|&s| name == s) { continue; }
        if name.starts_with('-') || name.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) {
            continue;
        }

        // Check if it's a graph.
        let is_graph = std::fs::read_to_string(&path)
            .map(|c| c.contains("\"nodes\"") && c.contains("\"edges\""))
            .unwrap_or(false);
        if !is_graph { continue; }

        let dest = graphs_dir.join(&name);
        if !dest.exists() {
            if std::fs::rename(&path, &dest).is_ok() {
                println!("  Moved: {} → graphs/", name);
                stats.moved += 1;
            }
        }
    }
}

fn fix_non_graph_files(graphs_dir: &Path, stats: &mut MigrationStats) {
    let entries = match std::fs::read_dir(graphs_dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() { continue; }
        let name = path.file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default();
        if !name.ends_with(".json") { continue; }
        if name.contains("-archive") { continue; }

        // Check if it has nodes.
        let contents = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        if !contents.contains("\"nodes\"") {
            // Try to convert section-based files to a graph.
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&contents) {
                if let Some(converted) = try_convert_to_graph(&value, &name) {
                    if std::fs::write(&path, converted).is_ok() {
                        println!("  Converted: {} (sections → graph)", name);
                        stats.fixed += 1;
                        continue;
                    }
                }
            }
            // Can't convert — move to a .bak file so it doesn't show up.
            let bak = path.with_extension("json.bak");
            if std::fs::rename(&path, &bak).is_ok() {
                println!("  Backed up: {} (not a graph, moved to .bak)", name);
                stats.removed += 1;
            }
        }
    }
}

/// Try to convert a section-based JSON file into a proper knowledge graph.
/// Returns the JSON string if conversion succeeds.
fn try_convert_to_graph(value: &serde_json::Value, filename: &str) -> Option<String> {
    // Look for a structure like { "meta": {...}, "1_section": { "summary": "...", "data_points": [...] } }
    let obj = value.as_object()?;

    let topic = filename.strip_suffix(".json").unwrap_or(filename);
    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    // Create a hub node for the topic.
    nodes.push(serde_json::json!({
        "label": topic,
        "kind": "concept",
        "summary": obj.get("meta")
            .and_then(|m| m.get("title"))
            .and_then(|t| t.as_str())
            .unwrap_or(topic),
        "created": obj.get("meta").and_then(|m| m.get("created")).and_then(|c| c.as_str()).unwrap_or(""),
        "updated": obj.get("meta").and_then(|m| m.get("updated")).and_then(|u| u.as_str()).unwrap_or(""),
        "tags": ["topic", "converted"]
    }));

    // Convert each section into a node.
    let mut idx = 1;
    for (key, section) in obj {
        if key == "meta" { continue; }

        let summary = section.get("summary")
            .and_then(|s| s.as_str())
            .or_else(|| {
                // Try to extract from data_points or other fields.
                if let Some(arr) = section.as_array() {
                    arr.first().and_then(|v| v.as_str())
                } else if let Some(s) = section.as_str() {
                    Some(s)
                } else {
                    None
                }
            })
            .unwrap_or("");

        if summary.is_empty() && !section.is_object() { continue; }

        let label = key.trim_start_matches(|c: char| c.is_ascii_digit() || c == '_')
            .replace('_', " ");
        let label = if label.is_empty() { key.clone() } else { label };

        nodes.push(serde_json::json!({
            "label": label,
            "kind": "concept",
            "summary": if summary.len() > 300 { &summary[..300] } else { summary },
            "created": "",
            "updated": "",
            "tags": ["converted"]
        }));

        // Edge from hub to this section node.
        edges.push(serde_json::json!([0, idx, {
            "relation": "covers",
            "context": format!("Section from converted research notes"),
            "since": "",
            "confidence": 0.5,
            "log_odds": 0.0,
            "tests": 0,
            "survived": 0,
            "basis": "inferred",
            "last_tested": "",
            "decay_category": "fact",
            "source_id": "migration:convert",
            "evidence_log": [],
            "justificatory_chain": [],
            "refutation_log": [],
            "valid_from": "",
            "valid_until": "",
            "view": "semantic",
            "source": "migration: converted from section-based notes",
            "importance": 0.5,
            "references": 0,
            "beneficial_impact": 0.0,
            "corroboration_strength": 0.0,
            "competition_group": ""
        }]));

        idx += 1;
    }

    if nodes.len() < 2 { return None; } // Not enough content to convert.

    let graph = serde_json::json!({
        "nodes": nodes,
        "edges": edges,
    });

    serde_json::to_string_pretty(&graph).ok()
}

fn fix_graph_files(graphs_dir: &Path, stats: &mut MigrationStats) {
    let entries = match std::fs::read_dir(graphs_dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() { continue; }
        let name = path.file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default();
        if !name.ends_with(".json") || name.contains("-archive") || name.ends_with(".bak") {
            continue;
        }

        let topic = path.file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();

        fix_single_graph(&path, &topic, stats);
    }
}

fn fix_single_graph(path: &Path, topic: &str, stats: &mut MigrationStats) {
    let mut kg = crate::knowledge::KnowledgeGraph::load(path);
    if kg.node_count() == 0 { return; }

    let mut changed = false;

    // Backfill refutation logs and Thurisaz format.
    kg.backfill_refutation_logs();
    let thurisaz_count = kg.backfill_to_thurisaz();
    if thurisaz_count > 0 { changed = true; }

    // Consolidate: dedup, merge, chain collapse.
    let report = kg.consolidate();
    if report.nodes_merged > 0 || report.edges_merged > 0 { changed = true; }

    // Link orphans.
    kg.link_orphans(topic);

    // Compute corroboration strength.
    kg.compute_corroboration_strength();

    // Always re-save to clean up any lenient-parsed data.
    kg.save();

    if changed {
        println!("  Fixed: {} ({} thurisaz, {} merged, {} edges merged)",
            topic, thurisaz_count, report.nodes_merged, report.edges_merged);
        stats.fixed += 1;
    } else {
        stats.ok += 1;
    }
}
