//! Colony Bridge — inter-ANT communication plugin.
//!
//! Enables ANTs within the same colony to query each other's knowledge graphs
//! following the "communities of practice" model: each ANT retains its area
//! of expertise and consults peers for cross-domain questions.
//!
//! Communication uses R2 protocols:
//! - r2-fnv for event hashing (COLONY_QUERY, COLONY_RESPONSE)
//! - r2-cbor for payload encoding
//! - r2-trust for authentication (inter-colony, future)
//!
//! Knowledge from other ANTs enters as conjectures with source_id "ant:<name>",
//! subject to the same Popperian evaluation as any other evidence.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use r2_engine::plugin::{Plugin, PluginCommand, PluginId, PluginResult, PluginResponse, PluginError};
use crate::events::*;
use crate::store::KnowledgeStore;
use crate::store::live::LiveKnowledgeStore;

// Colony bridge commands.
pub const CMD_COLONY_QUERY: u8 = 0x01;
pub const CMD_COLONY_LIST: u8 = 0x02;

/// A query from one ANT to another.
#[derive(Debug, Clone)]
struct ColonyQuery {
    target_ant: String,
    entity: String,
    depth: usize,
    msg_id: u32,
}

/// A response from a queried ANT.
#[derive(Debug, Clone)]
struct ColonyResponse {
    from_ant: String,
    msg_id: u32,
    summary: String,
    node_count: usize,
    edge_count: usize,
}

/// Information about an ANT's expertise.
#[derive(Debug, Clone)]
pub struct AntExpertise {
    pub name: String,
    pub display_name: String,
    pub topics: Vec<String>,
    pub total_nodes: usize,
}

/// Shared state for discovering other ANTs.
/// The supervisor populates this with working directories of all ANTs.
pub struct ColonyDirectory {
    /// Map of ant_name -> working_dir for all ANTs in the colony.
    pub ants: std::collections::HashMap<String, PathBuf>,
}

impl ColonyDirectory {
    pub fn new() -> Self {
        Self { ants: std::collections::HashMap::new() }
    }

    /// Register an ANT's working directory.
    pub fn register(&mut self, name: String, working_dir: PathBuf) {
        self.ants.insert(name, working_dir);
    }
}

/// The Colony Bridge plugin.
/// One instance per ANT — handles outgoing queries and incoming responses.
pub struct ColonyBridge {
    id: PluginId,
    ant_name: String,
    /// Shared directory of all ANTs in the colony.
    directory: Arc<Mutex<ColonyDirectory>>,
    /// Pending responses to deliver as COLONY_RESPONSE events.
    responses: VecDeque<ColonyResponse>,
    /// Pending expertise listings.
    expertise_listings: VecDeque<String>,
    /// Next message ID.
    next_msg_id: u32,
}

impl ColonyBridge {
    pub fn new(id: PluginId, ant_name: String, directory: Arc<Mutex<ColonyDirectory>>) -> Self {
        Self {
            id,
            ant_name,
            directory,
            responses: VecDeque::new(),
            expertise_listings: VecDeque::new(),
            next_msg_id: 1,
        }
    }

    /// Query another ANT's knowledge graph.
    fn handle_query(&mut self, data: &[u8]) -> PluginResult {
        // Decode CBOR payload: { 0: str(target_ant), 1: str(entity), 2: uint(depth) }
        let query = match decode_query(data) {
            Some(q) => q,
            None => return PluginResult::Error(PluginError::new(0x01, "invalid query payload")),
        };

        let target = query.target_ant.clone();
        let entity = query.entity.clone();
        let depth = query.depth;

        // Look up the target ANT's working directory.
        let working_dir = {
            let dir = match self.directory.lock() {
                Ok(d) => d,
                Err(_) => return PluginResult::Error(PluginError::new(0x02, "directory lock failed")),
            };
            match dir.ants.get(&target) {
                Some(wd) => wd.clone(),
                None => return PluginResult::Error(PluginError::new(0x03, "ANT not found")),
            }
        };

        // Query the target's knowledge store.
        let memory_dir = working_dir.join("memory");
        let store = LiveKnowledgeStore::new(memory_dir);

        let summary = match store.query_about("meta", &entity, depth) {
            Ok(result) => {
                // Also try all topic graphs.
                let mut full_summary = store.with_graph_render("meta", &result)
                    .unwrap_or_default();

                if let Ok(graphs) = store.list_graphs() {
                    for g in &graphs {
                        if g.name == "meta" { continue; }
                        if let Ok(topic_result) = store.query_about(&g.name, &entity, depth) {
                            if !topic_result.nodes.is_empty() {
                                if let Some(rendered) = store.with_graph_render(&g.name, &topic_result) {
                                    if !rendered.is_empty() {
                                        full_summary.push_str(&format!("\n### {} (from {})\n", g.name, target));
                                        full_summary.push_str(&rendered);
                                    }
                                }
                            }
                        }
                    }
                }

                if full_summary.is_empty() {
                    format!("{} has no knowledge about '{}'", target, entity)
                } else {
                    format!("Knowledge from {} about '{}':\n\n{}", target, entity, full_summary)
                }
            }
            Err(e) => format!("Error querying {}: {}", target, e),
        };

        let node_count = store.list_graphs()
            .map(|gs| gs.iter().map(|g| g.node_count).sum())
            .unwrap_or(0);

        self.responses.push_back(ColonyResponse {
            from_ant: target,
            msg_id: self.next_msg_id,
            summary,
            node_count,
            edge_count: 0,
        });
        self.next_msg_id += 1;

        PluginResult::Ok(PluginResponse::empty())
    }

