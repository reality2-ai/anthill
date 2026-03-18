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
        .route("/api/ants", get(list_ants))
        .route("/api/ants/{id}/chat", post(send_chat))
        .route("/api/ants/{id}/cancel/{task_id}", post(cancel_task))
        .route("/api/ants/{id}/config", get(get_config).put(put_config))
        .route("/api/ants/create", post(create_ant))
        .route("/api/ants/{id}", axum::routing::delete(delete_ant))
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

/// Generate a stylised ant icon as SVG.
fn render_icon(size: u32) -> Vec<u8> {
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{s}" height="{s}" viewBox="0 0 512 512">
  <rect width="512" height="512" rx="96" fill="#1a1a2e"/>
  <!-- Antennae -->
  <line x1="220" y1="120" x2="190" y2="60" stroke="#4ade80" stroke-width="8" stroke-linecap="round"/>
  <line x1="292" y1="120" x2="322" y2="60" stroke="#4ade80" stroke-width="8" stroke-linecap="round"/>
  <circle cx="186" cy="56" r="10" fill="#4ade80"/>
  <circle cx="326" cy="56" r="10" fill="#4ade80"/>
  <!-- Head -->
  <ellipse cx="256" cy="148" rx="52" ry="44" fill="#e94560"/>
  <!-- Eyes -->
  <circle cx="238" cy="140" r="10" fill="#1a1a2e"/>
  <circle cx="274" cy="140" r="10" fill="#1a1a2e"/>
  <circle cx="240" cy="138" r="4" fill="#fff"/>
  <circle cx="276" cy="138" r="4" fill="#fff"/>
  <!-- Thorax -->
  <ellipse cx="256" cy="232" rx="42" ry="46" fill="#e94560"/>
  <!-- Abdomen -->
  <ellipse cx="256" cy="348" rx="64" ry="72" fill="#e94560"/>
  <!-- Abdomen stripes -->
  <ellipse cx="256" cy="320" rx="56" ry="8" fill="#c4314e" opacity="0.5"/>
  <ellipse cx="256" cy="350" rx="60" ry="8" fill="#c4314e" opacity="0.5"/>
  <ellipse cx="256" cy="380" rx="52" ry="8" fill="#c4314e" opacity="0.5"/>
  <!-- Legs left -->
  <line x1="224" y1="216" x2="150" y2="180" stroke="#e94560" stroke-width="8" stroke-linecap="round"/>
  <line x1="150" y1="180" x2="120" y2="220" stroke="#e94560" stroke-width="8" stroke-linecap="round"/>
  <line x1="218" y1="240" x2="140" y2="252" stroke="#e94560" stroke-width="8" stroke-linecap="round"/>
  <line x1="140" y1="252" x2="108" y2="290" stroke="#e94560" stroke-width="8" stroke-linecap="round"/>
  <line x1="220" y1="268" x2="148" y2="310" stroke="#e94560" stroke-width="8" stroke-linecap="round"/>
  <line x1="148" y1="310" x2="118" y2="356" stroke="#e94560" stroke-width="8" stroke-linecap="round"/>
  <!-- Legs right -->
  <line x1="288" y1="216" x2="362" y2="180" stroke="#e94560" stroke-width="8" stroke-linecap="round"/>
  <line x1="362" y1="180" x2="392" y2="220" stroke="#e94560" stroke-width="8" stroke-linecap="round"/>
  <line x1="294" y1="240" x2="372" y2="252" stroke="#e94560" stroke-width="8" stroke-linecap="round"/>
  <line x1="372" y1="252" x2="404" y2="290" stroke="#e94560" stroke-width="8" stroke-linecap="round"/>
  <line x1="292" y1="268" x2="364" y2="310" stroke="#e94560" stroke-width="8" stroke-linecap="round"/>
  <line x1="364" y1="310" x2="394" y2="356" stroke="#e94560" stroke-width="8" stroke-linecap="round"/>
  <!-- Label -->
  <text x="256" y="478" text-anchor="middle" font-family="Arial,sans-serif" font-weight="bold" font-size="48" fill="#4ade80">ANTHILL</text>
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

/// GET /api/ants — list all ants with status.
async fn list_ants(State(state): State<AppState>) -> impl IntoResponse {
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

/// GET /api/ants/:id/config — read an ANT's config.
async fn get_config(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.registry.read_config(&id) {
        Some(content) => (StatusCode::OK, content).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// PUT /api/ants/:id/config — update an ANT's config.
#[derive(Deserialize)]
struct ConfigUpdate {
    content: String,
}

async fn put_config(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<ConfigUpdate>,
) -> impl IntoResponse {
    // Validate it's valid TOML.
    if toml::from_str::<crate::config::Config>(&req.content).is_err() {
        return (StatusCode::BAD_REQUEST, "Invalid TOML config").into_response();
    }
    match state.registry.write_config(&id, &req.content) {
        Ok(()) => (StatusCode::OK, "Config saved. Restart the ANT to apply.").into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

/// POST /api/ants/create — create a new ANT.
#[derive(Deserialize)]
struct CreateAnt {
    id: String,
    name: String,
    token: String,
    working_dir: String,
    system_prompt: Option<String>,
}

async fn create_ant(
    State(state): State<AppState>,
    Json(req): Json<CreateAnt>,
) -> impl IntoResponse {
    // Validate id (alphanumeric + hyphens only).
    if req.id.is_empty()
        || !req.id.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        return (StatusCode::BAD_REQUEST, "Invalid ANT id. Use alphanumeric, hyphens, underscores.").into_response();
    }

    // Check it doesn't already exist.
    if state.registry.read_config(&req.id).is_some() {
        return (StatusCode::CONFLICT, "ANT already exists").into_response();
    }

    // Generate ant.toml.
    let prompt = req.system_prompt.unwrap_or_else(|| "You are a helpful assistant.".into());
    let config = format!(
        r#"name = "{name}"
mode = "claude"

[telegram]
token = "{token}"

[claude]
working_dir = "{working_dir}"
memory_dir = "memory"
repos_dir = "repos"
skip_permissions = true
backup_interval_hours = 6

system_prompt = """\
{prompt}"""
"#,
        name = req.name,
        token = req.token,
        working_dir = req.working_dir,
        prompt = prompt,
    );

    match state.registry.write_config(&req.id, &config) {
        Ok(()) => (StatusCode::CREATED, "ANT created. Restart Anthill to start it.").into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

/// DELETE /api/ants/:id — delete an ANT.
async fn delete_ant(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    // Stop the ANT if it's running (abort its tasks).
    {
        let bots = state.registry.bots.read().await;
        if let Some(handle) = bots.get(&id) {
            if let Ok(tasks) = handle.tasks.lock() {
                for task in tasks.values() {
                    task.handle.abort();
                }
            }
        }
    }
    // Remove from registry.
    state.registry.bots.write().await.remove(&id);

    // Delete config.
    match state.registry.delete_config(&id) {
        Ok(()) => (StatusCode::OK, "ANT deleted").into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
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
