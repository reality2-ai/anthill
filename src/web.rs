//! Web server — serves the Anthill dashboard and WebSocket API.
//!
//! Embedded HTML served from a single binary. WebSocket streams
//! real-time bot events. REST API for bot listing and message sending.
//! Chat history persisted to disk, loaded on connect.

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse};
use axum::routing::{get, post};
use axum::Json;
use serde::Deserialize;
use std::net::SocketAddr;
use std::sync::Arc;

use crate::history::SharedHistory;
use crate::registry::BotRegistry;

/// Shared state for Axum handlers.
#[derive(Clone)]
pub struct AppState {
    pub registry: Arc<BotRegistry>,
    pub history: SharedHistory,
}

/// Embedded web app HTML.
const WEB_APP_HTML: &str = include_str!("web_app.html");

/// Start the web server.
pub async fn run_web_server(registry: Arc<BotRegistry>, history: SharedHistory, bind: SocketAddr) {
    let state = AppState {
        registry,
        history,
    };

    let app = axum::Router::new()
        .route("/", get(index))
        .route("/ws", get(ws_handler))
        .route("/manifest.json", get(manifest))
        .route("/sw.js", get(service_worker))
        .route("/icon.svg", get(icon_512))
        .route("/icon-192.svg", get(icon_192))
        .route("/icon-512.svg", get(icon_512))
        .route("/icon-192.png", get(icon_192))
        .route("/icon-512.png", get(icon_512))
        .route("/api/bots", get(list_bots))
        .route("/api/bots/{name}/chat", post(send_chat))
        .route("/api/bots/{name}/cancel/{task_id}", post(cancel_task))
        .with_state(state);

    log::info!("Web server listening on {}", bind);

    let listener = tokio::net::TcpListener::bind(bind).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

/// GET / — serve the embedded web app.
async fn index() -> Html<&'static str> {
    Html(WEB_APP_HTML)
}

/// GET /manifest.json — PWA manifest.
async fn manifest() -> impl IntoResponse {
    let json = serde_json::json!({
        "name": "Anthill",
        "short_name": "Anthill",
        "description": "AI-powered bots backed by Claude Code",
        "start_url": "/",
        "display": "standalone",
        "background_color": "#1a1a2e",
        "theme_color": "#1a1a2e",
        "icons": [
            { "src": "/icon.svg", "sizes": "any", "type": "image/svg+xml", "purpose": "any" },
            { "src": "/icon-192.svg", "sizes": "192x192", "type": "image/svg+xml", "purpose": "any maskable" },
            { "src": "/icon-512.svg", "sizes": "512x512", "type": "image/svg+xml", "purpose": "any maskable" }
        ]
    });
    (
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        json.to_string(),
    )
}

/// GET /sw.js — minimal service worker for PWA install.
async fn service_worker() -> impl IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "application/javascript")],
        "self.addEventListener('fetch', e => e.respondWith(fetch(e.request)));",
    )
}

/// Generate a PNG icon from an SVG at the given size.
fn render_icon(size: u32) -> Vec<u8> {
    // Simple SVG → return as SVG disguised as PNG is not ideal.
    // Instead, generate a minimal valid PNG with the R2 logo.
    // For now, return an SVG served with PNG content type — browsers handle it.
    // TODO: proper PNG generation or embedded PNG.
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{s}" height="{s}" viewBox="0 0 512 512">
  <rect width="512" height="512" rx="96" fill="#1a1a2e"/>
  <text x="256" y="300" text-anchor="middle" font-family="Arial,sans-serif" font-weight="bold" font-size="200" fill="#e94560">AH</text>
  <text x="256" y="420" text-anchor="middle" font-family="Arial,sans-serif" font-size="100" fill="#4ade80">IL</text>
</svg>"##,
        s = size
    );
    svg.into_bytes()
}

/// GET /icon-192.png
async fn icon_192() -> impl IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "image/svg+xml")],
        render_icon(192),
    )
}

/// GET /icon-512.png
async fn icon_512() -> impl IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "image/svg+xml")],
        render_icon(512),
    )
}

/// GET /api/bots — list all bots with status.
async fn list_bots(State(state): State<AppState>) -> impl IntoResponse {
    let bots = state.registry.list_bots().await;
    Json(bots)
}

/// POST /api/bots/:name/chat — send a message to a bot.
#[derive(Deserialize)]
struct ChatRequest {
    message: String,
    #[serde(default)]
    chat_id: i64,
}

async fn send_chat(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(req): Json<ChatRequest>,
) -> impl IntoResponse {
    if state.registry.send_message(&name, req.chat_id, req.message).await {
        StatusCode::ACCEPTED
    } else {
        StatusCode::NOT_FOUND
    }
}

/// POST /api/bots/:name/cancel/:task_id — cancel a running task.
async fn cancel_task(
    State(state): State<AppState>,
    Path((name, task_id)): Path<(String, u32)>,
) -> impl IntoResponse {
    let bots = state.registry.bots.read().await;
    if let Some(handle) = bots.get(&name) {
        if let Ok(mut tasks) = handle.tasks.lock() {
            if let Some(task) = tasks.remove(&task_id) {
                task.handle.abort();
                return StatusCode::OK;
            }
        }
    }
    StatusCode::NOT_FOUND
}

/// GET /ws — WebSocket upgrade.
async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, state))
}

/// Handle a WebSocket connection.
async fn handle_ws(mut socket: WebSocket, state: AppState) {
    let registry = &state.registry;

    // Subscribe to the global event broadcast.
    let mut rx = registry.global_tx.subscribe();

    // Send initial snapshot with bots and chat history.
    let bots = registry.list_bots().await;
    let bot_ids: Vec<String> = bots.iter().map(|b| b.id.clone()).collect();

    let history = if let Ok(mut h) = state.history.lock() {
        h.all_history(&bot_ids)
    } else {
        std::collections::HashMap::new()
    };

    let snapshot = serde_json::json!({
        "type": "snapshot",
        "bots": bots,
        "history": history,
    });
    if socket
        .send(Message::Text(snapshot.to_string().into()))
        .await
        .is_err()
    {
        return;
    }

    // Stream events and handle incoming messages.
    loop {
        tokio::select! {
            // Broadcast event → send to client.
            Ok(event) = rx.recv() => {
                let json = match serde_json::to_string(&event) {
                    Ok(j) => j,
                    Err(_) => continue,
                };
                if socket.send(Message::Text(json.into())).await.is_err() {
                    break;
                }
            }

            // Client message → handle command.
            Some(Ok(msg)) = socket.recv() => {
                if let Message::Text(text) = msg {
                    if let Ok(cmd) = serde_json::from_str::<WsCommand>(&text) {
                        match cmd {
                            WsCommand::Chat { bot, message, chat_id } => {
                                registry.send_message(&bot, chat_id.unwrap_or(0), message).await;
                            }
                            WsCommand::Cancel { bot, task_id } => {
                                let bots = registry.bots.read().await;
                                if let Some(handle) = bots.get(&bot) {
                                    if let Ok(mut tasks) = handle.tasks.lock() {
                                        if let Some(task) = tasks.remove(&task_id) {
                                            task.handle.abort();
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            else => break,
        }
    }
}

/// Commands received from WebSocket clients.
#[derive(Deserialize)]
#[serde(tag = "type")]
enum WsCommand {
    #[serde(rename = "chat")]
    Chat {
        bot: String,
        message: String,
        chat_id: Option<i64>,
    },
    #[serde(rename = "cancel")]
    Cancel { bot: String, task_id: u32 },
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
