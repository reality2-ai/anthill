//! MCP (Model Context Protocol) server for knowledge graph operations.
//!
//! Exposes the knowledge graph API as structured tools that Claude Code
//! can call directly, instead of reading/writing raw JSON files.
//!
//! Protocol: JSON-RPC over stdio (MCP specification).
//! Launch: anthill --mcp-server --memory-dir <path>

use std::io::{BufRead, Write};
use std::path::PathBuf;

use crate::knowledge::*;
use petgraph::visit::EdgeRef;

/// Run the MCP server loop (stdio JSON-RPC).
pub fn run_mcp_server(memory_dir: PathBuf) {
    let graphs_dir = memory_dir.join("graphs");
    let meta_path = memory_dir.join("knowledge.json");

    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut reader = stdin.lock();
    let mut writer = stdout.lock();

    // Send server capabilities on startup.
    let init_response = serde_json::json!({
        "jsonrpc": "2.0",
        "result": {
            "protocolVersion": "2024-11-05",
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "anthill-graph", "version": env!("CARGO_PKG_VERSION") }
        }
    });

    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            break; // EOF
        }
        let line = line.trim();
        if line.is_empty() { continue; }

        let request: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let id = request.get("id").cloned();
        let method = request.get("method").and_then(|m| m.as_str()).unwrap_or("");

        let response = match method {
            "initialize" => {
                init_response.clone()
            }
            "tools/list" => {
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "tools": tool_definitions() }
                })
            }
            "tools/call" => {
                let tool_name = request.pointer("/params/name")
                    .and_then(|n| n.as_str()).unwrap_or("");
                let args = request.pointer("/params/arguments")
                    .cloned().unwrap_or(serde_json::json!({}));
                let result = handle_tool_call(tool_name, &args, &meta_path, &graphs_dir);
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "content": [{ "type": "text", "text": result }]
                    }
                })
            }
            "notifications/initialized" => continue, // no response needed
            _ => {
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32601, "message": format!("Unknown method: {}", method) }
                })
            }
        };

        let out = serde_json::to_string(&response).unwrap_or_default();
        let _ = writeln!(writer, "{}", out);
        let _ = writer.flush();
    }
}

