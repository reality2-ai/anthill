//! SlackPlugin — bridges Slack to R2 events via Socket Mode.
//!
//! Incoming Slack messages → RELAY_COMMAND events (ai mode) or RELAY_INPUT (raw mode).
//! Outgoing messages sent via Slack Web API (plugin-to-plugin data plane).
//!
//! Uses Socket Mode (WebSocket) — no public URL needed. Works behind Tailscale.

use r2_engine::plugin::*;
use tokio::sync::mpsc;

use crate::events::RELAY_COMMAND;
use crate::plugins::telegram_bot::MessageQueue;

/// An incoming message from Slack.
struct IncomingMessage {
    text: String,
    channel: String,
    #[allow(dead_code)]
    user: String,
}

pub struct SlackPlugin {
    id: PluginId,
    incoming_rx: mpsc::Receiver<IncomingMessage>,
    #[allow(dead_code)]
    outgoing_tx: mpsc::UnboundedSender<(String, String)>, // (channel, text)
    poll_buf: Vec<u8>,
    message_queue: MessageQueue,
}

impl SlackPlugin {
    /// Create the plugin and spawn the Slack Socket Mode connection.
    pub fn new(
        id: PluginId,
        rt: &tokio::runtime::Handle,
        bot_token: String,
        app_token: String,
            message_queue: MessageQueue,
    ) -> Self {
        let (in_tx, in_rx) = mpsc::channel::<IncomingMessage>(64);
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<(String, String)>();

        let bot_token_clone = bot_token.clone();

        // Spawn outgoing message sender.
        rt.spawn(async move {
            let client = reqwest::Client::new();
            while let Some((channel, text)) = out_rx.recv().await {
                let _ = client
                    .post("https://slack.com/api/chat.postMessage")
                    .bearer_auth(&bot_token_clone)
                    .json(&serde_json::json!({
                        "channel": channel,
                        "text": text,
                        "mrkdwn": true,
                    }))
                    .send()
                    .await;
            }
        });

        // Spawn Socket Mode listener.
        let in_tx_clone = in_tx;
        rt.spawn(async move {
            loop {
                if let Err(e) = run_socket_mode(&app_token, &bot_token, &in_tx_clone).await {
                    log::error!("Slack Socket Mode error: {}. Reconnecting in 5s...", e);
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                }
            }
        });

        Self {
            id,
            incoming_rx: in_rx,
            outgoing_tx: out_tx,
            poll_buf: Vec::new(),
            message_queue,
        }
    }

}

impl Plugin for SlackPlugin {
    fn execute(&mut self, _command: PluginCommand, _data: &[u8]) -> PluginResult {
        // Outgoing messages are handled via the ClaudeCliPlugin data plane,
        // not through direct plugin commands. This plugin is input-only.
        PluginResult::Error(PluginError::new(0xFF, "no commands supported"))
    }

    fn name(&self) -> &str {
        "slack"
    }

    fn id(&self) -> PluginId {
        self.id
    }

    fn poll(&mut self) -> Option<(u32, &[u8])> {
        match self.incoming_rx.try_recv() {
            Ok(msg) => {
                // Hash the channel ID to a u64 for use as chat_id.
                let channel_hash = hash_channel(&msg.channel);

                let cmd_type = classify_command(&msg.text);

                // Strip command prefixes so the plugin gets just the content.
                let queue_text = match cmd_type {
                    8 => msg.text.trim().strip_prefix("/followup").unwrap_or(&msg.text).trim().to_string(),
                    9 => msg.text.trim().strip_prefix("/analyse").or_else(|| msg.text.trim().strip_prefix("/analyze")).unwrap_or(&msg.text).trim().to_string(),
                    _ => msg.text,
                };
                if let Ok(mut q) = self.message_queue.lock() {
                    q.push_back((channel_hash as i64, queue_text, "slack".into()));
                }

                self.poll_buf.clear();
                self.poll_buf.push(0xA3);
                self.poll_buf.push(0x00);
                self.poll_buf.push(cmd_type);
                self.poll_buf.push(0x01);
                encode_uint(&mut self.poll_buf, channel_hash);
                self.poll_buf.push(0x02);
                self.poll_buf.push(0x00);

                Some((RELAY_COMMAND, &self.poll_buf))
            }
            Err(_) => None,
        }
    }
}

/// Socket Mode connection — opens WebSocket, receives events, sends acks.
async fn run_socket_mode(
    app_token: &str,
    _bot_token: &str,
    in_tx: &mpsc::Sender<IncomingMessage>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use tokio_tungstenite::tungstenite;

    // Request a WebSocket URL from Slack.
    let client = reqwest::Client::new();
    let resp = client
        .post("https://slack.com/api/apps.connections.open")
        .bearer_auth(app_token)
        .send()
        .await?;

    let body: serde_json::Value = resp.json().await?;
    let ws_url = body["url"]
        .as_str()
        .ok_or("No WebSocket URL in response")?;

    log::info!("Slack Socket Mode connected");

    // Connect via WebSocket.
    let (mut ws, _) = tokio_tungstenite::connect_async(ws_url).await?;

    use futures_util::{SinkExt, StreamExt};

    while let Some(msg) = ws.next().await {
        let msg = msg?;
        if let tungstenite::Message::Text(text) = msg {
            let json: serde_json::Value = match serde_json::from_str(&text) {
                Ok(v) => v,
                Err(_) => continue,
            };

            let msg_type = json.get("type").and_then(|t| t.as_str()).unwrap_or("");

            // Acknowledge the envelope.
            if let Some(envelope_id) = json.get("envelope_id").and_then(|e| e.as_str()) {
                let ack = serde_json::json!({"envelope_id": envelope_id});
                let _ = ws.send(tungstenite::Message::Text(ack.to_string().into())).await;
            }

            if msg_type == "events_api" {
                if let Some(event) = json.pointer("/payload/event") {
                    let event_type = event.get("type").and_then(|t| t.as_str()).unwrap_or("");

                    if event_type == "message" {
                        // Ignore bot messages (prevent loops).
                        if event.get("bot_id").is_some() || event.get("subtype").is_some() {
                            continue;
                        }

                        let text = event.get("text").and_then(|t| t.as_str()).unwrap_or("");
                        let channel = event.get("channel").and_then(|c| c.as_str()).unwrap_or("");
                        let user = event.get("user").and_then(|u| u.as_str()).unwrap_or("");

                        if !text.is_empty() {
                            let _ = in_tx
                                .send(IncomingMessage {
                                    text: text.to_string(),
                                    channel: channel.to_string(),
                                    user: user.to_string(),
                                })
                                .await;
                        }
                    }
                }
            }
        }
    }

    Err("WebSocket closed".into())
}

fn classify_command(text: &str) -> u8 {
    let trimmed = text.trim();
    match trimmed {
        "/help" | "/start" => 1,
        "/ants" | "/bots" => 2,
        "/usage" => 3,
        "/new" => 6,
        "/status" => 7,
        s if s == "/cancel all" => 5,
        s if s == "/cancel" || s.starts_with("/cancel ") => 4,
        s if s == "/followup" || s.starts_with("/followup ") => 8,
        s if s.starts_with("/analyse ") || s.starts_with("/analyze ") => 9,
        "/reflect" => 10,
        _ => 0,
    }
}

fn hash_channel(channel: &str) -> u64 {
    // FNV-1a hash of the channel ID — fits in a u64 for use as chat_id.
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in channel.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
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
