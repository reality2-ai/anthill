//! AiPlugin — bridges Claude API responses into the R2 event bus.
//!
//! Shares a `VecDeque<AiResponse>` with the background claude_worker.
//! `poll()` checks the queue and emits `RELAY_AI_READY` when a response
//! is available. The AiSentant then pops the actual response.

use r2_engine::plugin::*;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use crate::claude_worker::AiResponse;
use crate::events::RELAY_AI_READY;

pub struct AiPlugin {
    id: PluginId,
    response_queue: Arc<Mutex<VecDeque<AiResponse>>>,
    /// Pre-encoded CBOR buffer for poll().
    poll_buf: Vec<u8>,
}

impl AiPlugin {
    pub fn new(id: PluginId, response_queue: Arc<Mutex<VecDeque<AiResponse>>>) -> Self {
        Self {
            id,
            response_queue,
            poll_buf: Vec::new(),
        }
    }
}

impl Plugin for AiPlugin {
    fn execute(&mut self, _command: PluginCommand, _data: &[u8]) -> PluginResult {
        PluginResult::Error(PluginError::new(0xFF, "no commands supported"))
    }

    fn name(&self) -> &str {
        "claude"
    }

    fn id(&self) -> PluginId {
        self.id
    }

    fn poll(&mut self) -> Option<(u32, &[u8])> {
        let queue = self.response_queue.lock().ok()?;
        let front = queue.front()?;
        let kind = front.kind as u64;
        let chat_id = front.chat_id as u64;
        drop(queue);

        // Encode CBOR: { 0: uint(kind), 1: uint(chat_id) }
        self.poll_buf.clear();
        self.poll_buf.push(0xA2); // map(2)
        // key 0, value uint(kind)
        self.poll_buf.push(0x00);
        self.poll_buf.push(kind as u8); // 0 or 1, fits in single byte
        // key 1, value uint(chat_id)
        self.poll_buf.push(0x01);
        encode_uint(&mut self.poll_buf, chat_id);

        Some((RELAY_AI_READY, &self.poll_buf))
    }
}

/// Encode a CBOR unsigned integer.
fn encode_uint(buf: &mut Vec<u8>, v: u64) {
    if v <= 23 {
        buf.push(v as u8);
    } else if v <= 0xFF {
        buf.push(0x18);
        buf.push(v as u8);
    } else if v <= 0xFFFF {
        buf.push(0x19);
        buf.extend_from_slice(&(v as u16).to_be_bytes());
    } else if v <= 0xFFFF_FFFF {
        buf.push(0x1A);
        buf.extend_from_slice(&(v as u32).to_be_bytes());
    } else {
        buf.push(0x1B);
        buf.extend_from_slice(&v.to_be_bytes());
    }
}
