//! MCP (Model Context Protocol) server for knowledge graph operations.
//!
//! Exposes the knowledge graph API as structured tools that Claude Code
//! can call directly. All writes go through the validated KnowledgeStore
//! trait — the AI cannot write invalid data.
//!
//! Protocol: JSON-RPC over stdio (MCP specification).
//! Launch: anthill --mcp-server --memory-dir <path>

use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::Arc;

use crate::reputation::{ReputationRegistry, SourceCategory};
use crate::store::live::LiveKnowledgeStore;
use crate::store::{KnowledgeStore, ValidatedEdge, ValidatedEvidence, ValidatedNode};

/// Run the MCP server loop (stdio JSON-RPC).
pub fn run_mcp_server(memory_dir: PathBuf) {
    let store = Arc::new(LiveKnowledgeStore::new(memory_dir.clone()));

    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut reader = stdin.lock();
    let mut writer = stdout.lock();

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
            break;
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let request: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let id = request.get("id").cloned();
        let method = request.get("method").and_then(|m| m.as_str()).unwrap_or("");

        let response = match method {
            "initialize" => init_response.clone(),
            "tools/list" => {
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "tools": tool_definitions() }
                })
            }
            "tools/call" => {
                let tool_name = request
                    .pointer("/params/name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("");
                let args = request
                    .pointer("/params/arguments")
                    .cloned()
                    .unwrap_or(serde_json::json!({}));
                let result = handle_tool_call(tool_name, &args, &store, &memory_dir);
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "content": [{ "type": "text", "text": result }]
                    }
                })
            }
            "notifications/initialized" => continue,
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

