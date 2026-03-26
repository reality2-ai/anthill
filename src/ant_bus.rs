//! AntBus — inter-ANT event bus following R2 sentant communication semantics.
//!
//! This is the pragmatic layer for ANT-to-ANT messaging. It follows the R2
//! specification's event model (R2-SENTANT §5, R2-DEF §3.3.1) but works with
//! Anthill's async worker tasks rather than requiring full FSM sentants.
//!
//! ## R2 Alignment
//!
//! | R2 Concept           | AntBus Implementation                              |
//! |----------------------|-----------------------------------------------------|
//! | `@local` dispatch    | `Target::Local` — deliver to all ANTs on this hive  |
//! | `@sender` reply      | `Target::Sender` — reply via `reply_to` address     |
//! | Sentant name routing | `Target::Named(name)` — case-insensitive lookup     |
//! | Fire-and-forget      | `send()` returns immediately, no delivery guarantee |
//! | Origin tagging       | `Origin::Internal` for same-hive, `External` future |
//! | Event depth limit    | Max 10 hops to prevent loops                        |
//!
//! ## Future Migration
//!
//! The `AntEvent` struct is designed to map directly to `r2_engine::Event`:
//! - `event_name` → `EventHash` (via `r2_fnv::r2_hash`)
//! - `payload` → CBOR-encoded `&[u8]`
//! - `target` → `r2_engine::Target`

use std::collections::HashMap;
use tokio::sync::{broadcast, mpsc, RwLock};

/// Where an event should be delivered.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum Target {
    /// A specific ANT by name (case-insensitive).
    Named(String),
    /// All ANTs on this hive (equivalent to R2 `@local`).
    Local,
    /// Reply to whoever sent the triggering event (R2 `@sender`).
    Sender,
}

/// Origin of an event — set by the bus, never by the sender.
/// (R2-SENTANT §5.4: "The sender MUST NOT set the origin field.")
#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
pub enum Origin {
    /// From another ANT on the same hive.
    Internal,
    /// From a remote hive (future: relay/distributed).
    External,
    /// From the user (web, telegram, slack).
    User,
}

/// A reply address — carried with events so recipients can reply via `@sender`.
/// (R2-WIRE §9: route stack for reply routing.)
#[derive(Debug, Clone)]
pub struct ReplyAddress {
    /// The ANT that sent the original event.
    pub from_ant: String,
    /// The chat_id to deliver the reply to.
    pub chat_id: i64,
    /// The WebSocket event channel for delivering the reply to the UI.
    #[allow(dead_code)]
    pub reply_tx: broadcast::Sender<crate::registry::WsEvent>,
}

/// An event on the AntBus.
///
/// Maps to R2-SENTANT §5.1 event structure:
/// - `event_name` → R2 event hash (will be FNV-1a in full migration)
/// - `data` → R2 CBOR payload (here as JSON for pragmatism)
/// - `origin` → R2 origin tag (Internal/External)
/// - `depth` → R2 event depth counter (loop prevention, §10.2)
#[derive(Debug, Clone)]
pub struct AntEvent {
    /// Event name (e.g., "colony.query", "colony.reply", "knowledge.share").
    pub event_name: String,
    /// Structured payload — the message content.
    pub data: serde_json::Value,
    /// Where to deliver this event.
    pub target: Target,
    /// Set by the bus, not the sender.
    #[allow(dead_code)]
    pub origin: Origin,
    /// Reply address for `Target::Sender` routing.
    pub reply_to: Option<ReplyAddress>,
    /// Loop prevention counter (R2-SENTANT §10.2, max 10).
    pub depth: u8,
}

/// Maximum event depth before discarding (R2-SENTANT §10.2).
const MAX_EVENT_DEPTH: u8 = 10;

/// A subscription — an ANT declares which events it handles.
struct AntSubscription {
    /// ANT identifier (directory name).
    ant_id: String,
    /// Display name for UI.
    display_name: String,
    /// Channel to deliver events to this ANT's worker.
    tx: mpsc::UnboundedSender<AntEvent>,
}

/// The AntBus — central event dispatcher for inter-ANT communication.
///
/// Follows R2-ENGINE EventBus dispatch semantics (bus.rs):
/// 1. Resolve target to concrete ANT(s)
/// 2. Tag origin
/// 3. Check depth limit
/// 4. Deliver to each matching ANT's channel
pub struct AntBus {
    /// Registered ANTs, keyed by lowercase name.
    subscriptions: RwLock<HashMap<String, AntSubscription>>,
    /// Global WebSocket channel for UI notifications.
    global_tx: broadcast::Sender<crate::registry::WsEvent>,
}

impl AntBus {
    pub fn new(global_tx: broadcast::Sender<crate::registry::WsEvent>) -> Self {
        Self {
            subscriptions: RwLock::new(HashMap::new()),
            global_tx,
        }
    }

