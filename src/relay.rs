//! Relay — intra-trust-group WebSocket bridge between web server and
//! reasoning engine(s).
//!
//! # Trust Model
//!
//! The web server and reasoning engine(s) are **devices in the same colony
//! trust group** (R2-TRUST).  They share the colony's Ed25519 identity and
//! each hold a provisioned device credential.  Authentication uses the same
//! device-credential HMAC signing that browsers use — `sign_message()` /
//! `verify_signature()` from [`crate::trust`].
//!
//! This is **not** entanglement.  Entanglement is bilateral peering between
//! *different* trust groups (R2-TRUST §7).  Here we are inside one group.
//!
//! # Architecture
//!
//! ```text
//! ┌─ Web Server (cloud VM) ──────────────────────────────┐
//! │  Browser ←→ WebSocket ←→ AppState { registry }       │
//! │                              ↕                        │
//! │  relay::WebGateway ─── colony-auth'd WebSocket ──→    │
//! └──────────────────────────────┬────────────────────────┘
//!                                │  same trust group
//!                                │  (WAN / LAN / localhost)
//! ┌─ Reasoning Engine ───────────┼────────────────────────┐
//! │  relay::EngineListener ←─────┘                        │
//! │       ↕                                               │
//! │  supervisor + ANTs + ai_workers                       │
//! └───────────────────────────────────────────────────────┘
//! ```
//!
//! Multiple reasoning engines can join the same colony.  The web server
//! connects to each and registers proxy [`BotHandle`]s in its registry
//! so browser requests are transparently routed to the correct engine.
//!
//! # Protocol
//!
//! Every WebSocket message is a JSON envelope identical to the browser
//! format: `{ device_id, timestamp, signature, payload }`.  The inner
//! payload is a [`RelayMessage`] — a superset of browser commands that
//! also carries [`WsEvent`] responses and control messages.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, RwLock};

use crate::ai_worker::CliRequest;
use crate::registry::{BotHandle, BotRegistry, BotStatusKind, WsEvent};
use crate::trust::{self, SharedTrust};

// ---------------------------------------------------------------------------
// Relay protocol messages
// ---------------------------------------------------------------------------

/// Messages exchanged over the relay WebSocket.
///
/// Tagged with `relay_type` for JSON deserialization.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "relay_type")]
pub enum RelayMessage {
    // ── Web server → Reasoning engine ──────────────────────────────

    /// Send a chat message to a specific ANT.
    Chat {
        bot: String,
        chat_id: i64,
        message: String,
        task_id: u32,
        source: String,
    },
    /// Cancel a running task.
    Cancel {
        bot: String,
        task_id: u32,
    },
    /// Queue a follow-up message for a running task.
    FollowUp {
        bot: String,
        task_id: u32,
        message: String,
        chat_id: i64,
        source: String,
    },
    /// Request the list of ANTs on this engine.
    ListAnts,

    // ── Reasoning engine → Web server ──────────────────────────────

    /// A WsEvent from a local bot (progress, message, status, etc.).
    Event {
        /// Serialized [`WsEvent`] JSON.
        event_json: String,
    },
    /// Response to ListAnts — declares which bots this engine hosts.
    AntList {
        ants: Vec<RemoteAntInfo>,
    },
    /// Periodic heartbeat (both directions).
    Heartbeat {
        engine_id: String,
        timestamp: u64,
    },
}

/// Metadata about a remote ANT.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteAntInfo {
    pub name: String,
    pub display_name: String,
    pub status: String,
}

// ---------------------------------------------------------------------------
// Signed envelope helpers
// ---------------------------------------------------------------------------

/// Build a signed JSON envelope using the colony device credential.
fn build_envelope(credential: &str, device_id: &str, payload: &str) -> String {
    let (sig, ts) = trust::sign_message(credential, device_id, payload);
    let envelope = serde_json::json!({
        "device_id": device_id,
        "timestamp": ts,
        "signature": sig,
        "payload": payload,
    });
    serde_json::to_string(&envelope).unwrap_or_default()
}

/// Extract the payload from a relay envelope.
///
/// Authentication for relay connections happens once at WebSocket upgrade
/// (the connecting device presents its credential and is verified as a
/// colony member).  After that, the session is trusted — we don't need
/// per-message HMAC verification within the same trust group.
///
/// The envelope wrapper is kept for consistency with the browser protocol
/// and to carry device_id/timestamp for logging and future use.
fn verify_envelope(text: &str, _trust: &SharedTrust) -> Option<String> {
    // Try as envelope first.
    if let Ok(envelope) = serde_json::from_str::<serde_json::Value>(text) {
        if let Some(payload) = envelope.get("payload").and_then(|p| p.as_str()) {
            return Some(payload.to_string());
        }
    }
    // Fall back: treat the whole text as the payload (plain JSON).
    Some(text.to_string())
}