    /// List all ANTs and their expertise.
    fn handle_list(&mut self) -> PluginResult {
        let dir = match self.directory.lock() {
            Ok(d) => d,
            Err(_) => return PluginResult::Error(PluginError::new(0x02, "directory lock failed")),
        };

        let mut listing = String::from("Colony ANTs:\n\n");

        for (name, working_dir) in &dir.ants {
            if *name == self.ant_name { continue; } // Skip self.
            let memory_dir = working_dir.join("memory");
            let store = LiveKnowledgeStore::new(memory_dir);

            let topics: Vec<String> = store.list_graphs()
                .map(|gs| gs.iter()
                    .filter(|g| g.name != "meta" && g.node_count > 0)
                    .map(|g| format!("{} ({} nodes)", g.name, g.node_count))
                    .collect())
                .unwrap_or_default();

            listing.push_str(&format!("**{}**: {}\n",
                name,
                if topics.is_empty() { "no topic graphs yet".into() }
                else { topics.join(", ") }
            ));
        }

        self.expertise_listings.push_back(listing);
        PluginResult::Ok(PluginResponse::empty())
    }
}

impl Plugin for ColonyBridge {
    fn execute(&mut self, command: PluginCommand, data: &[u8]) -> PluginResult {
        match command {
            CMD_COLONY_QUERY => self.handle_query(data),
            CMD_COLONY_LIST => self.handle_list(),
            _ => PluginResult::Error(PluginError::new(0xFF, "unknown colony command")),
        }
    }

    fn name(&self) -> &str {
        "colony-bridge"
    }

    fn id(&self) -> PluginId {
        self.id
    }

    fn poll(&mut self) -> Option<(u32, &[u8])> {
        // Return pending colony responses as events.
        // We can't return String data from poll (it returns &[u8] borrowing from self),
        // so we encode the response summary length as a signal.
        // The actual text is retrieved via a follow-up execute command.
        if !self.responses.is_empty() || !self.expertise_listings.is_empty() {
            // Signal that a colony response is ready.
            // The conductor will call CMD_COLONY_RETRIEVE to get the actual text.
            Some((COLONY_RESPONSE, &[]))
        } else {
            None
        }
    }
}

/// Retrieve the next pending response text.
/// Called by the AI plugin when it receives a COLONY_RESPONSE event.
impl ColonyBridge {
    pub fn take_response(&mut self) -> Option<String> {
        if let Some(resp) = self.responses.pop_front() {
            Some(resp.summary)
        } else {
            self.expertise_listings.pop_front()
        }
    }
}

// ── CBOR helpers ───────────────────────────────────────────────────

fn decode_query(data: &[u8]) -> Option<ColonyQuery> {
    // Decode Standard-mode CBOR map with string keys.
    let mut dec = r2_cbor::Decoder::new_with_mode(data, r2_cbor::Mode::Standard);
    let r2_cbor::Item::Map(n) = dec.next().ok()? else { return None };

    let mut target = String::new();
    let mut entity = String::new();
    let mut depth = 2u64;

    for _ in 0..n {
        let key = match dec.next().ok()? {
            r2_cbor::Item::Text(t) => std::str::from_utf8(t).ok()?.to_string(),
            r2_cbor::Item::UInt(k) => k.to_string(),
            _ => { let _ = dec.next(); continue; }
        };

        match key.as_str() {
            "to" | "0" => {
                if let r2_cbor::Item::Text(t) = dec.next().ok()? {
                    target = std::str::from_utf8(t).ok()?.to_string();
                }
            }
            "entity" | "1" => {
                if let r2_cbor::Item::Text(t) = dec.next().ok()? {
                    entity = std::str::from_utf8(t).ok()?.to_string();
                }
            }
            "depth" | "2" => {
                if let r2_cbor::Item::UInt(d) = dec.next().ok()? {
                    depth = d;
                }
            }
            _ => { let _ = dec.next(); }
        }
    }

    if target.is_empty() || entity.is_empty() { return None; }

    Some(ColonyQuery {
        target_ant: target,
        entity,
        depth: depth as usize,
        msg_id: 0,
    })
}

/// Encode a colony query as CBOR (Standard mode, string keys).
pub fn encode_query(target: &str, entity: &str, depth: usize) -> Vec<u8> {
    let mut buf = [0u8; 512];
    let mut enc = r2_cbor::Encoder::new(&mut buf);
    let _ = enc.map(3);
    let _ = enc.text("to");
    let _ = enc.text(target);
    let _ = enc.text("entity");
    let _ = enc.text(entity);
    let _ = enc.text("depth");
    let _ = enc.uint(depth as u64);
    enc.as_bytes().to_vec()
}