pub fn tool_definitions() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({
            "name": "graph_query_about",
            "description": "Query the knowledge graph: 'what do I know about X?' Traverses from a node, returns connected subgraph with confidence levels.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "entity": { "type": "string", "description": "Node label or keyword to search for" },
                    "depth": { "type": "integer", "description": "How many hops to traverse (default: 2)", "default": 2 },
                    "graph": { "type": "string", "description": "Graph name (default: meta). Use a topic name like 'anthill', 'infrastructure'." }
                },
                "required": ["entity"]
            }
        }),
        serde_json::json!({
            "name": "graph_query_path",
            "description": "Find paths between two entities in the knowledge graph, showing confidence along the path.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "from": { "type": "string", "description": "Starting node label" },
                    "to": { "type": "string", "description": "Target node label" },
                    "graph": { "type": "string" }
                },
                "required": ["from", "to"]
            }
        }),
        serde_json::json!({
            "name": "graph_add_node",
            "description": "Add a new node to a knowledge graph. The node is validated before insertion.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "label": { "type": "string", "description": "Human-readable name" },
                    "kind": { "type": "string", "enum": ["person", "project", "server", "tool", "concept", "decision", "event", "fact", "theory", "mechanism", "principle", "constraint", "problem", "claim", "open_question", "implementation", "entity", "spec", "repo", "platform", "framework"] },
                    "summary": { "type": "string" },
                    "tags": { "type": "array", "items": { "type": "string" } },
                    "graph": { "type": "string" }
                },
                "required": ["label", "kind", "summary"]
            }
        }),
        serde_json::json!({
            "name": "graph_add_edge",
            "description": "Add a conjectural relationship between two nodes. Validated: from/to must exist, basis must be valid.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "from": { "type": "string", "description": "Source node label" },
                    "to": { "type": "string", "description": "Target node label" },
                    "relation": { "type": "string", "description": "Relationship type (e.g. 'works_on', 'deployed_on')" },
                    "context": { "type": "string", "description": "Brief description" },
                    "basis": { "type": "string", "enum": ["observed", "told", "inferred", "assumed"] },
                    "view": { "type": "string", "enum": ["semantic", "temporal", "causal", "entity"] },
                    "source": { "type": "string", "description": "Provenance (e.g. 'document:README.md', 'user:roy')" },
                    "beneficial_impact": { "type": "number", "description": "Impact on people/planet (-1.0 to 1.0, default 0)" },
                    "graph": { "type": "string" }
                },
                "required": ["from", "to", "relation", "basis"]
            }
        }),
        serde_json::json!({
            "name": "graph_update_evidence",
            "description": "Update an edge with typed evidence (primary Thurisaz update path). Validated evidence types only.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "from": { "type": "string" },
                    "to": { "type": "string" },
                    "relation": { "type": "string" },
                    "evidence_type": { "type": "string", "enum": ["corroboration", "contradiction", "refutation_survived", "refutation_failed", "human_attestation", "consistency", "inconsistency", "synthesis", "competition_won", "competition_lost", "pattern_transfer", "inconsequential_search"] },
                    "test": { "type": "string", "description": "What was tested or observed" },
                    "detail": { "type": "string", "description": "The evidence itself" },
                    "source_id": { "type": "string", "description": "e.g. 'document:README.md', 'user:roy', 'ai:inference'" },
                    "graph": { "type": "string" }
                },
                "required": ["from", "to", "relation", "evidence_type", "test"]
            }
        }),
        serde_json::json!({
            "name": "graph_strengthen",
            "description": "Strengthen an edge (refutation survived — actively tried to disprove, claim held). BF=2.5.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "from": { "type": "string" }, "to": { "type": "string" },
                    "relation": { "type": "string" },
                    "test": { "type": "string", "description": "What refutation was attempted" },
                    "evidence": { "type": "string", "description": "What evidence was considered" },
                    "graph": { "type": "string" }
                },
                "required": ["from", "to", "relation"]
            }
        }),
        serde_json::json!({
            "name": "graph_weaken",
            "description": "Weaken an edge (inconsistency found). BF=0.4.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "from": { "type": "string" }, "to": { "type": "string" },
                    "relation": { "type": "string" },
                    "test": { "type": "string" }, "evidence": { "type": "string" },
                    "graph": { "type": "string" }
                },
                "required": ["from", "to", "relation"]
            }
        }),
        serde_json::json!({
            "name": "graph_contradict",
            "description": "Contradict an edge (refutation failed — direct contradiction). BF=0.1.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "from": { "type": "string" }, "to": { "type": "string" },
                    "relation": { "type": "string" },
                    "test": { "type": "string" }, "evidence": { "type": "string" },
                    "graph": { "type": "string" }
                },
                "required": ["from", "to", "relation"]
            }
        }),
        serde_json::json!({
            "name": "graph_add_citation",
            "description": "Add a citation/reference to an existing edge. Every edge should have at least one citation for provenance.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "from": { "type": "string", "description": "Source node label" },
                    "to": { "type": "string", "description": "Target node label" },
                    "relation": { "type": "string", "description": "Edge relation" },
                    "url": { "type": "string", "description": "URL of the source (if web-based)" },
                    "title": { "type": "string", "description": "Title or short description of the source" },
                    "author": { "type": "string", "description": "Author(s) if known" },
                    "date": { "type": "string", "description": "Publication date or year" },
                    "ref_type": { "type": "string", "enum": ["peer_reviewed", "official_report", "book", "news", "blog", "website", "personal", "ant_knowledge", "ai_inference"], "description": "Type of source" },
                    "quality": { "type": "number", "description": "Quality score 0.0-1.0 (default based on ref_type)" },
                    "graph": { "type": "string" }
                },
                "required": ["from", "to", "relation", "ref_type"]
            }
        }),
        serde_json::json!({
            "name": "graph_query_uncertain",
            "description": "List edges below a confidence threshold.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "threshold": { "type": "number", "description": "Confidence threshold (default 0.5)" },
                    "graph": { "type": "string" }
                }
            }
        }),
        serde_json::json!({
            "name": "graph_query_by_kind",
            "description": "List all nodes of a specific kind.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "kind": { "type": "string", "enum": ["person", "project", "server", "tool", "concept", "decision", "event", "fact"] },
                    "graph": { "type": "string" }
                },
                "required": ["kind"]
            }
        }),
        serde_json::json!({
            "name": "graph_list_graphs",
            "description": "List all available knowledge graphs with node counts.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        serde_json::json!({
            "name": "graph_list_orphans",
            "description": "List nodes with no meaningful connections (only '?' edges).",
            "inputSchema": {
                "type": "object",
                "properties": { "graph": { "type": "string" } }
            }
        }),
        serde_json::json!({
            "name": "graph_query_justification",
            "description": "Show the full justificatory chain and evidence log for an edge.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "from": { "type": "string" }, "to": { "type": "string" },
                    "relation": { "type": "string" }, "graph": { "type": "string" }
                },
                "required": ["from", "to", "relation"]
            }
        }),
        serde_json::json!({
            "name": "graph_query_reputation",
            "description": "Query source reputation scores.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "source_id": { "type": "string", "description": "Source ID to query (empty = list all)" }
                },
                "required": ["source_id"]
            }
        }),
        // Colony tools — inter-ANT communication.
        serde_json::json!({
            "name": "talk_to_ant",
            "description": "Send a message to another ANT that fires up their AI worker to think and respond. Use this to have a real conversation — the other ANT reasons with their own knowledge and expertise. The response arrives as a follow-up in your conversation.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "ant": { "type": "string", "description": "Name of the ANT to talk to" },
                    "message": { "type": "string", "description": "Your message — what you want to discuss or ask" }
                },
                "required": ["ant", "message"]
            }
        }),
        serde_json::json!({
            "name": "query_ant",
            "description": "Quick read-only peek at another ANT's existing knowledge. For a real conversation where they THINK about your question, use talk_to_ant instead.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "ant": { "type": "string", "description": "Name of the ANT to ask (use list_colony_ants to see available)" },
                    "entity": { "type": "string", "description": "Entity or topic to ask about" },
                    "depth": { "type": "integer", "description": "How many hops to traverse (default: 2)", "default": 2 }
                },
                "required": ["ant", "entity"]
            }
        }),
        serde_json::json!({
            "name": "list_colony_ants",
            "description": "List all ANTs in the colony with their areas of expertise (topic graphs). Use this to discover which ANT to ask about a topic.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        // Git cognitive tools — thought history and branches.
        serde_json::json!({
            "name": "thought_history",
            "description": "Search your thinking history. Each commit is an atomic thought. Use 'since' to see what changed since a specific commit hash.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "graph": { "type": "string", "description": "Graph name (optional — default: all)" },
                    "limit": { "type": "integer", "description": "Max entries to return (default: 10)", "default": 10 },
                    "since": { "type": "string", "description": "Commit hash — show what changed since then" }
                }
            }
        }),
        serde_json::json!({
            "name": "thought_branch",
            "description": "Create a thought branch for speculative exploration. Work freely on the branch without affecting your main knowledge. Merge if the ideas survive evaluation, abandon if they don't.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["create", "merge", "abandon", "list", "current"], "description": "What to do" },
                    "name": { "type": "string", "description": "Branch name (for create/merge/abandon)" }
                },
                "required": ["action"]
            }
        }),
    ]
}