    /// Register an ANT on the bus. Returns a receiver for incoming events.
    ///
    /// Equivalent to registering a sentant with the EventBus (r2-engine bus.rs).
    pub async fn register(&self, ant_id: &str, display_name: &str) -> mpsc::UnboundedReceiver<AntEvent> {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut subs = self.subscriptions.write().await;
        subs.insert(ant_id.to_lowercase(), AntSubscription {
            ant_id: ant_id.to_string(),
            display_name: display_name.to_string(),
            tx,
        });
        log::info!("[ant_bus] Registered ANT '{}' ({})", display_name, ant_id);
        rx
    }

    /// Unregister an ANT from the bus.
    #[allow(dead_code)]
    pub async fn unregister(&self, ant_id: &str) {
        let mut subs = self.subscriptions.write().await;
        if subs.remove(&ant_id.to_lowercase()).is_some() {
            log::info!("[ant_bus] Unregistered ANT '{}'", ant_id);
        }
    }

    /// List registered ANTs (for @mention autocomplete).
    #[allow(dead_code)]
    pub async fn list_ants(&self) -> Vec<(String, String)> {
        let subs = self.subscriptions.read().await;
        subs.values().map(|s| (s.ant_id.clone(), s.display_name.clone())).collect()
    }

    /// Send an event on the bus. Fire-and-forget (R2-WIRE §8.1).
    ///
    /// Returns the number of ANTs the event was delivered to.
    pub async fn send(&self, mut event: AntEvent) -> usize {
        // Depth check (R2-SENTANT §10.2).
        if event.depth >= MAX_EVENT_DEPTH {
            log::warn!("[ant_bus] Event depth exceeded ({}), discarding: {}", event.depth, event.event_name);
            return 0;
        }
        event.depth += 1;

        let subs = self.subscriptions.read().await;
        let mut delivered = 0;

        match &event.target {
            Target::Named(name) => {
                let key = name.to_lowercase();
                // Try exact match first, then display name match.
                let sub = subs.get(&key).or_else(|| {
                    subs.values().find(|s| s.display_name.to_lowercase() == key)
                });
                if let Some(sub) = sub {
                    if sub.tx.send(event.clone()).is_ok() {
                        delivered = 1;
                        log::info!("[ant_bus] Delivered '{}' to '{}'", event.event_name, sub.display_name);
                    } else {
                        log::warn!("[ant_bus] ANT '{}' channel closed", sub.display_name);
                    }
                } else {
                    log::warn!("[ant_bus] No ANT found matching '{}'", name);
                }
            }
            Target::Local => {
                // Deliver to ALL registered ANTs (R2 @local / @hive).
                for sub in subs.values() {
                    if sub.tx.send(event.clone()).is_ok() {
                        delivered += 1;
                    }
                }
                log::info!("[ant_bus] Broadcast '{}' to {} ANTs", event.event_name, delivered);
            }
            Target::Sender => {
                // Reply routing (R2-WIRE §9) — use the reply_to address.
                if let Some(ref reply) = event.reply_to {
                    let key = reply.from_ant.to_lowercase();
                    if let Some(sub) = subs.get(&key) {
                        if sub.tx.send(event.clone()).is_ok() {
                            delivered = 1;
                        }
                    }
                    // Also notify the UI so the reply appears in the original chat.
                    let text = event.data.get("text").and_then(|t| t.as_str()).unwrap_or("");
                    let from = event.data.get("from").and_then(|f| f.as_str()).unwrap_or("unknown");
                    if !text.is_empty() {
                        let _ = self.global_tx.send(crate::registry::WsEvent::Message {
                            bot: reply.from_ant.clone(),
                            chat_id: reply.chat_id,
                            text: format!("**@{}** replied:\n\n{}", from, text),
                            task_id: 0,
                        });
                    }
                } else {
                    log::warn!("[ant_bus] Target::Sender but no reply_to address");
                }
            }
        }

        delivered
    }

    /// Convenience: send a colony query from one ANT to another.
    ///
    /// This is the @mention flow:
    /// 1. Creates a `colony.query` event with the question + context
    /// 2. Targets the named ANT
    /// 3. Includes a reply address so the response routes back via `@sender`
    pub async fn ask(
        &self,
        from_ant: &str,
        target_ant: &str,
        chat_id: i64,
        question: String,
        context: String,
    ) -> bool {
        let event = AntEvent {
            event_name: "colony.query".into(),
            data: serde_json::json!({
                "from": from_ant,
                "question": question,
                "context": context,
            }),
            target: Target::Named(target_ant.into()),
            origin: Origin::Internal,
            reply_to: Some(ReplyAddress {
                from_ant: from_ant.into(),
                chat_id,
                reply_tx: self.global_tx.clone(),
            }),
            depth: 0,
        };

        self.send(event).await > 0
    }

    /// Convenience: send a reply back to the originator of a colony query.
    ///
    /// Uses `Target::Sender` (R2 `@sender`) to route back via the reply address.
    #[allow(dead_code)]
    pub async fn reply(
        &self,
        from_ant: &str,
        reply_to: ReplyAddress,
        response_text: String,
    ) {
        let event = AntEvent {
            event_name: "colony.reply".into(),
            data: serde_json::json!({
                "from": from_ant,
                "text": response_text,
            }),
            target: Target::Sender,
            origin: Origin::Internal,
            reply_to: Some(reply_to),
            depth: 0,
        };

        self.send(event).await;
    }
}