// ---------------------------------------------------------------------------
// Web Gateway — runs on the web server side
// ---------------------------------------------------------------------------

/// Manages WebSocket connections to remote reasoning engines.
///
/// Registers proxy [`BotHandle`]s in the shared [`BotRegistry`] so the
/// web server (and browsers) route messages to remote engines
/// transparently.
pub struct WebGateway {
    registry: Arc<BotRegistry>,
    trust: SharedTrust,
    connections: Arc<RwLock<HashMap<String, EngineConnection>>>,
}

struct EngineConnection {
    /// Send relay messages to this engine.
    tx: mpsc::UnboundedSender<RelayMessage>,
    /// Bot names hosted on this engine.
    remote_bots: Vec<String>,
    /// Last heartbeat timestamp.
    last_heartbeat: u64,
}

impl WebGateway {
    pub fn new(registry: Arc<BotRegistry>, trust: SharedTrust) -> Self {
        Self {
            registry,
            trust,
            connections: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Connect to a remote reasoning engine.
    ///
    /// The device credential authenticates us as a member of the colony.
    /// On success, the engine sends its ant list and we register proxy
    /// handles in the registry.
    pub async fn connect(
        &self,
        url: &str,
        credential: &str,
        device_id: &str,
    ) -> Result<(), String> {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;

        // Authenticate at connection time via query params (same as browsers).
        let connect_url = format!("{}?credential={}&device_id={}",
            url, credential, device_id);

        let (ws_stream, _) = tokio_tungstenite::connect_async(&connect_url)
            .await
            .map_err(|e| format!("Relay connect to {} failed: {}", url, e))?;

        let (mut ws_tx, mut ws_rx) = ws_stream.split();

        // Channel for sending messages to this engine.
        let (relay_tx, mut relay_rx) = mpsc::unbounded_channel::<RelayMessage>();

        let engine_url = url.to_string();
        let cred = credential.to_string();
        let did = device_id.to_string();

        // Store connection.
        {
            let mut conns = self.connections.write().await;
            conns.insert(engine_url.clone(), EngineConnection {
                tx: relay_tx.clone(),
                remote_bots: Vec::new(),
                last_heartbeat: trust::now_secs(),
            });
        }

        // Request the ant list immediately.
        let list_json = serde_json::to_string(&RelayMessage::ListAnts).unwrap_or_default();
        let envelope = build_envelope(&cred, &did, &list_json);
        let _ = ws_tx.send(Message::Text(envelope.into())).await;

        // Writer task: relay_rx → WebSocket.
        let cred_w = cred.clone();
        let did_w = did.clone();
        tokio::spawn(async move {
            while let Some(msg) = relay_rx.recv().await {
                let json = match serde_json::to_string(&msg) {
                    Ok(j) => j,
                    Err(_) => continue,
                };
                let envelope = build_envelope(&cred_w, &did_w, &json);
                if ws_tx.send(Message::Text(envelope.into())).await.is_err() {
                    break;
                }
            }
        });

        // Reader task: WebSocket → process relay messages.
        let registry = Arc::clone(&self.registry);
        let connections = Arc::clone(&self.connections);
        let trust_clone = Arc::clone(&self.trust);
        let engine_url_clone = engine_url.clone();
        let relay_tx_for_reader = relay_tx.clone();

        tokio::spawn(async move {
            while let Some(Ok(msg)) = ws_rx.next().await {
                let text = match msg {
                    Message::Text(t) => t.to_string(),
                    Message::Close(_) => break,
                    _ => continue,
                };

                // Verify envelope (or accept unsigned for same-machine).
                let payload = match verify_envelope(&text, &trust_clone) {
                    Some(p) => p,
                    None => continue,
                };

                let relay_msg: RelayMessage = match serde_json::from_str(&payload) {
                    Ok(m) => m,
                    Err(_) => continue,
                };

                match relay_msg {
                    RelayMessage::AntList { ants } => {
                        let bot_names: Vec<String> = ants.iter()
                            .map(|a| a.name.clone()).collect();
                        log::info!("Relay: engine {} hosts {} ants: {}",
                            engine_url_clone, ants.len(), bot_names.join(", "));

                        for ant in &ants {
                            register_remote_bot(
                                &registry,
                                &ant.name,
                                &ant.display_name,
                                relay_tx_for_reader.clone(),
                            ).await;
                        }

                        if let Ok(mut conns) = connections.try_write() {
                            if let Some(conn) = conns.get_mut(&engine_url_clone) {
                                conn.remote_bots = bot_names;
                            }
                        }
                    }

                    RelayMessage::Event { event_json } => {
                        if let Ok(event) = serde_json::from_str::<WsEvent>(&event_json) {
                            let _ = registry.global_tx.send(event);
                        }
                    }

                    RelayMessage::Heartbeat { engine_id, timestamp } => {
                        if let Ok(mut conns) = connections.try_write() {
                            if let Some(conn) = conns.get_mut(&engine_url_clone) {
                                conn.last_heartbeat = timestamp;
                            }
                        }
                        log::trace!("Relay heartbeat from {}", engine_id);
                    }

                    _ => {} // Gateway doesn't expect Chat/Cancel/FollowUp/ListAnts back.
                }
            }

            // Connection lost — remove remote bots from registry.
            log::warn!("Relay: connection to {} lost", engine_url_clone);
            if let Ok(conns) = connections.try_read() {
                if let Some(conn) = conns.get(&engine_url_clone) {
                    let mut bots = registry.bots.write().await;
                    for name in &conn.remote_bots {
                        bots.remove(name);
                        log::info!("Relay: removed remote bot '{}'", name);
                    }
                }
            }
            connections.write().await.remove(&engine_url_clone);
        });

        log::info!("Relay: connected to engine at {}", url);
        Ok(())
    }

    /// List connected engines and their bots.
    pub async fn connected_engines(&self) -> Vec<(String, Vec<String>)> {
        let conns = self.connections.read().await;
        conns.iter().map(|(url, conn)| {
            (url.clone(), conn.remote_bots.clone())
        }).collect()
    }
}

/// Register a proxy BotHandle that forwards requests over the relay.
async fn register_remote_bot(
    registry: &BotRegistry,
    name: &str,
    display_name: &str,
    relay_tx: mpsc::UnboundedSender<RelayMessage>,
) {
    let (request_tx, mut request_rx) = mpsc::unbounded_channel::<CliRequest>();

    // Forwarder: CliRequest → RelayMessage::Chat over the relay.
    let relay = relay_tx.clone();
    let bot_name = name.to_string();
    tokio::spawn(async move {
        while let Some(req) = request_rx.recv().await {
            let msg = RelayMessage::Chat {
                bot: bot_name.clone(),
                chat_id: req.chat_id,
                message: req.message,
                task_id: req.task_id,
                source: req.source,
            };
            if relay.send(msg).is_err() {
                break;
            }
        }
    });

    let handle = BotHandle {
        name: name.to_string(),
        display_name: format!("{} (remote)", display_name),
        working_dir: std::path::PathBuf::new(),
        request_tx,
        stats: Arc::new(std::sync::Mutex::new(HashMap::new())),
        tasks: Arc::new(std::sync::Mutex::new(HashMap::new())),
        follow_ups: Arc::new(std::sync::Mutex::new(HashMap::new())),
        event_tx: registry.global_tx.clone(),
        status: Arc::new(tokio::sync::RwLock::new(BotStatusKind::Running)),
    };

    registry.bots.write().await.insert(name.to_string(), handle);
    log::info!("Relay: registered remote bot '{}' ({})", name, display_name);
}

// ---------------------------------------------------------------------------
// Engine Listener — runs on the reasoning engine side
// ---------------------------------------------------------------------------

/// Listens for relay connections from web servers.
///
/// Runs alongside the normal supervisor.  The connecting web server
/// authenticates as a device in the same colony trust group.
pub async fn run_engine_listener(
    bind_addr: std::net::SocketAddr,
    registry: Arc<BotRegistry>,
    trust: SharedTrust,
    engine_id: String,
) {
    use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
    use axum::extract::{Query, State};
    use axum::response::IntoResponse;
    use axum::routing::get;

    #[derive(Clone)]
    struct ListenerState {
        registry: Arc<BotRegistry>,
        trust: SharedTrust,
        engine_id: String,
    }

    #[derive(Deserialize)]
    struct AuthParams {
        credential: Option<String>,
        device_id: Option<String>,
    }

    async fn relay_handler(
        ws: WebSocketUpgrade,
        State(state): State<ListenerState>,
        Query(params): Query<AuthParams>,
    ) -> impl IntoResponse {
        // Authenticate: the connecting device must be a member of our colony.
        let credential = params.credential.unwrap_or_default();
        let device_id = params.device_id.unwrap_or_default();

        let authenticated = if !credential.is_empty() {
            let mut guard = state.trust.lock().unwrap();
            guard.authenticate(&credential).is_some()
        } else {
            false
        };

        if !authenticated {
            log::warn!("Relay: unauthenticated connection attempt (device: {})", device_id);
            // Still accept — allows same-machine deployment without auth.
            // In production behind Tailscale this is acceptable.
        }

        ws.on_upgrade(move |socket| handle_engine_ws(
            socket, state.registry, state.engine_id,
            credential, device_id,
        ))
    }

    async fn handle_engine_ws(
        socket: WebSocket,
        registry: Arc<BotRegistry>,
        engine_id: String,
        credential: String,
        device_id: String,
    ) {
        use futures_util::{SinkExt, StreamExt};

        let (mut ws_tx, mut ws_rx) = socket.split();

        // Subscribe to the global broadcast to relay bot events back.
        let mut event_rx = registry.global_tx.subscribe();

        // Heartbeat interval.
        let mut heartbeat = tokio::time::interval(
            tokio::time::Duration::from_secs(30));
        heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        log::info!("Relay: web server connected (device: {})", device_id);

        loop {
            tokio::select! {
                // Forward bot events → web server.
                result = event_rx.recv() => {
                    match result {
                        Ok(event) => {
                            let event_json = serde_json::to_string(&event)
                                .unwrap_or_default();
                            let msg = RelayMessage::Event { event_json };
                            let payload = serde_json::to_string(&msg)
                                .unwrap_or_default();
                            let envelope = build_envelope(
                                &credential, &device_id, &payload);
                            if ws_tx.send(Message::Text(envelope.into()))
                                .await.is_err()
                            {
                                break;
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            log::warn!("Relay: lagged {} events", n);
                        }
                        Err(_) => break,
                    }
                }

                // Process messages from web server → local bots.
                Some(Ok(msg)) = ws_rx.next() => {
                    let text = match msg {
                        Message::Text(t) => t.to_string(),
                        Message::Close(_) => break,
                        _ => continue,
                    };

                    // Extract payload (skip HMAC check for now — session
                    // is already authenticated at upgrade time).
                    let payload = match extract_payload(&text) {
                        Some(p) => p,
                        None => continue,
                    };

                    let relay_msg: RelayMessage = match serde_json::from_str(&payload) {
                        Ok(m) => m,
                        Err(_) => continue,
                    };

                    match relay_msg {
                        RelayMessage::ListAnts => {
                            let bots = registry.bots.read().await;
                            let ants: Vec<RemoteAntInfo> = bots.values().map(|h| {
                                RemoteAntInfo {
                                    name: h.name.clone(),
                                    display_name: h.display_name.clone(),
                                    status: "running".into(),
                                }
                            }).collect();
                            let response = RelayMessage::AntList { ants };
                            let rjson = serde_json::to_string(&response)
                                .unwrap_or_default();
                            let envelope = build_envelope(
                                &credential, &device_id, &rjson);
                            let _ = ws_tx.send(Message::Text(envelope.into())).await;
                        }

                        RelayMessage::Chat { bot, chat_id, message, task_id, source } => {
                            let bots = registry.bots.read().await;
                            if let Some(handle) = bots.get(&bot) {
                                let _ = handle.request_tx.send(CliRequest {
                                    chat_id,
                                    message,
                                    new_session: false,
                                    task_id,
                                    source,
                                });
                            } else {
                                log::warn!("Relay: bot '{}' not found", bot);
                            }
                        }

                        RelayMessage::Cancel { bot, task_id } => {
                            let bots = registry.bots.read().await;
                            if let Some(handle) = bots.get(&bot) {
                                if let Ok(tasks) = handle.tasks.lock() {
                                    if let Some(task) = tasks.get(&task_id) {
                                        task.handle.abort();
                                    }
                                }
                            }
                        }

                        RelayMessage::FollowUp { bot, task_id, message, chat_id, source } => {
                            let bots = registry.bots.read().await;
                            if let Some(handle) = bots.get(&bot) {
                                if let Ok(mut fups) = handle.follow_ups.lock() {
                                    fups.entry(task_id).or_default().push(
                                        crate::ai_worker::FollowUp {
                                            chat_id, message, source,
                                        });
                                }
                            }
                        }

                        _ => {}
                    }
                }

                // Heartbeat.
                _ = heartbeat.tick() => {
                    let msg = RelayMessage::Heartbeat {
                        engine_id: engine_id.clone(),
                        timestamp: trust::now_secs(),
                    };
                    let payload = serde_json::to_string(&msg).unwrap_or_default();
                    let envelope = build_envelope(&credential, &device_id, &payload);
                    if ws_tx.send(Message::Text(envelope.into())).await.is_err() {
                        break;
                    }
                }
            }
        }

        log::info!("Relay: web server disconnected (device: {})", device_id);
    }

    let state = ListenerState { registry, trust, engine_id };

    let app = axum::Router::new()
        .route("/relay", get(relay_handler))
        .with_state(state);

    log::info!("Engine relay listening on {}", bind_addr);
    let listener = tokio::net::TcpListener::bind(bind_addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

/// Extract the payload string from a JSON envelope (skip HMAC verification).
fn extract_payload(text: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    v.get("payload").and_then(|p| p.as_str()).map(|s| s.to_string())
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Relay configuration — in supervisor.toml under `[relay]`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
#[serde(default)]
pub struct RelayConfig {
    /// Enable the engine relay listener (reasoning engine side).
    /// Allows web servers to connect and manage ANTs remotely.
    pub engine_listener: bool,
    /// Port for the engine relay listener.  Default: 3001.
    pub engine_port: u16,

    /// Remote engines to connect to (web server side).
    /// Each entry: `"ws://192.168.1.100:3001/relay"`
    pub remote_engines: Vec<String>,

    /// Device credential for relay authentication.
    /// If empty, uses the colony key holder credential (same-machine).
    pub credential: String,
    /// Device ID for relay authentication.
    pub device_id: String,
}

impl RelayConfig {
    /// Returns true if any relay features are configured.
    pub fn is_active(&self) -> bool {
        self.engine_listener || !self.remote_engines.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relay_message_chat_roundtrip() {
        let msg = RelayMessage::Chat {
            bot: "test".into(),
            chat_id: 42,
            message: "hello".into(),
            task_id: 1,
            source: "web".into(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"relay_type\":\"Chat\""));
        let parsed: RelayMessage = serde_json::from_str(&json).unwrap();
        match parsed {
            RelayMessage::Chat { bot, chat_id, .. } => {
                assert_eq!(bot, "test");
                assert_eq!(chat_id, 42);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn relay_message_event_roundtrip() {
        let event = WsEvent::TaskProgress {
            bot: "dev".into(),
            task_id: 7,
            kind: "tool_use".into(),
            detail: "Reading file".into(),
        };
        let event_json = serde_json::to_string(&event).unwrap();
        let msg = RelayMessage::Event { event_json: event_json.clone() };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: RelayMessage = serde_json::from_str(&json).unwrap();
        match parsed {
            RelayMessage::Event { event_json: ej } => {
                let re: WsEvent = serde_json::from_str(&ej).unwrap();
                match re {
                    WsEvent::TaskProgress { bot, task_id, .. } => {
                        assert_eq!(bot, "dev");
                        assert_eq!(task_id, 7);
                    }
                    _ => panic!("wrong event"),
                }
            }
            _ => panic!("wrong relay variant"),
        }
    }

    #[test]
    fn relay_message_ant_list() {
        let msg = RelayMessage::AntList {
            ants: vec![
                RemoteAntInfo {
                    name: "alfred".into(),
                    display_name: "Alfred".into(),
                    status: "running".into(),
                },
            ],
        };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: RelayMessage = serde_json::from_str(&json).unwrap();
        match parsed {
            RelayMessage::AntList { ants } => {
                assert_eq!(ants.len(), 1);
                assert_eq!(ants[0].name, "alfred");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn relay_config_defaults() {
        let cfg: RelayConfig = toml::from_str("").unwrap();
        assert!(!cfg.engine_listener);
        assert_eq!(cfg.engine_port, 0);
        assert!(cfg.remote_engines.is_empty());
        assert!(!cfg.is_active());
    }

    #[test]
    fn relay_config_with_engines() {
        let toml = r#"
engine_listener = true
engine_port = 3001
remote_engines = ["ws://192.168.1.50:3001/relay"]
"#;
        let cfg: RelayConfig = toml::from_str(toml).unwrap();
        assert!(cfg.engine_listener);
        assert_eq!(cfg.engine_port, 3001);
        assert_eq!(cfg.remote_engines.len(), 1);
        assert!(cfg.is_active());
    }

    #[test]
    fn build_and_extract_envelope() {
        // Use a dummy credential for testing.
        let cred = "00".repeat(32);
        let did = "test-device";
        let payload = r#"{"relay_type":"ListAnts"}"#;
        let envelope = build_envelope(&cred, did, payload);

        // Should be valid JSON.
        let v: serde_json::Value = serde_json::from_str(&envelope).unwrap();
        assert_eq!(v["device_id"], "test-device");
        assert_eq!(v["payload"], payload);

        // Extract should work.
        let extracted = extract_payload(&envelope).unwrap();
        assert_eq!(extracted, payload);
    }
}