fn tool_definitions() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({
            "name": "graph_query_about",
            "description": "Query the knowledge graph: 'what do I know about X?' Traverses from a node, returns connected subgraph with confidence levels.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "entity": { "type": "string", "description": "Entity label to query about" },
                    "depth": { "type": "integer", "description": "Traversal depth (default 2)", "default": 2 },
                    "graph": { "type": "string", "description": "Topic graph name (default: meta)", "default": "meta" }
                },
                "required": ["entity"]
            }
        }),
        serde_json::json!({
            "name": "graph_query_path",
            "description": "Find how two entities are connected. Returns shortest path(s) with cumulative confidence (product along the chain).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "from": { "type": "string", "description": "Source entity label" },
                    "to": { "type": "string", "description": "Target entity label" },
                    "graph": { "type": "string", "description": "Topic graph name (default: meta)", "default": "meta" }
                },
                "required": ["from", "to"]
            }
        }),
        serde_json::json!({
            "name": "graph_add_node",
            "description": "Add a node to the knowledge graph. Returns the node ID. Checks for duplicates.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "label": { "type": "string" },
                    "kind": { "type": "string", "enum": ["person","project","server","tool","concept","decision","event","fact"] },
                    "summary": { "type": "string" },
                    "tags": { "type": "array", "items": { "type": "string" } },
                    "graph": { "type": "string", "default": "meta" }
                },
                "required": ["label", "kind", "summary"]
            }
        }),
        serde_json::json!({
            "name": "graph_add_edge",
            "description": "Add a conjectural edge between two nodes. Auto-sets valid_from, source. Checks for duplicates.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "from": { "type": "string", "description": "Source entity label" },
                    "to": { "type": "string", "description": "Target entity label" },
                    "relation": { "type": "string" },
                    "basis": { "type": "string", "enum": ["observed","told","inferred","assumed"], "default": "told" },
                    "view": { "type": "string", "enum": ["semantic","temporal","causal","entity"], "default": "entity" },
                    "context": { "type": "string", "default": "" },
                    "graph": { "type": "string", "default": "meta" }
                },
                "required": ["from", "to", "relation"]
            }
        }),
        serde_json::json!({
            "name": "graph_strengthen",
            "description": "A conjecture survived refutation — increase its confidence. Increments tests and survived.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "from": { "type": "string" },
                    "to": { "type": "string" },
                    "relation": { "type": "string" },
                    "graph": { "type": "string", "default": "meta" }
                },
                "required": ["from", "to", "relation"]
            }
        }),
        serde_json::json!({
            "name": "graph_weaken",
            "description": "A conjecture was tested and evidence weakened it. Increments tests only (not survived).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "from": { "type": "string" },
                    "to": { "type": "string" },
                    "relation": { "type": "string" },
                    "graph": { "type": "string", "default": "meta" }
                },
                "required": ["from", "to", "relation"]
            }
        }),
        serde_json::json!({
            "name": "graph_contradict",
            "description": "Strong evidence directly contradicts a conjecture — sharp confidence penalty (×0.3).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "from": { "type": "string" },
                    "to": { "type": "string" },
                    "relation": { "type": "string" },
                    "graph": { "type": "string", "default": "meta" }
                },
                "required": ["from", "to", "relation"]
            }
        }),
        serde_json::json!({
            "name": "graph_query_uncertain",
            "description": "List all edges below a confidence threshold.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "threshold": { "type": "number", "default": 0.5 },
                    "graph": { "type": "string", "default": "meta" }
                }
            }
        }),
        serde_json::json!({
            "name": "graph_query_by_kind",
            "description": "List all nodes of a specific kind (person, project, tool, etc.).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "kind": { "type": "string", "enum": ["person","project","server","tool","concept","decision","event","fact"] },
                    "graph": { "type": "string", "default": "meta" }
                },
                "required": ["kind"]
            }
        }),
        serde_json::json!({
            "name": "graph_list_graphs",
            "description": "List all available knowledge graphs (meta + topics).",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        serde_json::json!({
            "name": "graph_list_orphans",
            "description": "List nodes connected only by '?' placeholder edges.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "graph": { "type": "string", "default": "meta" }
                }
            }
        }),
    ]
}

fn resolve_graph_path(graph_name: &str, meta_path: &std::path::Path, graphs_dir: &std::path::Path) -> PathBuf {
    if graph_name.is_empty() || graph_name == "meta" {
        meta_path.to_path_buf()
    } else {
        graphs_dir.join(format!("{}.json", graph_name))
    }
}

