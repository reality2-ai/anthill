//! Web server — serves the Anthill dashboard and WebSocket API.
//!
//! Embedded HTML served from a single binary. WebSocket streams
//! real-time bot events. REST API for bot listing and message sending.
//! Chat history persisted to disk, loaded on connect.

use axum::body::Bytes;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse};
use axum::routing::{get, post};
use axum::Json;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use crate::history::SharedHistory;
use crate::registry::BotRegistry;
use crate::trust::SharedTrust;

/// Shared state for Axum handlers.
#[derive(Clone)]
pub struct AppState {
    pub registry: Arc<BotRegistry>,
    pub history: SharedHistory,
    pub trust: SharedTrust,
    /// Channel to tell the supervisor to reload/spawn new ants.
    pub reload_tx: tokio::sync::mpsc::Sender<()>,
}

/// Embedded web app HTML.
const WEB_APP_HTML: &str = include_str!("web_app.html");

/// Start the web server.
pub async fn run_web_server(
    registry: Arc<BotRegistry>,
    history: SharedHistory,
    trust: SharedTrust,
    reload_tx: tokio::sync::mpsc::Sender<()>,
    bind: SocketAddr,
) {
    let state = AppState {
        registry,
        history,
        trust,
        reload_tx,
    };

    // Protected API routes — require credential in X-Credential header.
    let protected_api = axum::Router::new()
        .route("/api/ants", get(list_ants))
        .route("/api/ants/{id}/chat", post(send_chat))
        .route("/api/ants/{id}/cancel/{task_id}", post(cancel_task))
        .route("/api/ants/{id}/config", get(get_config).put(put_config))
        .route("/api/ants/create", post(create_ant))
        .route("/api/ants/{id}/files", get(list_files))
        .route("/api/ants/{id}/files/{*path}", get(get_file).delete(delete_file))
        .route("/api/ants/{id}/upload/{*path}", post(upload_file))
        .route("/api/ants/{id}", axum::routing::delete(delete_ant))
        .route("/api/ants/reload", post(reload_ants))
        .route("/api/backends", get(list_backends))
        .route("/api/auth/devices", get(auth_list_devices))
        .route("/api/auth/devices/{id}", axum::routing::delete(auth_revoke_device))
        .route("/api/auth/join-code", post(auth_generate_join_code))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    // Public routes — no auth required.
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
        .route("/logo.svg", get(logo))
        .route("/api/auth/verify", post(auth_verify))
        .route("/api/auth/join", post(auth_join))
        .route("/api/auth/status", get(auth_status))
        .merge(protected_api)
        .with_state(state);

    log::info!("Web server listening on {}", bind);

    let listener = match tokio::net::TcpListener::bind(bind).await {
        Ok(l) => l,
        Err(e) => {
            log::error!("Failed to bind web server to {}: {}", bind, e);
            return;
        }
    };
    if let Err(e) = axum::serve(listener, app).await {
        log::error!("Web server error: {}", e);
    }
}

/// GET / — serve the embedded web app.
async fn index() -> Html<&'static str> {
    Html(WEB_APP_HTML)
}

/// Embedded logo SVG.
const LOGO_SVG: &str = include_str!("../docs/logo.svg");

/// GET /logo.svg
async fn logo() -> impl IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "image/svg+xml")],
        LOGO_SVG,
    )
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

