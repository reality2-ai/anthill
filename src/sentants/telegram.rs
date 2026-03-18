//! Telegram sentant — bridges Telegram messages into the R2 event bus.
//!
//! Listens for RELAY_CHUNK and RELAY_PTY_EXIT to send responses.
//! The TelegramPlugin handles the actual Bot API interaction;
//! this sentant handles the R2-side event routing.

use r2_engine::action::Action;
use r2_engine::action_buf::ActionBuf;
use r2_engine::event::Event;
use r2_engine::plugin::PluginId;
use r2_engine::sentant::{Sentant, StateId};

use crate::events::*;
use crate::plugins::telegram_bot;

pub struct TelegramSentant {
    state: StateId,
    telegram_plugin_id: PluginId,
}

impl TelegramSentant {
    pub fn new(telegram_plugin_id: PluginId) -> Self {
        Self {
            state: 0,
            telegram_plugin_id,
        }
    }
}

impl Sentant for TelegramSentant {
    fn handle_event(&mut self, event: &Event, actions: &mut ActionBuf) {
        match event.hash {
            RELAY_PTY_EXIT => {
                // Notify the user that the session ended.
                // Try to extract chat_id from payload.
                if let Some(chat_id) = decode_chat_id(event.payload) {
                    let msg = "Session ended.";
                    let payload = encode_text_chat_cbor(msg, chat_id);
                    actions.push(Action::plugin_call(
                        self.telegram_plugin_id,
                        telegram_bot::CMD_SEND_TEXT,
                        &payload,
                    ));
                }
            }

            _ => {}
        }
    }

    fn state(&self) -> StateId {
        self.state
    }

    fn class_hash(&self) -> u32 {
        r2_fnv::fnv1a_32(b"ai.reality2.relay.telegram")
    }

    fn name(&self) -> &str {
        "telegram"
    }

    fn subscriptions(&self) -> &[u32] {
        &[RELAY_PTY_EXIT]
    }
}

/// Decode chat_id from CBOR key 1.
fn decode_chat_id(data: &[u8]) -> Option<i64> {
    let mut dec = r2_cbor::Decoder::new_with_mode(data, r2_cbor::Mode::Standard);
    let r2_cbor::Item::Map(n) = dec.next().ok()? else { return None };
    for _ in 0..n {
        let r2_cbor::Item::UInt(key) = dec.next().ok()? else { return None };
        if key == 1 {
            let r2_cbor::Item::UInt(v) = dec.next().ok()? else { return None };
            return Some(v as i64);
        }
        let _ = dec.next().ok()?;
    }
    None
}

/// Encode CBOR: { 0: text, 1: uint(chat_id) }
fn encode_text_chat_cbor(text: &str, chat_id: i64) -> Vec<u8> {
    let mut buf = [0u8; 4200];
    let mut enc = r2_cbor::Encoder::new(&mut buf);
    enc.map(2).unwrap();
    enc.kv(0, &r2_cbor::Value::Text(text)).unwrap();
    enc.kv(1, &r2_cbor::Value::UInt(chat_id as u64)).unwrap();
    enc.as_bytes().to_vec()
}
