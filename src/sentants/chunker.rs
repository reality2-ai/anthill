//! Chunker sentant — pure FSM for PTY output batching.
//!
//! States: Idle(0) → Buffering(1) → Idle
//! Receives RELAY_OUTPUT → tells plugin to append, starts debounce timer.
//! Receives RELAY_FLUSH → tells plugin to flush buffer to Telegram.

use r2_engine::action::Action;
use r2_engine::action_buf::ActionBuf;
use r2_engine::event::{Event, Target};
use r2_engine::plugin::PluginId;
use r2_engine::sentant::{Sentant, StateId};

use crate::events::*;
use crate::plugins::chunker as chunker_plugin;

const STATE_IDLE: StateId = 0;
const STATE_BUFFERING: StateId = 1;

/// Debounce delay in milliseconds.
const DEBOUNCE_MS: u32 = 500;

pub struct ChunkerSentant {
    state: StateId,
    plugin_id: PluginId,
}

impl ChunkerSentant {
    pub fn new(plugin_id: PluginId) -> Self {
        Self {
            state: STATE_IDLE,
            plugin_id,
        }
    }
}

impl Sentant for ChunkerSentant {
    fn handle_event(&mut self, event: &Event, actions: &mut ActionBuf) {
        match event.hash {
            RELAY_OUTPUT => {
                // Tell the plugin to append the data (payload passes through).
                actions.push(Action::plugin_call(
                    self.plugin_id,
                    chunker_plugin::CMD_APPEND,
                    event.payload,
                ));

                if self.state == STATE_IDLE {
                    self.state = STATE_BUFFERING;
                    actions.push(Action::delayed_send(
                        DEBOUNCE_MS,
                        Target::Local,
                        RELAY_FLUSH,
                        &[0xA0],
                    ));
                }
            }

            RELAY_FLUSH => {
                // Tell the plugin to flush and send to Telegram.
                actions.push(Action::plugin_call(
                    self.plugin_id,
                    chunker_plugin::CMD_FLUSH,
                    &[],
                ));
                self.state = STATE_IDLE;
            }

            _ => {}
        }
    }

    fn state(&self) -> StateId {
        self.state
    }

    fn class_hash(&self) -> u32 {
        r2_fnv::fnv1a_32(b"ai.reality2.relay.chunker")
    }

    fn name(&self) -> &str {
        "chunker"
    }

    fn subscriptions(&self) -> &[u32] {
        &[RELAY_OUTPUT, RELAY_FLUSH]
    }
}
