//! Event name hashes for anthill.
//!
//! All computed at compile time via `r2_fnv::fnv1a_32` on pre-canonicalized
//! byte strings (lowercase, no whitespace).

/// User input from Telegram → terminal.
/// Payload: `{ 0: text(command), 1: uint(chat_id) }`
pub const RELAY_INPUT: u32 = r2_fnv::fnv1a_32(b"relay.input");

/// Raw PTY output from plugin (no chat_id) → terminal sentant.
/// Payload: `{ 0: bytes(data) }`
pub const RELAY_PTY_RAW: u32 = r2_fnv::fnv1a_32(b"relay.pty_raw");

/// PTY output with chat_id → chunker.
/// Payload: `{ 0: bytes(data), 1: uint(chat_id) }`
pub const RELAY_OUTPUT: u32 = r2_fnv::fnv1a_32(b"relay.output");

/// Chunked output → Telegram.
/// Payload: `{ 0: text(chunk), 1: uint(chat_id) }`
#[allow(dead_code)]
pub const RELAY_CHUNK: u32 = r2_fnv::fnv1a_32(b"relay.chunk");

/// PTY session started.
/// Payload: `{ 0: uint(chat_id) }`
#[allow(dead_code)]
pub const RELAY_PTY_START: u32 = r2_fnv::fnv1a_32(b"relay.pty_start");

/// PTY process exited.
/// Payload: `{ 0: uint(chat_id), 1: int(exit_code) }`
pub const RELAY_PTY_EXIT: u32 = r2_fnv::fnv1a_32(b"relay.pty_exit");

/// Internal: chunker debounce flush timer.
/// Payload: `{ 0: uint(chat_id) }`
pub const RELAY_FLUSH: u32 = r2_fnv::fnv1a_32(b"relay.flush");

/// Natural-language command from Telegram → AI sentant (ai_mode only).
/// Payload: `{ 0: text(command), 1: uint(chat_id) }`
pub const RELAY_COMMAND: u32 = r2_fnv::fnv1a_32(b"relay.command");

/// Claude API response is available in the shared queue.
/// Payload: `{ 0: uint(kind), 1: uint(chat_id) }`
///   kind: 0 = translate, 1 = summarise
pub const RELAY_AI_READY: u32 = r2_fnv::fnv1a_32(b"relay.ai_ready");

/// Internal: output debounce timer for AI summarisation.
/// Payload: `{ 0: uint(chat_id) }`
pub const RELAY_AI_COLLECT: u32 = r2_fnv::fnv1a_32(b"relay.ai_collect");