fn handle_tool_call(
    tool: &str,
    args: &serde_json::Value,
    meta_path: &std::path::Path,
    graphs_dir: &std::path::Path,
) -> String {
    let graph_name = args.get("graph").and_then(|g| g.as_str()).unwrap_or("meta");
    let path = resolve_graph_path(graph_name, meta_path, graphs_dir);

    match tool {
        "graph_query_about" => {
            let entity = args.get("entity").and_then(|e| e.as_str()).unwrap_or("");
            let depth = args.get("depth").and_then(|d| d.as_u64()).unwrap_or(2) as usize;
            let kg = KnowledgeGraph::load(&path);
            let result = kg.query_about(entity, depth);
            kg.render_query_result(&result, 8000)
        }
        "graph_query_path" => {
            let from = args.get("from").and_then(|f| f.as_str()).unwrap_or("");
            let to = args.get("to").and_then(|t| t.as_str()).unwrap_or("");
            let kg = KnowledgeGraph::load(&path);
            let result = kg.query_path(from, to, 5);
            if result.paths.is_empty() {
                format!("No path found between '{}' and '{}'", from, to)
            } else {
                kg.render_query_result(&result, 8000)
            }
        }
        "graph_add_node" => {
            let label = args.get("label").and_then(|l| l.as_str()).unwrap_or("");
            let kind_str = args.get("kind").and_then(|k| k.as_str()).unwrap_or("fact");
            let summary = args.get("summary").and_then(|s| s.as_str()).unwrap_or("");
            let tags: Vec<String> = args.get("tags")
                .and_then(|t| t.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default();

            if label.is_empty() { return "Error: label is required".into(); }

            let _ = std::fs::create_dir_all(graphs_dir);
            let mut kg = KnowledgeGraph::load(&path);

            // Check for duplicate.
            if kg.find_by_label(label).is_some() {
                return format!("Node '{}' already exists", label);
            }

            let kind = match kind_str {
                "person" => NodeKind::Person,
                "project" => NodeKind::Project,
                "server" => NodeKind::Server,
                "tool" => NodeKind::Tool,
                "concept" => NodeKind::Concept,
                "decision" => NodeKind::Decision,
                "event" => NodeKind::Event,
                _ => NodeKind::Fact,
            };

            let today = chrono_today();
            kg.graph.add_node(KnowledgeNode {
                label: label.into(),
                kind,
                summary: summary.into(),
                created: today.clone(),
                updated: today,
                tags,
                _extra: serde_json::Map::new(),
            });
            kg.rebuild_index();
            kg.save();
            format!("Added node '{}' ({})", label, kind_str)
        }
        "graph_add_edge" => {
            let from = args.get("from").and_then(|f| f.as_str()).unwrap_or("");
            let to = args.get("to").and_then(|t| t.as_str()).unwrap_or("");
            let relation = args.get("relation").and_then(|r| r.as_str()).unwrap_or("");
            let basis_str = args.get("basis").and_then(|b| b.as_str()).unwrap_or("told");
            let view_str = args.get("view").and_then(|v| v.as_str()).unwrap_or("entity");
            let context = args.get("context").and_then(|c| c.as_str()).unwrap_or("");

            let _ = std::fs::create_dir_all(graphs_dir);
            let mut kg = KnowledgeGraph::load(&path);

            let from_idx = match kg.find_by_label(from) {
                Some(idx) => idx,
                None => return format!("Node '{}' not found", from),
            };
            let to_idx = match kg.find_by_label(to) {
                Some(idx) => idx,
                None => return format!("Node '{}' not found", to),
            };

            // Check for duplicate.
            if kg.has_edge_between(from_idx, to_idx, relation) {
                return format!("Edge '{}' → {} → '{}' already exists", from, relation, to);
            }

            let basis = match basis_str {
                "observed" => Basis::Observed,
                "told" => Basis::Told,
                "inferred" => Basis::Inferred,
                _ => Basis::Assumed,
            };
            let view = match view_str {
                "semantic" => EdgeView::Semantic,
                "temporal" => EdgeView::Temporal,
                "causal" => EdgeView::Causal,
                _ => EdgeView::Entity,
            };
            let today = chrono_today();
            let confidence = basis.initial_confidence();

            kg.graph.add_edge(from_idx, to_idx, KnowledgeEdge {
                relation: relation.into(),
                context: context.into(),
                since: today.clone(),
                confidence,
                tests: 0,
                survived: 0,
                basis,
                last_tested: String::new(),
                importance: 0.5,
                references: 0,
                valid_from: today,
                valid_until: String::new(),
                view,
                source: "mcp".into(),
            });
            kg.save();
            format!("Added edge: '{}' → {} → '{}' [confidence: {:.0}%]", from, relation, to, confidence * 100.0)
        }
        "graph_strengthen" | "graph_weaken" | "graph_contradict" => {
            let from = args.get("from").and_then(|f| f.as_str()).unwrap_or("");
            let to = args.get("to").and_then(|t| t.as_str()).unwrap_or("");
            let relation = args.get("relation").and_then(|r| r.as_str()).unwrap_or("");

            let mut kg = KnowledgeGraph::load(&path);
            let from_idx = match kg.find_by_label(from) {
                Some(idx) => idx,
                None => return format!("Node '{}' not found", from),
            };
            let to_idx = match kg.find_by_label(to) {
                Some(idx) => idx,
                None => return format!("Node '{}' not found", to),
            };

            // Find the edge.
            let edge_idx = kg.graph.edges_directed(from_idx, petgraph::Direction::Outgoing)
                .find(|e| e.target() == to_idx && e.weight().relation == relation)
                .map(|e| e.id());

            match edge_idx {
                Some(eid) => {
                    let today = chrono_today();
                    let before = kg.graph[eid].confidence;
                    match tool {
                        "graph_strengthen" => kg.graph[eid].strengthen(&today),
                        "graph_weaken" => kg.graph[eid].weaken(&today),
                        "graph_contradict" => kg.graph[eid].contradict(&today),
                        _ => {}
                    }
                    let after = kg.graph[eid].confidence;
                    let tests = kg.graph[eid].tests;
                    let survived = kg.graph[eid].survived;
                    kg.save();
                    format!("'{}' → {} → '{}': confidence {:.0}% → {:.0}% (tests: {}, survived: {})",
                        from, relation, to, before * 100.0, after * 100.0, tests, survived)
                }
                None => format!("Edge '{}' → {} → '{}' not found", from, relation, to),
            }
        }
        "graph_query_uncertain" => {
            let threshold = args.get("threshold").and_then(|t| t.as_f64()).unwrap_or(0.5);
            let kg = KnowledgeGraph::load(&path);
            let result = kg.query_uncertain(threshold);
            if result.edges.is_empty() {
                format!("No edges below {:.0}% confidence", threshold * 100.0)
            } else {
                kg.render_query_result(&result, 8000)
            }
        }
        "graph_query_by_kind" => {
            let kind_str = args.get("kind").and_then(|k| k.as_str()).unwrap_or("fact");
            let kind = match kind_str {
                "person" => NodeKind::Person,
                "project" => NodeKind::Project,
                "server" => NodeKind::Server,
                "tool" => NodeKind::Tool,
                "concept" => NodeKind::Concept,
                "decision" => NodeKind::Decision,
                "event" => NodeKind::Event,
                _ => NodeKind::Fact,
            };
            let kg = KnowledgeGraph::load(&path);
            let result = kg.query_by_kind(&kind);
            if result.nodes.is_empty() {
                format!("No {} nodes found", kind_str)
            } else {
                kg.render_query_result(&result, 8000)
            }
        }
        "graph_list_graphs" => {
            let mut graphs = vec!["meta".to_string()];
            if graphs_dir.exists() {
                if let Ok(entries) = std::fs::read_dir(graphs_dir) {
                    for entry in entries.flatten() {
                        let p = entry.path();
                        if p.extension().map(|e| e == "json").unwrap_or(false) {
                            if let Some(stem) = p.file_stem() {
                                let name = stem.to_string_lossy().to_string();
                                let kg = KnowledgeGraph::load(&p);
                                graphs.push(format!("{} ({} nodes)", name, kg.node_count()));
                            }
                        }
                    }
                }
            }
            graphs.join("\n")
        }
        "graph_list_orphans" => {
            let kg = KnowledgeGraph::load(&path);
            let orphans: Vec<String> = kg.graph.node_indices()
                .filter(|&idx| {
                    let edges: Vec<_> = kg.graph.edges(idx).collect();
                    edges.is_empty() || edges.iter().all(|e| e.weight().relation == "?")
                })
                .map(|idx| {
                    let n = &kg.graph[idx];
                    format!("{} ({}): {}", n.label, n.kind, n.summary)
                })
                .collect();
            if orphans.is_empty() {
                "No orphan nodes".into()
            } else {
                format!("{} orphan(s):\n{}", orphans.len(), orphans.join("\n"))
            }
        }
        _ => format!("Unknown tool: {}", tool),
    }
}

fn chrono_today() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days = now / 86400;
    // Simple date calculation (approximate — good enough for YYYY-MM-DD).
    let y = 1970 + (days * 400 / 146097);
    let remaining = days - (y - 1970) * 365 - ((y - 1969) / 4) + ((y - 1901) / 100) - ((y - 1601) / 400);
    let month_days = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut m = 0;
    let mut d = remaining;
    for (i, &md) in month_days.iter().enumerate() {
        let md = if i == 1 && (y % 4 == 0 && (y % 100 != 0 || y % 400 == 0)) { 29 } else { md };
        if d < md { m = i + 1; break; }
        d -= md;
    }
    if m == 0 { m = 12; }
    format!("{:04}-{:02}-{:02}", y, m, d + 1)
}