/// Generate a square icon: mound + lightbulb (matches the logo).
fn render_icon(size: u32) -> Vec<u8> {
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{s}" height="{s}" viewBox="0 0 512 512">
  <rect width="512" height="512" rx="96" fill="#1a1a2e"/>
  <!-- Mound -->
  <path d="M80,400 Q256,120 432,400" fill="#e94560"/>
  <!-- Entrance -->
  <ellipse cx="256" cy="396" rx="36" ry="22" fill="#1a1a2e"/>
  <!-- Lightbulb emerging from mound -->
  <path d="M230,240 Q218,210 234,185 Q256,165 278,185 Q294,210 282,240" fill="none" stroke="#4ade80" stroke-width="6"/>
  <line x1="234" y1="240" x2="278" y2="240" stroke="#4ade80" stroke-width="5"/>
  <line x1="238" y1="250" x2="274" y2="250" stroke="#4ade80" stroke-width="5"/>
  <!-- Glow -->
  <circle cx="256" cy="210" r="14" fill="#4ade80" opacity="0.3"/>
  <circle cx="256" cy="210" r="7" fill="#4ade80" opacity="0.6"/>
  <!-- Rays -->
  <line x1="256" y1="155" x2="256" y2="135" stroke="#4ade80" stroke-width="4" opacity="0.5" stroke-linecap="round"/>
  <line x1="212" y1="172" x2="198" y2="158" stroke="#4ade80" stroke-width="4" opacity="0.4" stroke-linecap="round"/>
  <line x1="300" y1="172" x2="314" y2="158" stroke="#4ade80" stroke-width="4" opacity="0.4" stroke-linecap="round"/>
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

/// GET /api/ants/:id/config — read an ANT's config as structured JSON.
async fn get_config(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.registry.read_config(&id) {
        Some(content) => {
            // Parse TOML and return as JSON for the form UI.
            match toml::from_str::<crate::config::Config>(&content) {
                Ok(cfg) => Json(serde_json::json!({
                    "name": cfg.name.unwrap_or_default(),
                    "telegram_token": cfg.telegram.token.unwrap_or_default(),
                    "telegram_allow": cfg.telegram.allow,
                    "slack_bot_token": cfg.slack.bot_token.unwrap_or_default(),
                    "slack_app_token": cfg.slack.app_token.unwrap_or_default(),
                    "working_dir": cfg.claude.working_dir.unwrap_or_default(),
                    "sync_channels": cfg.claude.sync_channels,
                    "backup_interval_hours": cfg.claude.backup_interval_hours,
                    "backup_remote": cfg.claude.backup_remote,
                    "system_prompt": cfg.claude.system_prompt.unwrap_or_default(),
                    "backends": cfg.claude.backends,
                })).into_response(),
                Err(_) => {
                    (StatusCode::OK, content).into_response()
                }
            }
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// PUT /api/ants/:id/config — update an ANT's config from structured fields.
#[derive(Deserialize)]
#[allow(dead_code)]
struct ConfigUpdate {
    name: Option<String>,
    backends: Option<Vec<String>>,
    telegram_token: Option<String>,
    telegram_allow: Option<Vec<i64>>,
    slack_bot_token: Option<String>,
    slack_app_token: Option<String>,
    working_dir: Option<String>,
    memory_dir: Option<String>,
    repos_dir: Option<String>,
    skip_permissions: Option<bool>,
    sync_channels: Option<bool>,
    backup_interval_hours: Option<u32>,
    backup_remote: Option<String>,
    system_prompt: Option<String>,
}

async fn put_config(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<ConfigUpdate>,
) -> impl IntoResponse {
    // Build a typed config and serialize to TOML.
    let cfg = crate::config::Config {
        name: req.name.clone().filter(|s| !s.is_empty()),
        mode: String::new(),
        telegram: crate::config::TelegramConfig {
            token: req.telegram_token.clone().filter(|s| !s.is_empty()),
            allow: req.telegram_allow.clone().unwrap_or_default(),
        },
        slack: crate::config::SlackConfig {
            bot_token: req.slack_bot_token.clone().filter(|s| !s.is_empty()),
            app_token: req.slack_app_token.clone().filter(|s| !s.is_empty()),
        },
        claude: crate::config::ClaudeConfig {
            backends: req.backends.clone().unwrap_or_else(|| vec!["claude".into()]),
            working_dir: req.working_dir.clone().filter(|s| !s.is_empty()),
            system_prompt: req.system_prompt.clone().filter(|s| !s.is_empty()),
            sync_channels: req.sync_channels.unwrap_or(false),
            backup_interval_hours: req.backup_interval_hours.unwrap_or(0),
            backup_remote: req.backup_remote.clone().unwrap_or_default(),
            ..Default::default()
        },
    };

    let toml = match toml::to_string_pretty(&cfg) {
        Ok(t) => t,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("Config error: {}", e)).into_response(),
    };

    match state.registry.write_config(&id, &toml) {
        Ok(()) => (StatusCode::OK, "Config saved. Restart Anthill to apply.").into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

/// POST /api/ants/create — create a new ANT.
#[derive(Deserialize)]
struct CreateAnt {
    id: String,
    name: Option<String>,
    token: Option<String>,
    working_dir: Option<String>,
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

    // Build typed config and serialize.
    let cfg = crate::config::Config {
        name: req.name.filter(|s| !s.is_empty()).or_else(|| Some(req.id.clone())),
        mode: String::new(),
        telegram: crate::config::TelegramConfig {
            token: req.token.filter(|s| !s.is_empty()),
            allow: Vec::new(),
        },
        slack: crate::config::SlackConfig::default(),
        claude: crate::config::ClaudeConfig {
            working_dir: req.working_dir.filter(|s| !s.is_empty()),
            system_prompt: req.system_prompt.filter(|s| !s.is_empty()),
            ..Default::default()
        },
    };

    let config = match toml::to_string_pretty(&cfg) {
        Ok(t) => t,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("Config error: {}", e)).into_response(),
    };

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

/// GET /api/backends — list available AI backends.
async fn list_backends() -> impl IntoResponse {
    let backends = crate::ai_worker::detect_backends();
    Json(backends.iter().map(|(name, installed)| {
        serde_json::json!({ "name": name, "installed": installed })
    }).collect::<Vec<_>>())
}

/// POST /api/ants/reload — tell the supervisor to scan for new ants.
async fn reload_ants(
    State(state): State<AppState>,
) -> impl IntoResponse {
    match state.reload_tx.send(()).await {
        Ok(()) => (StatusCode::OK, "Reload triggered").into_response(),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Reload channel closed").into_response(),
    }
}

// --- Auth middleware ---

/// Middleware that checks the X-Credential header on protected routes.
async fn auth_middleware(
    State(state): State<AppState>,
    req: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> impl IntoResponse {
    // Credential must be in X-Credential header. No query param fallback.
    let credential = req
        .headers()
        .get("x-credential")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    if check_auth(&state, &credential).is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    next.run(req).await.into_response()
}

// urlencoding module removed — credential no longer accepted via query params.
#[allow(dead_code)]
mod urlencoding {
    pub fn decode(s: &str) -> Result<String, ()> {
        Ok(s.to_string())
    }
}

// --- Authentication ---

/// Helper: check if a request is authenticated. Returns device name or error.
fn check_auth(state: &AppState, credential: &str) -> Result<String, StatusCode> {
    if credential.is_empty() {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let mut trust = state.trust.lock().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    match trust.authenticate(credential) {
        Some(device) => Ok(device.name),
        None => Err(StatusCode::UNAUTHORIZED),
    }
}

/// POST /api/auth/verify — check if a credential is valid.
#[derive(Deserialize)]
struct AuthVerify {
    credential: String,
}

async fn auth_verify(
    State(state): State<AppState>,
    Json(req): Json<AuthVerify>,
) -> impl IntoResponse {
    match check_auth(&state, &req.credential) {
        Ok(name) => Json(serde_json::json!({
            "authenticated": true,
            "device_name": name,
        })).into_response(),
        Err(_) => Json(serde_json::json!({
            "authenticated": false,
        })).into_response(),
    }
}

/// POST /api/auth/join — join the colony with a join code.
#[derive(Deserialize)]
struct AuthJoin {
    code: String,
    device_name: String,
}

async fn auth_join(
    State(state): State<AppState>,
    Json(req): Json<AuthJoin>,
) -> impl IntoResponse {
    let mut trust = match state.trust.lock() {
        Ok(t) => t,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };

    if !trust.verify_join_code(&req.code) {
        return (StatusCode::FORBIDDEN, "Invalid or expired join code").into_response();
    }

    let name = if req.device_name.is_empty() { "unnamed device" } else { &req.device_name };
    let device = trust.provision_device(name);
    log::info!("Device joined colony: '{}' ({})", device.name, &device.id[..8]);

    Json(serde_json::json!({
        "device_id": device.id,
        "credential": device.credential,
        "name": device.name,
    })).into_response()
}

/// GET /api/auth/devices — list all provisioned devices (auth via middleware).
async fn auth_list_devices(
    State(state): State<AppState>,
) -> impl IntoResponse {
    let trust = match state.trust.lock() {
        Ok(t) => t,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };

    let devices: Vec<_> = trust.list_devices().iter().map(|d| {
        serde_json::json!({
            "id": d.id,
            "name": d.name,
            "joined_at": d.joined_at,
            "last_seen": d.last_seen,
        })
    }).collect();

    Json(devices).into_response()
}

/// DELETE /api/auth/devices/:id — revoke a device (auth via middleware).
async fn auth_revoke_device(
    State(state): State<AppState>,
    Path(device_id): Path<String>,
) -> impl IntoResponse {
    let mut trust = match state.trust.lock() {
        Ok(t) => t,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR,
    };

    if trust.revoke_device(&device_id) {
        log::info!("Device revoked: {}", &device_id[..8.min(device_id.len())]);
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    }
}

/// POST /api/auth/join-code — generate a join code (auth via middleware).
async fn auth_generate_join_code(
    State(state): State<AppState>,
) -> impl IntoResponse {
    let mut trust = match state.trust.lock() {
        Ok(t) => t,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };

    let code = trust.generate_join_code();
    log::info!("Join code generated: {}", code);

    Json(serde_json::json!({
        "code": code,
        "expires_in_seconds": 300,
    })).into_response()
}

/// GET /api/auth/status — public: is the colony empty (queen bootstrap)?
async fn auth_status(
    State(state): State<AppState>,
) -> impl IntoResponse {
    let trust = match state.trust.lock() {
        Ok(t) => t,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    Json(serde_json::json!({
        "empty_colony": trust.is_empty_colony(),
    })).into_response()
}

// --- File management ---

#[derive(Serialize)]
struct FileEntry {
    name: String,
    is_dir: bool,
    size: u64,
}

/// Resolve a safe path within an ANT's working directory. Prevents traversal.
/// Resolve a safe path within an ANT's working directory.
/// Uses canonicalization to prevent symlink escapes and .. traversal.
async fn resolve_ant_path(
    registry: &crate::registry::BotRegistry,
    ant_id: &str,
    subpath: &str,
) -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    let bots = registry.bots.read().await;
    let handle = bots.get(ant_id)?;
    let base = handle.working_dir.clone();
    drop(bots);

    // Canonicalize the base to resolve any symlinks in the working dir itself.
    let canonical_base = std::fs::canonicalize(&base).ok()?;

    // Strip leading slashes, reject paths with null bytes.
    let clean = subpath.trim_start_matches('/');
    if clean.contains('\0') {
        return None;
    }

    let full = base.join(clean);

    // For existing paths: canonicalize and verify prefix.
    // For new paths (uploads): canonicalize the parent and verify prefix.
    if full.exists() {
        let canonical = std::fs::canonicalize(&full).ok()?;
        if !canonical.starts_with(&canonical_base) {
            log::warn!("Path traversal blocked: {} escapes {}", canonical.display(), canonical_base.display());
            return None;
        }
        Some((canonical_base, canonical))
    } else {
        // Path doesn't exist yet (upload). Check the parent.
        let parent = full.parent()?;
        if parent.exists() {
            let canonical_parent = std::fs::canonicalize(parent).ok()?;
            if !canonical_parent.starts_with(&canonical_base) {
                log::warn!("Path traversal blocked: {} escapes {}", canonical_parent.display(), canonical_base.display());
                return None;
            }
        }
        // Also verify the joined path lexically doesn't escape
        // (handles cases where parent doesn't exist yet).
        let normalized = canonical_base.join(clean);
        if !normalized.starts_with(&canonical_base) {
            return None;
        }
        Some((canonical_base, full))
    }
}

/// GET /api/ants/:id/files — list root workspace directories.
async fn list_files(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let bots = state.registry.bots.read().await;
    let handle = match bots.get(&id) {
        Some(h) => h,
        None => return StatusCode::NOT_FOUND.into_response(),
    };
    let base = handle.working_dir.clone();
    drop(bots);

    match read_dir_entries(&base) {
        Some(entries) => Json(entries).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// GET /api/ants/:id/files/{*path} — list directory or serve file.
async fn get_file(
    State(state): State<AppState>,
    Path((id, subpath)): Path<(String, String)>,
) -> impl IntoResponse {
    let (_, full) = match resolve_ant_path(&state.registry, &id, &subpath).await {
        Some(p) => p,
        None => return StatusCode::NOT_FOUND.into_response(),
    };

    if full.is_dir() {
        match read_dir_entries(&full) {
            Some(entries) => Json(entries).into_response(),
            None => StatusCode::NOT_FOUND.into_response(),
        }
    } else if full.is_file() {
        match std::fs::read(&full) {
            Ok(data) => {
                let content_type = guess_content_type(&full);
                ([(axum::http::header::CONTENT_TYPE, content_type)], data).into_response()
            }
            Err(_) => StatusCode::NOT_FOUND.into_response(),
        }
    } else {
        StatusCode::NOT_FOUND.into_response()
    }
}

/// POST /api/ants/:id/upload/{*path} — upload a file.
async fn upload_file(
    State(state): State<AppState>,
    Path((id, subpath)): Path<(String, String)>,
    body: Bytes,
) -> impl IntoResponse {
    let (_, full) = match resolve_ant_path(&state.registry, &id, &subpath).await {
        Some(p) => p,
        None => return StatusCode::NOT_FOUND.into_response(),
    };

    // Create parent directories.
    if let Some(parent) = full.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    match std::fs::write(&full, &body) {
        Ok(()) => {
            log::info!("[{}] uploaded: {}", id, subpath);
            StatusCode::CREATED.into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// DELETE /api/ants/:id/files/{*path} — delete a file or empty directory.
async fn delete_file(
    State(state): State<AppState>,
    Path((id, subpath)): Path<(String, String)>,
) -> impl IntoResponse {
    let (_, full) = match resolve_ant_path(&state.registry, &id, &subpath).await {
        Some(p) => p,
        None => return StatusCode::NOT_FOUND,
    };

    if full.is_file() {
        std::fs::remove_file(&full).map(|_| StatusCode::OK).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
    } else if full.is_dir() {
        std::fs::remove_dir(&full).map(|_| StatusCode::OK).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
    } else {
        StatusCode::NOT_FOUND
    }
}

fn read_dir_entries(dir: &std::path::Path) -> Option<Vec<FileEntry>> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut result: Vec<FileEntry> = entries
        .filter_map(|e| e.ok())
        .filter(|e| {
            // Hide .git directory.
            e.file_name().to_string_lossy() != ".git" && e.file_name().to_string_lossy() != ".gitignore"
        })
        .map(|e| {
            let meta = e.metadata().ok();
            FileEntry {
                name: e.file_name().to_string_lossy().to_string(),
                is_dir: meta.as_ref().map(|m| m.is_dir()).unwrap_or(false),
                size: meta.as_ref().map(|m| m.len()).unwrap_or(0),
            }
        })
        .collect();
    result.sort_by(|a, b| {
        b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name))
    });
    Some(result)
}

fn guess_content_type(path: &std::path::Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("html") | Some("htm") => "text/html",
        Some("css") => "text/css",
        Some("js") => "application/javascript",
        Some("json") => "application/json",
        Some("toml") => "text/plain",
        Some("md") => "text/markdown",
        Some("txt") | Some("log") => "text/plain",
        Some("rs") | Some("py") | Some("sh") | Some("rb") | Some("go") | Some("ts") | Some("c") | Some("h") => "text/plain",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("svg") => "image/svg+xml",
        Some("pdf") => "application/pdf",
        Some("zip") => "application/zip",
        Some("tar") => "application/x-tar",
        _ => "application/octet-stream",
    }
}

/// GET /ws — WebSocket upgrade (with credential + device_id in query string).
async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let credential = params.get("credential").cloned().unwrap_or_default();
    let device_id = params.get("device_id").cloned().unwrap_or_default();

    // Verify credential.
    if check_auth(&state, &credential).is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    ws.on_upgrade(move |socket| handle_ws(socket, state, credential, device_id))
        .into_response()
}

/// Handle a WebSocket connection with HMAC envelope support.
async fn handle_ws(
    mut socket: WebSocket,
    state: AppState,
    credential: String,
    _device_id: String,
) {
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

    // Helper: wrap outgoing message in a signed envelope.
    let send_signed = |payload: &str, cred: &str| -> String {
        let (sig, ts) = crate::trust::sign_message(cred, "server", payload);
        serde_json::json!({
            "device_id": "server",
            "timestamp": ts,
            "signature": sig,
            "payload": payload,
        }).to_string()
    };

    let snapshot = serde_json::json!({
        "type": "snapshot",
        "bots": bots,
        "history": history,
    });
    let snapshot_str = snapshot.to_string();
    let envelope = send_signed(&snapshot_str, &credential);
    if socket
        .send(Message::Text(envelope.into()))
        .await
        .is_err()
    {
        return;
    }

    // Stream events and handle incoming messages.
    loop {
        tokio::select! {
            // Broadcast event → sign and send to client.
            Ok(event) = rx.recv() => {
                let json = match serde_json::to_string(&event) {
                    Ok(j) => j,
                    Err(_) => continue,
                };
                let envelope = send_signed(&json, &credential);
                if socket.send(Message::Text(envelope.into())).await.is_err() {
                    break;
                }
            }

            // Client message → verify signature, then handle command.
            Some(Ok(msg)) = socket.recv() => {
                if let Message::Text(text) = msg {
                    // Try to parse as signed envelope first.
                    let payload_str = if let Ok(env) = serde_json::from_str::<WsEnvelope>(&text) {
                        // Verify HMAC signature.
                        if !crate::trust::verify_signature(
                            &credential, &env.device_id, env.timestamp, &env.payload, &env.signature,
                        ) {
                            log::warn!("WebSocket: invalid signature from device {}", env.device_id);
                            continue;
                        }
                        env.payload
                    } else {
                        // Unsigned message — accepted over HTTP where crypto.subtle is unavailable.
                        log::debug!("WebSocket: unsigned message received (HTTP mode)");
                        text.to_string()
                    };

                    if let Ok(cmd) = serde_json::from_str::<WsCommand>(&payload_str) {
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

/// Signed envelope for WebSocket messages.
#[derive(Deserialize)]
struct WsEnvelope {
    device_id: String,
    timestamp: u64,
    signature: String,
    payload: String,
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
