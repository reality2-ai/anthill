//! ClaudeCliPlugin — polls for completed claude CLI responses.
//!
//! Shares a `VecDeque<CliResponse>` with the background claude_cli_worker.
//! `poll()` emits `RELAY_AI_READY` when a response is available.

use r2_engine::plugin::*;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use crate::claude_cli::CliResponse;
use crate::events::RELAY_AI_READY;

pub struct ClaudeCliPlugin {
    id: PluginId,
    response_queue: Arc<Mutex<VecDeque<CliResponse>>>,
    poll_buf: Vec<u8>,
}

impl ClaudeCliPlugin {
    pub fn new(id: PluginId, response_queue: Arc<Mutex<VecDeque<CliResponse>>>) -> Self {
        Self {
            id,
            response_queue,
            poll_buf: Vec::new(),
        }
    }
}

impl Plugin for ClaudeCliPlugin {
    fn execute(&mut self, _command: PluginCommand, _data: &[u8]) -> PluginResult {
        PluginResult::Error(PluginError::new(0xFF, "no commands supported"))
    }

    fn name(&self) -> &str {
        "claude-cli"
    }

    fn id(&self) -> PluginId {
        self.id
    }

    fn poll(&mut self) -> Option<(u32, &[u8])> {
        let queue = self.response_queue.lock().ok()?;
        let front = queue.front()?;
        let chat_id = front.chat_id as u64;
        drop(queue);

        // Encode CBOR: { 0: uint(0), 1: uint(chat_id) }
        // kind=0 is unused here but keeps payload compatible with RELAY_AI_READY.
        self.poll_buf.clear();
        self.poll_buf.push(0xA2); // map(2)
        self.poll_buf.push(0x00); // key 0
        self.poll_buf.push(0x00); // value 0
        self.poll_buf.push(0x01); // key 1
        encode_uint(&mut self.poll_buf, chat_id);

        Some((RELAY_AI_READY, &self.poll_buf))
    }
}

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