pub fn handle_tool_call(
    tool: &str,
    args: &serde_json::Value,
    store: &LiveKnowledgeStore,
    memory_dir: &std::path::Path,
) -> String {
    let graph = args.get("graph").and_then(|g| g.as_str()).unwrap_or("meta");

    match tool {
        "graph_query_about" => {
            let entity = args.get("entity").and_then(|e| e.as_str()).unwrap_or("");
            let depth = args.get("depth").and_then(|d| d.as_u64()).unwrap_or(2) as usize;
            match store.query_about(graph, entity, depth) {
                Ok(result) => store
                    .with_graph_render(graph, &result)
                    .unwrap_or_else(|| format!("Found results for '{}'", entity)),
                Err(e) => format!("Error: {}", e),
            }
        }
        "graph_query_path" => {
            let from = args.get("from").and_then(|f| f.as_str()).unwrap_or("");
            let to = args.get("to").and_then(|t| t.as_str()).unwrap_or("");
            match store.query_path(graph, from, to, 5) {
                Ok(result) if result.paths.is_empty() => {
                    format!("No path found between '{}' and '{}'", from, to)
                }
                Ok(result) => store
                    .with_graph_render(graph, &result)
                    .unwrap_or_else(|| format!("Path found from '{}' to '{}'", from, to)),
                Err(e) => format!("Error: {}", e),
            }
        }
        "graph_add_node" => {
            let label = args.get("label").and_then(|l| l.as_str()).unwrap_or("");
            let kind = args.get("kind").and_then(|k| k.as_str()).unwrap_or("fact");
            let summary = args.get("summary").and_then(|s| s.as_str()).unwrap_or("");
            let tags: Vec<String> = args
                .get("tags")
                .and_then(|t| t.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();

            match ValidatedNode::new(label, kind, summary, tags) {
                Ok(node) => match store.add_node(graph, node) {
                    Ok(_) => format!("Added node '{}' ({})", label, kind),
                    Err(e) => format!("Error: {}", e),
                },
                Err(e) => format!("Error: {}", e),
            }
        }
        "graph_add_edge" => {
            let from = args.get("from").and_then(|f| f.as_str()).unwrap_or("");
            let to = args.get("to").and_then(|t| t.as_str()).unwrap_or("");
            let relation = args.get("relation").and_then(|r| r.as_str()).unwrap_or("");
            let context = args.get("context").and_then(|c| c.as_str()).unwrap_or("");
            let basis = args.get("basis").and_then(|b| b.as_str()).unwrap_or("told");
            let view = args
                .get("view")
                .and_then(|v| v.as_str())
                .unwrap_or("entity");
            let source = args.get("source").and_then(|s| s.as_str()).unwrap_or("mcp");
            let impact = args
                .get("beneficial_impact")
                .and_then(|b| b.as_f64())
                .unwrap_or(0.0);

            match ValidatedEdge::new(from, to, relation, context, basis, view, source, impact) {
                Ok(edge) => match store.add_edge(graph, edge) {
                    Ok(_) => format!("Added edge: '{}' → {} → '{}'", from, relation, to),
                    Err(e) => format!("Error: {}", e),
                },
                Err(e) => format!("Error: {}", e),
            }
        }
        "graph_update_evidence" => {
            let from = args.get("from").and_then(|f| f.as_str()).unwrap_or("");
            let to = args.get("to").and_then(|t| t.as_str()).unwrap_or("");
            let relation = args.get("relation").and_then(|r| r.as_str()).unwrap_or("");
            let evidence_type = args
                .get("evidence_type")
                .and_then(|e| e.as_str())
                .unwrap_or("");
            let test = args.get("test").and_then(|t| t.as_str()).unwrap_or("");
            let detail = args.get("detail").and_then(|d| d.as_str()).unwrap_or("");
            let source_id = args
                .get("source_id")
                .and_then(|s| s.as_str())
                .unwrap_or("ai:inference");

            // Get source reputation.
            let rep_path = memory_dir.join("reputation.json");
            let mut registry = ReputationRegistry::load(&rep_path);
            let category = if source_id.starts_with("user:") {
                SourceCategory::User
            } else if source_id.starts_with("document:") {
                SourceCategory::Document
            } else if source_id.starts_with("ai:") {
                SourceCategory::AiInference
            } else if source_id.starts_with("ant:") {
                SourceCategory::Ant
            } else {
                SourceCategory::Unknown
            };
            let reputation = registry.get_reputation(source_id, category);

            match ValidatedEvidence::new(evidence_type, test, detail, source_id, reputation) {
                Ok(evidence) => {
                    match store.update_evidence(graph, from, to, relation, evidence) {
                        Ok(update) => {
                            // Update source reputation.
                            if update.confidence_after > update.confidence_before {
                                registry.record_corroboration(source_id);
                            } else if update.confidence_after < update.confidence_before {
                                registry.record_contradiction(source_id);
                            }
                            registry.save(&rep_path);
                            format!("'{}' → {} → '{}': confidence {:.0}% → {:.0}% (BF={:.2}, rep={:.2})",
                            from, relation, to,
                            update.confidence_before * 100.0, update.confidence_after * 100.0,
                            update.bayes_factor, reputation)
                        }
                        Err(e) => format!("Error: {}", e),
                    }
                }
                Err(e) => format!("Error: {}", e),
            }
        }
        "graph_strengthen" => {
            let from = args.get("from").and_then(|f| f.as_str()).unwrap_or("");
            let to = args.get("to").and_then(|t| t.as_str()).unwrap_or("");
            let relation = args.get("relation").and_then(|r| r.as_str()).unwrap_or("");
            let test = args.get("test").and_then(|t| t.as_str()).unwrap_or("");
            let evidence = args.get("evidence").and_then(|e| e.as_str()).unwrap_or("");

            match store.strengthen(graph, from, to, relation, test, evidence) {
                Ok(u) => format!(
                    "'{}' → {} → '{}': {:.0}% → {:.0}%",
                    from,
                    relation,
                    to,
                    u.confidence_before * 100.0,
                    u.confidence_after * 100.0
                ),
                Err(e) => format!("Error: {}", e),
            }
        }
        "graph_weaken" => {
            let from = args.get("from").and_then(|f| f.as_str()).unwrap_or("");
            let to = args.get("to").and_then(|t| t.as_str()).unwrap_or("");
            let relation = args.get("relation").and_then(|r| r.as_str()).unwrap_or("");
            let test = args.get("test").and_then(|t| t.as_str()).unwrap_or("");
            let evidence = args.get("evidence").and_then(|e| e.as_str()).unwrap_or("");

            match store.weaken(graph, from, to, relation, test, evidence) {
                Ok(u) => format!(
                    "'{}' → {} → '{}': {:.0}% → {:.0}%",
                    from,
                    relation,
                    to,
                    u.confidence_before * 100.0,
                    u.confidence_after * 100.0
                ),
                Err(e) => format!("Error: {}", e),
            }
        }
        "graph_contradict" => {
            let from = args.get("from").and_then(|f| f.as_str()).unwrap_or("");
            let to = args.get("to").and_then(|t| t.as_str()).unwrap_or("");
            let relation = args.get("relation").and_then(|r| r.as_str()).unwrap_or("");
            let test = args.get("test").and_then(|t| t.as_str()).unwrap_or("");
            let evidence = args.get("evidence").and_then(|e| e.as_str()).unwrap_or("");

            match store.contradict(graph, from, to, relation, test, evidence) {
                Ok(u) => format!(
                    "'{}' → {} → '{}': {:.0}% → {:.0}%",
                    from,
                    relation,
                    to,
                    u.confidence_before * 100.0,
                    u.confidence_after * 100.0
                ),
                Err(e) => format!("Error: {}", e),
            }
        }
        "graph_add_citation" => {
            let from = args.get("from").and_then(|f| f.as_str()).unwrap_or("");
            let to = args.get("to").and_then(|t| t.as_str()).unwrap_or("");
            let relation = args.get("relation").and_then(|r| r.as_str()).unwrap_or("");
            let url = args.get("url").and_then(|u| u.as_str()).unwrap_or("");
            let title = args.get("title").and_then(|t| t.as_str()).unwrap_or("");
            let author = args.get("author").and_then(|a| a.as_str()).unwrap_or("");
            let date = args.get("date").and_then(|d| d.as_str()).unwrap_or("");
            let ref_type = args
                .get("ref_type")
                .and_then(|r| r.as_str())
                .unwrap_or("website");
            let quality = args.get("quality").and_then(|q| q.as_f64());

            let citation = crate::knowledge::Reference {
                cite_id: format!("cite-{:08x}", {
                    use std::hash::{Hash, Hasher};
                    let mut h = std::collections::hash_map::DefaultHasher::new();
                    url.hash(&mut h);
                    title.hash(&mut h);
                    h.finish() as u32
                }),
                url: url.into(),
                title: title.into(),
                author: author.into(),
                date: date.into(),
                accessed: crate::dateutil::today_string(),
                snippet: String::new(),
                ref_type: serde_json::from_value(serde_json::Value::String(ref_type.into()))
                    .unwrap_or_default(),
                quality: quality.unwrap_or_else(|| {
                    let rt: crate::knowledge::ReferenceType =
                        serde_json::from_value(serde_json::Value::String(ref_type.into()))
                            .unwrap_or_default();
                    rt.initial_quality()
                }),
            };

            match store.add_citation(graph, from, to, relation, citation) {
                Ok(()) => format!(
                    "Citation added to '{}' → {} → '{}': {}",
                    from,
                    relation,
                    to,
                    if !title.is_empty() { title } else { url }
                ),
                Err(e) => format!("Error: {}", e),
            }
        }
        "graph_query_uncertain" => {
            let threshold = args
                .get("threshold")
                .and_then(|t| t.as_f64())
                .unwrap_or(0.5);
            match store.query_uncertain(graph, threshold) {
                Ok(result) if result.edges.is_empty() => {
                    format!("No edges below {:.0}% confidence", threshold * 100.0)
                }
                Ok(result) => store
                    .with_graph_render(graph, &result)
                    .unwrap_or_else(|| "Results found".into()),
                Err(e) => format!("Error: {}", e),
            }
        }
        "graph_query_by_kind" => {
            let kind = args.get("kind").and_then(|k| k.as_str()).unwrap_or("fact");
            match store.query_by_kind(graph, kind) {
                Ok(result) if result.nodes.is_empty() => {
                    format!("No '{}' nodes found", kind)
                }
                Ok(result) => store
                    .with_graph_render(graph, &result)
                    .unwrap_or_else(|| "Results found".into()),
                Err(e) => format!("Error: {}", e),
            }
        }
        "graph_list_graphs" => match store.list_graphs() {
            Ok(graphs) => graphs
                .iter()
                .map(|g| {
                    format!(
                        "{} ({} nodes, {} edges)",
                        g.name, g.node_count, g.edge_count
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"),
            Err(e) => format!("Error: {}", e),
        },
        "graph_list_orphans" => match store.list_orphans(graph) {
            Ok(orphans) if orphans.is_empty() => "No orphan nodes".into(),
            Ok(orphans) => format!("{} orphan(s):\n{}", orphans.len(), orphans.join("\n")),
            Err(e) => format!("Error: {}", e),
        },
        "graph_query_justification" => {
            let from = args.get("from").and_then(|f| f.as_str()).unwrap_or("");
            let to = args.get("to").and_then(|t| t.as_str()).unwrap_or("");
            let relation = args.get("relation").and_then(|r| r.as_str()).unwrap_or("");
            match store.query_justification(graph, from, to, relation) {
                Ok(text) => text,
                Err(e) => format!("Error: {}", e),
            }
        }
        "graph_query_reputation" => {
            let source_id = args.get("source_id").and_then(|s| s.as_str()).unwrap_or("");
            let rep_path = memory_dir.join("reputation.json");
            let registry = ReputationRegistry::load(&rep_path);

            if source_id.is_empty() {
                registry.render_summary()
            } else {
                match registry.peek_reputation(source_id) {
                    Some(score) => {
                        let entry = &registry.sources[source_id];
                        format!("{}: {:.0}% reputation ({} corroborations, {} contradictions, category: {:?})",
                            source_id, score * 100.0, entry.corroborations, entry.contradictions, entry.category)
                    }
                    None => format!("Source '{}' not tracked yet.", source_id),
                }
            }
        }
        "talk_to_ant" => {
            let ant_name = args.get("ant").and_then(|a| a.as_str()).unwrap_or("");
            let message = args.get("message").and_then(|m| m.as_str()).unwrap_or("");

            if ant_name.is_empty() || message.is_empty() {
                return "Error: 'ant' and 'message' are required.".into();
            }

            // Derive self name and ants directory.
            let self_name = memory_dir
                .parent()
                .and_then(|p| p.parent())
                .and_then(|p| p.file_name())
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_else(|| "unknown".into());

            let ants_dir = memory_dir
                .parent()
                .and_then(|p| p.parent())
                .and_then(|p| p.parent());

            let exists = ants_dir
                .map(|d| d.join(ant_name).join("working").join("memory").exists())
                .unwrap_or(false);

            if !exists {
                return format!("ANT '{}' not found. Check the [COLONY] section in your prompt for available peers.", ant_name);
            }

            // Write a colony request file that the supervisor/worker picks up.
            // Format: JSON file in memory/colony_outbox/<target>.json
            let outbox = memory_dir.join("colony_outbox");
            let _ = std::fs::create_dir_all(&outbox);
            let request = serde_json::json!({
                "from": self_name,
                "to": ant_name,
                "message": message,
                "timestamp": crate::dateutil::datetime_now(),
            });
            let filename = format!(
                "{}-{}.json",
                ant_name,
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis()
            );
            let _ = std::fs::write(
                outbox.join(&filename),
                serde_json::to_string_pretty(&request).unwrap_or_default(),
            );

            format!("Message sent to {}. Their response will arrive in your conversation when they've thought about it.\n\n\
                     (Message: '{}')", ant_name,
                     if message.len() > 100 { &message[..100] } else { message })
        }
        "query_ant" => {
            let ant_name = args.get("ant").and_then(|a| a.as_str()).unwrap_or("");
            let question = args.get("entity").and_then(|e| e.as_str()).unwrap_or("");

            if ant_name.is_empty() || question.is_empty() {
                return "Error: 'ant' and 'entity' are required. Use list_colony_ants to see available ANTs.".into();
            }

            // DON'T read the other ANT's files directly — that bypasses their reasoning.
            // Instead, send a real message that fires up their AI worker.
            // The response will be forwarded back to our chat when ready.
            let ants_dir = memory_dir
                .parent()
                .and_then(|p| p.parent())
                .and_then(|p| p.parent());

            let _self_name = memory_dir
                .parent()
                .and_then(|p| p.parent())
                .and_then(|p| p.file_name())
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_else(|| "unknown".into());

            // Check the target ANT exists.
            let exists = ants_dir
                .map(|d| d.join(ant_name).join("working").join("memory").exists())
                .unwrap_or(false);

            if !exists {
                format!(
                    "ANT '{}' not found. Use list_colony_ants to see available ANTs.",
                    ant_name
                )
            } else {
                // Quick peek at the other ANT's knowledge for immediate context.
                // This is a READ-ONLY look at their graph — they haven't reasoned about
                // this question. For a real conversation, tell the user to use /ask.
                let other_memory = ants_dir
                    .unwrap()
                    .join(ant_name)
                    .join("working")
                    .join("memory");
                let other_store = LiveKnowledgeStore::new(other_memory);
                let depth = args.get("depth").and_then(|d| d.as_u64()).unwrap_or(2) as usize;

                let mut response = String::new();
                if let Ok(graphs) = other_store.list_graphs() {
                    for g in &graphs {
                        if let Ok(result) = other_store.query_about(&g.name, question, depth) {
                            if !result.nodes.is_empty() {
                                if let Some(rendered) =
                                    other_store.with_graph_render(&g.name, &result)
                                {
                                    if !rendered.trim().is_empty() {
                                        response.push_str(&format!(
                                            "### {} (from {})\n",
                                            g.name, ant_name
                                        ));
                                        response.push_str(&rendered);
                                        response.push('\n');
                                    }
                                }
                            }
                        }
                    }
                }

                if response.is_empty() {
                    format!(
                        "{} has no knowledge about '{}' in their graphs.\n\n\
                            For a real conversation where {} THINKS about your question, \
                            tell the user to type: /ask {} {}",
                        ant_name, question, ant_name, ant_name, question
                    )
                } else {
                    format!(
                        "READ-ONLY peek at {}'s existing knowledge about '{}' \
                            (source_id: 'ant:{}'):\n\n{}\n\
                            NOTE: This is what {} already knows — they haven't reasoned \
                            about your specific question. For a real conversation where \
                            they THINK about it, suggest the user type: /ask {} {}",
                        ant_name, question, ant_name, response, ant_name, ant_name, question
                    )
                }
            }
        }
        "list_colony_ants" => {
            // Find all ANTs in the colony.
            let ants_dir = memory_dir
                .parent()
                .and_then(|p| p.parent())
                .and_then(|p| p.parent());

            let Some(dir) = ants_dir else {
                return "Error: cannot locate colony directory".into();
            };

            let entries = match std::fs::read_dir(dir) {
                Ok(e) => e,
                Err(_) => return "Error: cannot read colony directory".into(),
            };

            let mut listing = String::from("Colony ANTs (communities of practice):\n\n");
            let self_name = memory_dir
                .parent()
                .and_then(|p| p.parent())
                .and_then(|p| p.file_name())
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_default();

            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let name = path
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_default();

                let is_self = name == self_name;
                let memory = path.join("working").join("memory");
                if !memory.exists() {
                    continue;
                }

                let other_store = LiveKnowledgeStore::new(memory);
                let topics: Vec<String> = other_store
                    .list_graphs()
                    .map(|gs| {
                        gs.iter()
                            .filter(|g| g.name != "meta" && g.node_count > 0)
                            .map(|g| format!("{} ({} nodes)", g.name, g.node_count))
                            .collect()
                    })
                    .unwrap_or_default();

                let marker = if is_self { " ← you" } else { "" };
                listing.push_str(&format!(
                    "**{}**{}: {}\n",
                    name,
                    marker,
                    if topics.is_empty() {
                        "no topic graphs yet".into()
                    } else {
                        topics.join(", ")
                    }
                ));
            }

            listing.push_str("\nUse query_ant(ant='<name>', entity='<topic>') to consult a peer.");
            listing
        }
        "thought_history" => {
            let limit = args.get("limit").and_then(|l| l.as_u64()).unwrap_or(10) as usize;
            let since = args.get("since").and_then(|s| s.as_str()).unwrap_or("");

            if !since.is_empty() {
                // Show what changed since a specific commit.
                match store.diff_since(since) {
                    Ok(diff) => diff,
                    Err(e) => format!("Error: {}", e),
                }
            } else {
                match store.history(graph, limit) {
                    Ok(commits) if commits.is_empty() => "No thought history yet.".into(),
                    Ok(commits) => {
                        let mut text = String::from("Thought history:\n\n");
                        for c in &commits {
                            text.push_str(&format!(
                                "{} | {} | {}\n",
                                c.hash, c.timestamp, c.message
                            ));
                        }
                        text.push_str("\nUse thought_history with 'since' parameter to see what changed since a specific commit.");
                        text
                    }
                    Err(e) => format!("Error: {}", e),
                }
            }
        }
        "thought_branch" => {
            let action = args.get("action").and_then(|a| a.as_str()).unwrap_or("");
            let name = args.get("name").and_then(|n| n.as_str()).unwrap_or("");

            match action {
                "create" => {
                    if name.is_empty() {
                        return "Error: branch name required".into();
                    }
                    match store.create_thought_branch(name) {
                        Ok(branch) => format!("Created thought branch: {}. Explore freely — merge or abandon when done.", branch),
                        Err(e) => format!("Error: {}", e),
                    }
                }
                "merge" => {
                    if name.is_empty() {
                        return "Error: branch name required".into();
                    }
                    let branch = if name.starts_with("thought/") {
                        name.to_string()
                    } else {
                        format!("thought/{}", name)
                    };
                    match store.merge_thought_branch(&branch) {
                        Ok(true) => {
                            format!("Merged {} — ideas adopted into main knowledge.", branch)
                        }
                        Ok(false) => format!(
                            "Merge conflict on {} — resolve manually or abandon.",
                            branch
                        ),
                        Err(e) => format!("Error: {}", e),
                    }
                }
                "abandon" => {
                    if name.is_empty() {
                        return "Error: branch name required".into();
                    }
                    let branch = if name.starts_with("thought/") {
                        name.to_string()
                    } else {
                        format!("thought/{}", name)
                    };
                    match store.abandon_thought_branch(&branch) {
                        Ok(()) => format!(
                            "Abandoned {} — dead-end exploration preserved in git history.",
                            branch
                        ),
                        Err(e) => format!("Error: {}", e),
                    }
                }
                "list" => match store.list_thought_branches() {
                    Ok(branches) if branches.is_empty() => "No active thought branches.".into(),
                    Ok(branches) => {
                        let current = store.current_branch().unwrap_or_default();
                        let mut text = String::from("Thought branches:\n\n");
                        for b in &branches {
                            let marker = if *b == current { " ← current" } else { "" };
                            text.push_str(&format!("  {}{}\n", b, marker));
                        }
                        text
                    }
                    Err(e) => format!("Error: {}", e),
                },
                "current" => match store.current_branch() {
                    Ok(branch) => format!("Current branch: {}", branch),
                    Err(e) => format!("Error: {}", e),
                },
                _ => "Error: action must be create, merge, abandon, list, or current".into(),
            }
        }
        _ => format!("Unknown tool: {}", tool),
    }
}
