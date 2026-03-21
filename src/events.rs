//! Event name hashes for Anthill.
//!
//! All computed at compile time via `r2_fnv::fnv1a_32`.

/// User message from a channel → conductor sentant.
/// Payload: `{ 0: uint(cmd_type), 1: uint(chat_id), 2: uint(cancel_task_id) }`
pub const RELAY_COMMAND: u32 = r2_fnv::fnv1a_32(b"relay.command");

/// AI response is available in the response queue.
/// Payload: `{ 0: uint(kind), 1: uint(chat_id) }`
pub const RELAY_AI_READY: u32 = r2_fnv::fnv1a_32(b"relay.ai_ready");

// ── Colony events (inter-ANT communication) ──

/// Query another ANT's knowledge graph.
/// Payload: CBOR { "to": str, "msg_id": uint, "query_type": str, "entity": str, "depth": uint }
pub const COLONY_QUERY: u32 = r2_fnv::fnv1a_32(b"colony.query");

/// Response from another ANT.
/// Payload: CBOR { "from": str, "msg_id": uint, "summary": str }
pub const COLONY_RESPONSE: u32 = r2_fnv::fnv1a_32(b"colony.response");
