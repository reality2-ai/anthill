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

use crate::ai_backends::AiBackend;
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
    /// Global backend registry — built from default config at startup.
    pub backend_registry: Arc<crate::ai_backends::BackendRegistry>,
}

/// Embedded web app HTML.
const WEB_APP_HTML: &str = include_str!("web_app.html");

/// Start the web server.
pub async fn run_web_server(
    registry: Arc<BotRegistry>,
    history: SharedHistory,
    trust: SharedTrust,
    reload_tx: tokio::sync::mpsc::Sender<()>,
    backend_registry: Arc<crate::ai_backends::BackendRegistry>,
    bind: SocketAddr,
) {
    let state = AppState {
        registry,
        history,
        trust,
        reload_tx,
        backend_registry,
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
        .route("/api/ants/{id}/upload/{*path}", post(upload_file)
            .layer(axum::extract::DefaultBodyLimit::max(MAX_UPLOAD_BYTES)))
        .route("/api/ants/{id}", axum::routing::delete(delete_ant))
        .route("/api/ants/reload", post(reload_ants))
        .route("/api/ants/{id}/restart", post(restart_ant))
        .route("/api/ants/{id}/compact-history", post(compact_history))
        .route("/api/ants/{id}/graph", get(get_graph))
        .route("/api/ants/{id}/export", get(export_graph))
        .route("/api/ants/{id}/rumination", get(get_rumination_log))
        .route("/api/ants/{id}/engine", get(get_engine_info))
        .route("/api/backends", get(list_backends))
        .route("/api/doctor", get(doctor_check))
        .route("/api/auth/devices", get(auth_list_devices))
        .route("/api/auth/devices/{id}", axum::routing::delete(auth_revoke_device))
        .route("/api/auth/qr-join", get(auth_qr_join))
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
        .route("/vendor/three.min.js", get(vendor_three))
        .route("/vendor/three-spritetext.min.js", get(vendor_spritetext))
        .route("/vendor/3d-force-graph.min.js", get(vendor_force_graph))
        .route("/vendor/force-graph.min.js", get(vendor_force_graph_2d))
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

// --- Vendor libraries (embedded) ---

const VENDOR_THREE: &str = include_str!("vendor/three.min.js");
const VENDOR_SPRITETEXT: &str = include_str!("vendor/three-spritetext.min.js");
const VENDOR_FORCE_GRAPH: &str = include_str!("vendor/3d-force-graph.min.js");
const VENDOR_FORCE_GRAPH_2D: &str = include_str!("vendor/force-graph.min.js");

async fn vendor_three() -> impl IntoResponse {
    ([(axum::http::header::CONTENT_TYPE, "application/javascript")], VENDOR_THREE)
}

async fn vendor_force_graph() -> impl IntoResponse {
    ([(axum::http::header::CONTENT_TYPE, "application/javascript")], VENDOR_FORCE_GRAPH)
}

async fn vendor_force_graph_2d() -> impl IntoResponse {
    ([(axum::http::header::CONTENT_TYPE, "application/javascript")], VENDOR_FORCE_GRAPH_2D)
}

async fn vendor_spritetext() -> impl IntoResponse {
    ([(axum::http::header::CONTENT_TYPE, "application/javascript")], VENDOR_SPRITETEXT)
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
                    "allow_base_code_changes": cfg.claude.allow_base_code_changes,
                    "backup_interval_hours": cfg.claude.backup_interval_hours,
                    "backup_remote": cfg.claude.backup_remote,
                    "system_prompt": cfg.claude.system_prompt.unwrap_or_default(),
                    "backend_strategy": cfg.claude.backend_strategy,
                    "backends": cfg.claude.backends,
                    "ai_default_category": cfg.ai.default_category,
                    "rumination": {
                        "enabled": cfg.claude.rumination.enabled,
                        "interval_secs": cfg.claude.rumination.interval_secs,
                        "refutation_enabled": cfg.claude.rumination.refutation_enabled,
                        "synthesis_enabled": cfg.claude.rumination.synthesis_enabled,
                        "contradiction_resolution": cfg.claude.rumination.contradiction_resolution,
                        "initiative_enabled": cfg.claude.rumination.initiative_enabled,
                        "min_idle_secs": cfg.claude.rumination.min_idle_secs,
                        "topics": cfg.claude.rumination.topics,
                    },
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
    backend_strategy: Option<crate::config::BackendStrategy>,
    backends: Option<Vec<String>>,
    ai_default_category: Option<String>,
    telegram_token: Option<String>,
    telegram_allow: Option<Vec<i64>>,
    slack_bot_token: Option<String>,
    slack_app_token: Option<String>,
    working_dir: Option<String>,
    memory_dir: Option<String>,
    repos_dir: Option<String>,
    skip_permissions: Option<bool>,
    sync_channels: Option<bool>,
    allow_base_code_changes: Option<bool>,
    backup_interval_hours: Option<u32>,
    backup_remote: Option<String>,
    system_prompt: Option<String>,
    rumination: Option<crate::config::RuminationConfig>,
}

async fn put_config(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<ConfigUpdate>,
) -> impl IntoResponse {
    // Load existing config so we only overwrite fields the UI controls.
    // This preserves [ai.categories], [ai.backends_config], memory_dir, etc.
    let mut cfg: crate::config::Config = state.registry.read_config(&id)
        .and_then(|c| toml::from_str(&c).ok())
        .unwrap_or_default();

    // Apply web UI updates on top of existing config.
    cfg.name = req.name.clone().filter(|s| !s.is_empty());
    cfg.telegram.token = req.telegram_token.clone().filter(|s| !s.is_empty());
    cfg.telegram.allow = req.telegram_allow.clone().unwrap_or_default();
    cfg.slack.bot_token = req.slack_bot_token.clone().filter(|s| !s.is_empty());
    cfg.slack.app_token = req.slack_app_token.clone().filter(|s| !s.is_empty());
    cfg.claude.backend_strategy = req.backend_strategy.clone().unwrap_or_default();
    cfg.claude.backends = req.backends.clone().unwrap_or_default();
    if let Some(ref wd) = req.working_dir {
        if !wd.is_empty() { cfg.claude.working_dir = Some(wd.clone()); }
    }
    cfg.claude.system_prompt = req.system_prompt.clone().filter(|s| !s.is_empty());
    cfg.claude.sync_channels = req.sync_channels.unwrap_or(false);
    cfg.claude.allow_base_code_changes = req.allow_base_code_changes.unwrap_or(false);
    cfg.claude.backup_interval_hours = req.backup_interval_hours.unwrap_or(0);
    cfg.claude.backup_remote = req.backup_remote.clone().unwrap_or_default();
    if let Some(ref rum) = req.rumination {
        cfg.claude.rumination = rum.clone();
    }
    // Update AI category — preserve existing categories/backends_config.
    if let Some(ref cat) = req.ai_default_category {
        cfg.ai.default_category = cat.clone();
    }

    let toml = match toml::to_string_pretty(&cfg) {
        Ok(t) => t,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("Config error: {}", e)).into_response(),
    };

    match state.registry.write_config(&id, &toml) {
        Ok(()) => {
            // Trigger reload so the ANT picks up the new config immediately.
            let _ = state.reload_tx.send(()).await;
            (StatusCode::OK, "Config saved and reloaded.").into_response()
        }
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
        ..Default::default()
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
    let backends = crate::ai_backends::detect_all_backends();
    Json(backends.iter().map(|(id, name, installed)| {
        serde_json::json!({ "id": id, "name": name, "installed": installed })
    }).collect::<Vec<_>>())
}

/// GET /api/ants/:id/engine — get current AI engine selection and ordered fallback list.
/// Optional ?category=X to preview what backends would be used for a different category.
async fn get_engine_info(
    State(state): State<AppState>,
    Path(id): Path<String>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let config_content = match state.registry.read_config(&id) {
        Some(c) => c,
        None => return StatusCode::NOT_FOUND.into_response(),
    };

    let cfg: crate::config::Config = match toml::from_str(&config_content) {
        Ok(c) => c,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };

    let selector = if let Some(preview) = params.get("category") {
        preview.clone()
    } else if !cfg.ai.default_category.is_empty() {
        cfg.ai.default_category.clone()
    } else if !cfg.ai.backends.is_empty() {
        cfg.ai.backends.join(",")
    } else {
        cfg.claude.backend_strategy.to_string()
    };

    let resolved = state.backend_registry.resolve(&selector);
    let ordered_backends: Vec<serde_json::Value> = resolved.iter().map(|b: &Arc<dyn AiBackend>| {
        serde_json::json!({
            "id": b.id(),
            "name": b.name(),
            "quality_tier": b.tags().quality_tier,
            "cost_tier": b.tags().cost_tier,
            "categories": b.tags().categories.iter().map(|c: &crate::ai_backends::EngineCategory| c.to_string()).collect::<Vec<String>>(),
        })
    }).collect();

    Json(serde_json::json!({
        "selected": selector,
        "ordered_backends": ordered_backends,
    })).into_response()
}

/// GET /api/doctor — run diagnostic checks.
async fn doctor_check() -> impl IntoResponse {
    let checks = crate::run_doctor_checks();
    Json(checks)
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

/// GET /api/ants/:id/graph?name=<topic> — return a knowledge graph in 3D visualization format.
/// If name is empty or "meta", returns the meta-graph. Otherwise loads memory/graphs/<name>.json.
async fn get_graph(
    State(state): State<AppState>,
    Path(id): Path<String>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let bots = state.registry.bots.read().await;
    let handle = match bots.get(&id) {
        Some(h) => h,
        None => return StatusCode::NOT_FOUND.into_response(),
    };

    let memory_dir = handle.working_dir.join("memory");
    drop(bots);

    let graph_name = params.get("name").map(|s| s.as_str()).unwrap_or("");
    let graph_key = if graph_name.is_empty() { "meta" } else { graph_name };

    let store = crate::store::live::LiveKnowledgeStore::new(memory_dir);

    // List available graphs.
    let available = match store.list_graphs() {
        Ok(graphs) => graphs.into_iter().map(|g| g.name).collect::<Vec<_>>(),
        Err(_) => vec!["meta".into()],
    };

    // Get visualization.
    use crate::store::KnowledgeStore;
    let mut viz = match store.to_visualization(graph_key) {
        Ok(v) => v,
        Err(_) => serde_json::json!({"nodes": [], "links": []}),
    };

    if let Some(obj) = viz.as_object_mut() {
        obj.insert("available_graphs".into(), serde_json::json!(available));
        obj.insert("current_graph".into(), serde_json::json!(graph_key));
    }
    Json(viz).into_response()
}

/// GET /api/ants/:id/rumination — get the rumination log.
async fn get_rumination_log(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let bots = state.registry.bots.read().await;
    let handle = match bots.get(&id) {
        Some(h) => h,
        None => return StatusCode::NOT_FOUND.into_response(),
    };
    let memory_dir = handle.working_dir.join("memory");
    drop(bots);

    let log = crate::maintenance::RuminationLog::load(&memory_dir);
    Json(serde_json::json!({
        "entries": log.entries,
        "count": log.entries.len(),
    })).into_response()
}

/// GET /api/ants/:id/export — download a self-contained HTML knowledge graph snapshot.
/// Optional query param: ?graph=<name> to export just one graph (default: all).
async fn export_graph(
    State(state): State<AppState>,
    Path(id): Path<String>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let bots = state.registry.bots.read().await;
    let handle = match bots.get(&id) {
        Some(h) => h,
        None => return StatusCode::NOT_FOUND.into_response(),
    };
    let memory_dir = handle.working_dir.join("memory");
    let display_name = handle.display_name.clone();
    drop(bots);

    let graph_filter = params.get("graph").cloned().filter(|g| !g.is_empty());
    let guidance = params.get("guidance").cloned().filter(|g| !g.is_empty());
    let include_citations = params.get("citations").map(|c| c != "false").unwrap_or(true);

    // Generate globally unique UUID for this snapshot.
    let uuid = uuid::Uuid::new_v4().to_string();

    let filename = if let Some(ref g) = graph_filter {
        format!("{}-{}-{}.html", id, g, uuid)
    } else {
        format!("{}-{}.html", id, uuid)
    };

    // Generate the export HTML.
    let tmp_path = std::env::temp_dir().join(&filename);
    let export_result = if let Some(ref graph_name) = graph_filter {
        crate::export::export_single_graph(&memory_dir, &display_name, graph_name, &tmp_path, guidance.as_deref(), include_citations)
    } else {
        crate::export::export_ant_graphs(&memory_dir, &display_name, &tmp_path, guidance.as_deref(), include_citations)
    };
    match export_result {
        Ok(()) => {
            match std::fs::read(&tmp_path) {
                Ok(html_bytes) => {
                    let _ = std::fs::remove_file(&tmp_path);
                    (
                        StatusCode::OK,
                        [
                            ("Content-Type", "text/html; charset=utf-8"),
                            ("Content-Disposition", &format!("attachment; filename=\"{}\"", filename)),
                        ],
                        html_bytes,
                    ).into_response()
                }
                Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Read error: {}", e)).into_response(),
            }
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Export error: {}", e)).into_response(),
    }
}

/// POST /api/ants/:id/compact-history — trim chat history to last 4 messages.
async fn compact_history(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    if let Ok(mut h) = state.history.lock() {
        let msgs = h.get_history(&id);
        if msgs.len() > 4 {
            let compacted = msgs.len() - 4;
            let kept: Vec<_> = msgs.iter().rev().take(4).rev().cloned().collect();
            let mut new_msgs = vec![crate::history::ChatMessage {
                role: "system".into(),
                text: format!("{} earlier messages compacted to knowledge graph.", compacted),
                task_id: 0,
                timestamp: crate::trust::now_secs(),
            }];
            new_msgs.extend(kept);
            h.replace_history(&id, new_msgs);
        }
    }
    StatusCode::OK
}

/// POST /api/ants/:id/restart — stop an ANT and trigger reload to restart with new config.
async fn restart_ant(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    // Stop the ANT: abort all tasks, remove from registry.
    {
        let mut bots = state.registry.bots.write().await;
        if let Some(handle) = bots.remove(&id) {
            if let Ok(tasks) = handle.tasks.lock() {
                for task in tasks.values() {
                    task.handle.abort();
                }
            }
            log::info!("ANT '{}' stopped for restart", id);
        }
    }

    // Signal the supervisor to reload — it will re-discover and re-spawn the ANT.
    match state.reload_tx.send(()).await {
        Ok(()) => {
            let _ = state.registry.global_tx.send(
                crate::registry::WsEvent::BotStatus {
                    bot: id.clone(),
                    status: "restarting".into(),
                }
            );
            (StatusCode::OK, format!("ANT '{}' restarting with new config", id)).into_response()
        }
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

    let name = if req.device_name.is_empty() { "unnamed device" } else { &req.device_name };

    // Validate, consume, and provision in one step.
    match trust.join_with_code(&req.code, name) {
        Some(device) => {
            log::info!("Device joined colony: '{}' ({})", device.name, &device.id[..8]);
            Json(serde_json::json!({
                "device_id": device.id,
                "credential": device.credential,
                "name": device.name,
            })).into_response()
        }
        None => {
            (StatusCode::FORBIDDEN, "Invalid or expired join code").into_response()
        }
    }
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

/// GET /api/auth/qr-join — generate join code + QR SVG (auth via middleware).
async fn auth_qr_join(
    State(state): State<AppState>,
    req: axum::http::Request<axum::body::Body>,
) -> impl IntoResponse {
    let mut trust = match state.trust.lock() {
        Ok(t) => t,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };

    let code = trust.generate_join_code();
    log::info!("QR join code generated: {}", code);

    // Build URL from the request's Host header.
    let host = req.headers()
        .get("host")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("localhost:3000");
    let url = format!("http://{}/#join={}", host, code);

    // Render QR as SVG.
    let svg = match qrcode::QrCode::new(url.as_bytes()) {
        Ok(qr) => {
            let image = qr.render::<qrcode::render::svg::Color>()
                .min_dimensions(200, 200)
                .dark_color(qrcode::render::svg::Color("#000000"))
                .light_color(qrcode::render::svg::Color("#ffffff"))
                .build();
            image
        }
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };

    Json(serde_json::json!({
        "code": code,
        "url": url,
        "svg": svg,
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

/// Maximum upload size: 50 MiB.
const MAX_UPLOAD_BYTES: usize = 50 * 1024 * 1024;

/// Extract a zip file into a directory using the system `unzip` command.
fn extract_zip(data: &[u8], dest: &std::path::Path) -> Result<usize, String> {
    let _ = std::fs::create_dir_all(dest);

    // Write zip to a temp file.
    let tmp = dest.join(".upload.zip.tmp");
    std::fs::write(&tmp, data).map_err(|e| format!("write tmp: {}", e))?;

    // Extract using unzip.
    let output = std::process::Command::new("unzip")
        .args(["-o", "-q"])  // overwrite, quiet
        .arg(&tmp)
        .arg("-d")
        .arg(dest)
        .output()
        .map_err(|e| format!("unzip not found: {}", e))?;

    let _ = std::fs::remove_file(&tmp);

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("unzip failed: {}", stderr));
    }

    // Count extracted files.
    let stdout = String::from_utf8_lossy(&output.stdout);
    let count = stdout.lines().filter(|l| l.contains("inflating") || l.contains("extracting")).count();
    // If quiet mode hides output, count files in dest that are newer than 5 seconds.
    let count = if count == 0 {
        walkdir(dest)
    } else {
        count
    };

    Ok(count)
}

/// Count files in a directory recursively.
fn walkdir(dir: &std::path::Path) -> usize {
    let mut count = 0;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                count += walkdir(&path);
            } else {
                count += 1;
            }
        }
    }
    count
}

/// POST /api/ants/:id/upload/{*path} — upload a file.
async fn upload_file(
    State(state): State<AppState>,
    Path((id, subpath)): Path<(String, String)>,
    body: Bytes,
) -> impl IntoResponse {
    if body.len() > MAX_UPLOAD_BYTES {
        return (StatusCode::PAYLOAD_TOO_LARGE, "Upload exceeds 50 MiB limit").into_response();
    }

    let (_, full) = match resolve_ant_path(&state.registry, &id, &subpath).await {
        Some(p) => p,
        None => return StatusCode::NOT_FOUND.into_response(),
    };

    // Create parent directories.
    if let Some(parent) = full.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            log::warn!("Failed to create parent dirs for upload: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to create directories").into_response();
        }
    }

    // Check if it's a zip file — extract contents into the target directory.
    let is_zip = subpath.ends_with(".zip") || (body.len() > 4 && body[0..4] == [0x50, 0x4B, 0x03, 0x04]);

    if is_zip {
        // Extract zip into the parent directory of the target path.
        let extract_dir = full.parent().unwrap_or(&full);
        match extract_zip(&body, extract_dir) {
            Ok(count) => {
                log::info!("[{}] uploaded and extracted zip: {} ({} files)", id, subpath, count);
                (StatusCode::CREATED, format!("Extracted {} files", count)).into_response()
            }
            Err(e) => {
                log::warn!("[{}] zip extraction failed: {}", id, e);
                // Fall back to saving as raw file.
                match std::fs::write(&full, &body) {
                    Ok(()) => StatusCode::CREATED.into_response(),
                    Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
                }
            }
        }
    } else {
        match std::fs::write(&full, &body) {
            Ok(()) => {
                log::info!("[{}] uploaded: {} ({} bytes)", id, subpath, body.len());
                StatusCode::CREATED.into_response()
            }
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        }
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

    // Collect active tasks per bot.
    let mut active_tasks: HashMap<String, Vec<serde_json::Value>> = HashMap::new();
    {
        let bots_map = registry.bots.read().await;
        for (name, handle) in bots_map.iter() {
            if let Ok(tasks) = handle.tasks.lock() {
                let task_list: Vec<serde_json::Value> = tasks.values().map(|t| {
                    let elapsed = t.started.elapsed().as_secs();
                    let progress = t.last_progress.lock().ok()
                        .and_then(|p| p.clone())
                        .unwrap_or_default();
                    let backend = t.backend.lock()
                        .map(|b| b.clone())
                        .unwrap_or_default();
                    serde_json::json!({
                        "task_id": t.task_id,
                        "preview": t.message_preview,
                        "elapsed_secs": elapsed,
                        "progress": progress,
                        "backend": backend,
                    })
                }).collect();
                if !task_list.is_empty() {
                    active_tasks.insert(name.clone(), task_list);
                }
            }
        }
    }

    let snapshot = serde_json::json!({
        "type": "snapshot",
        "bots": bots,
        "history": history,
        "tasks": active_tasks,
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
            result = rx.recv() => {
                match result {
                    Ok(event) => {
                        let json = match serde_json::to_string(&event) {
                            Ok(j) => j,
                            Err(_) => continue,
                        };
                        let envelope = send_signed(&json, &credential);
                        if socket.send(Message::Text(envelope.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        log::warn!("WebSocket client lagged — dropped {} events", n);
                        // Notify the client that events were missed.
                        let warning = serde_json::json!({
                            "type": "lag_warning",
                            "dropped": n,
                            "message": format!("Connection fell behind — {} events dropped. Refresh for current state.", n),
                        });
                        let envelope = send_signed(&warning.to_string(), &credential);
                        let _ = socket.send(Message::Text(envelope.into())).await;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
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
                                let cid = chat_id.unwrap_or(0);
                                let handled = handle_web_command(
                                    registry, &bot, cid, &message,
                                ).await;
                                if !handled {
                                    // Regular message → dispatch to AI worker.
                                    let sent = registry.send_message(&bot, cid, message).await;
                                    if !sent {
                                        // ANT not running — tell the user.
                                        let status = {
                                            let bots = registry.bots.read().await;
                                            if bots.contains_key(&bot) {
                                                "stopped or crashed"
                                            } else {
                                                "not running (configured but not started)"
                                            }
                                        };
                                        let _ = registry.global_tx.send(
                                            crate::registry::WsEvent::Message {
                                                bot: bot.clone(),
                                                chat_id: cid,
                                                text: format!(
                                                    "⚠️ ANT '{}' is {} — message not delivered.\n\n\
                                                    Check the server logs or restart the service.",
                                                    bot, status
                                                ),
                                                task_id: 0,
                                            }
                                        );
                                    }
                                }
                            }
                            WsCommand::Cancel { bot, task_id } => {
                                let bots = registry.bots.read().await;
                                if let Some(handle) = bots.get(&bot) {
                                    let cancelled = if let Ok(mut tasks) = handle.tasks.lock() {
                                        if let Some(task) = tasks.remove(&task_id) {
                                            task.handle.abort();
                                            true
                                        } else { false }
                                    } else { false };
                                    if cancelled {
                                        // Broadcast so the web UI removes the task card.
                                        let _ = registry.global_tx.send(
                                            crate::registry::WsEvent::TaskCompleted {
                                                bot: bot.clone(),
                                                task_id,
                                                duration_secs: 0,
                                            }
                                        );
                                        let _ = registry.global_tx.send(
                                            crate::registry::WsEvent::Message {
                                                bot: bot.clone(),
                                                chat_id: 0,
                                                text: format!("Cancelled task #{}.", task_id),
                                                task_id,
                                            }
                                        );
                                    }
                                }
                            }
                            WsCommand::FollowUp { bot, task_id, message } => {
                                let bots = registry.bots.read().await;
                                if let Some(handle) = bots.get(&bot) {
                                    if let Ok(mut fq) = handle.follow_ups.lock() {
                                        fq.entry(task_id).or_default().push(
                                            crate::ai_worker::FollowUp {
                                                chat_id: 0,
                                                message,
                                                source: "web".into(),
                                            }
                                        );
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

/// Handle local commands from the web (bypass AI worker).
/// Returns true if the command was handled locally, false if it should go to the worker.
async fn handle_web_command(
    registry: &BotRegistry,
    bot_name: &str,
    chat_id: i64,
    message: &str,
) -> bool {
    let trimmed = message.trim();
    let bots = registry.bots.read().await;
    let handle = match bots.get(bot_name) {
        Some(h) => h,
        None => return false,
    };

    let response = match trimmed {
        "/help" | "/start" => Some(
            "**anthill commands:**\n\n\
            /help — show this message\n\
            /status — live view of each worker\n\
            /ants — show running workers\n\
            /usage — session statistics\n\
            /cancel — cancel a running task\n\
            /followup — queue context for running task\n\
            /new — fresh conversation\n\
            /analyse <file> — thematic analysis → knowledge graph\n\
            /reflect — review and consolidate knowledge graph\n\
            /ruminate — trigger a rumination cycle now\n\
            /citations — resolve unknown citations and link to topic graphs\n\
            /questions — show pending questions from rumination\n\
            /ask <ant> <topic> — query another ANT's knowledge\n\
            /export — download knowledge graph as shareable HTML\n\
            /specify <file> — generate spec from code\n\
            /test-vectors <file> — generate test cases\n\n\
            Everything else is sent as a prompt to the AI.".into()
        ),
        "/ants" => {
            let tasks = handle.tasks.lock().ok();
            match tasks {
                Some(map) if map.is_empty() => Some("All workers idle.".into()),
                Some(map) => {
                    let mut out = format!("**{} worker{} active:**\n\n", map.len(), if map.len() == 1 { "" } else { "s" });
                    let mut sorted: Vec<_> = map.values().collect();
                    sorted.sort_by_key(|t| t.task_id);
                    for task in sorted {
                        let elapsed = task.started.elapsed().as_secs();
                        let time = if elapsed < 60 { format!("{}s", elapsed) }
                            else { format!("{}m {}s", elapsed / 60, elapsed % 60) };
                        out.push_str(&format!("**#{}** — {}\n  Working for {}\n\n",
                            task.task_id, task.message_preview, time));
                    }
                    Some(out)
                }
                None => Some("Status unavailable.".into()),
            }
        },
        "/status" => {
            let tasks = handle.tasks.lock().ok();
            match tasks {
                Some(map) if map.is_empty() => Some("All workers idle.".into()),
                Some(map) => {
                    let mut out = format!("**{} worker{} active:**\n\n", map.len(), if map.len() == 1 { "" } else { "s" });
                    let mut sorted: Vec<_> = map.values().collect();
                    sorted.sort_by_key(|t| t.task_id);
                    for task in sorted {
                        let elapsed = task.started.elapsed().as_secs();
                        let time = if elapsed < 60 { format!("{}s", elapsed) }
                            else { format!("{}m {}s", elapsed / 60, elapsed % 60) };
                        let backend = task.backend.lock()
                            .map(|b| if b.is_empty() { "starting".into() } else { b.clone() })
                            .unwrap_or_else(|_| "?".into());
                        let progress = task.last_progress.lock().ok()
                            .and_then(|p| p.clone())
                            .unwrap_or_else(|| "waiting...".into());
                        out.push_str(&format!("**#{}** [{}] — {}\n  ⏱ {}  → {}\n\n",
                            task.task_id, backend, task.message_preview, time, progress));
                    }
                    Some(out)
                }
                None => Some("Status unavailable.".into()),
            }
        },
        s if s == "/cancel" || s.starts_with("/cancel ") => {
            let mut tasks = match handle.tasks.lock() {
                Ok(m) => m,
                Err(_) => return true,
            };
            let mut cancelled_ids = Vec::new();
            let msg = if trimmed == "/cancel all" {
                let count = tasks.len();
                for task in tasks.values() {
                    task.handle.abort();
                    cancelled_ids.push(task.task_id);
                }
                tasks.clear();
                format!("Cancelled {} task(s).", count)
            } else if trimmed == "/cancel" {
                let latest = tasks.values().max_by_key(|t| t.task_id).map(|t| t.task_id);
                if let Some(id) = latest {
                    if let Some(task) = tasks.remove(&id) {
                        task.handle.abort();
                        cancelled_ids.push(id);
                        format!("Cancelled task #{}.", id)
                    } else { "No running tasks.".into() }
                } else { "No running tasks.".into() }
            } else {
                let id: u32 = trimmed.strip_prefix("/cancel ").and_then(|s| s.trim().parse().ok()).unwrap_or(0);
                if let Some(task) = tasks.remove(&id) {
                    task.handle.abort();
                    cancelled_ids.push(id);
                    format!("Cancelled task #{}.", id)
                } else { format!("No task with ID {}.", id) }
            };
            drop(tasks);
            // Broadcast TaskCompleted for each cancelled task so the web UI removes them.
            for id in cancelled_ids {
                let _ = registry.global_tx.send(crate::registry::WsEvent::TaskCompleted {
                    bot: bot_name.into(), task_id: id, duration_secs: 0,
                });
            }
            Some(msg)
        },
        "/reprocess-graphs" => {
            // Reprocess all graphs: link orphans, consolidate.
            let memory_dir = handle.working_dir.join("memory");
            drop(bots);
            let graphs_dir = memory_dir.join("graphs");
            let _ = std::fs::create_dir_all(&graphs_dir);

            // Move stray graph files from memory/ into memory/graphs/.
            // Clean up .corrupted and .tmp files.
            let skip = ["knowledge.json", "knowledge-archive.json",
                         "episodes.json", "embeddings.json",
                         "reputation.json", "questions.json",
                         "rumination_log.json"];
            let mut moved_count = 0u32;
            let mut cleaned_count = 0u32;
            if let Ok(entries) = std::fs::read_dir(&memory_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if !path.is_file() { continue; }
                    let filename = path.file_name()
                        .map(|f| f.to_string_lossy().to_string())
                        .unwrap_or_default();

                    // Clean up corrupted and tmp files.
                    if filename.ends_with(".corrupted") || filename.ends_with(".json.tmp") {
                        let _ = std::fs::remove_file(&path);
                        cleaned_count += 1;
                        continue;
                    }

                    // Skip non-JSON, known root files, and user memory files.
                    if !filename.ends_with(".json") { continue; }
                    if skip.iter().any(|&s| filename == s) { continue; }
                    if filename.starts_with('-') || filename.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) {
                        continue; // user memory like "123456.json" or "-1.json"
                    }

                    // Check if it's a knowledge graph.
                    let is_graph = std::fs::read_to_string(&path)
                        .map(|c| c.contains("\"nodes\"") && c.contains("\"edges\""))
                        .unwrap_or(false);
                    if !is_graph { continue; }

                    let dest = graphs_dir.join(&filename);
                    if !dest.exists() {
                        if let Err(e) = std::fs::rename(&path, &dest) {
                            log::warn!("Failed to move {} to graphs/: {}", path.display(), e);
                        } else {
                            log::info!("Moved {} → graphs/", filename);
                            moved_count += 1;
                        }
                    }
                }
            }
            // Also clean up inside graphs/.
            if let Ok(entries) = std::fs::read_dir(&graphs_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    let filename = path.file_name()
                        .map(|f| f.to_string_lossy().to_string())
                        .unwrap_or_default();
                    if filename.ends_with(".corrupted") || filename.ends_with(".json.tmp") {
                        let _ = std::fs::remove_file(&path);
                        cleaned_count += 1;
                    }
                }
            }

            // Use the store to consolidate all graphs.
            use crate::store::KnowledgeStore;
            let store = crate::store::live::LiveKnowledgeStore::new(memory_dir.clone());
            let mut processed = 0;

            if let Ok(graphs) = store.list_graphs() {
                for g in &graphs {
                    if let Ok(report) = store.consolidate(&g.name) {
                        let _ = store.backfill_thurisaz(&g.name);
                        let _ = store.link_orphans(&g.name);
                        if report.nodes_merged > 0 || report.edges_merged > 0 {
                            log::info!("Reprocessed '{}': {} merged, {} edges merged",
                                g.name, report.nodes_merged, report.edges_merged);
                        }
                        processed += 1;
                    }
                }
            }

            // Remove legacy JSON graph files where CBOR exists with equal or more data.
            let mut json_removed = 0u32;
            if let Ok(entries) = std::fs::read_dir(&graphs_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if !path.is_file() { continue; }
                    let filename = path.file_name()
                        .map(|f| f.to_string_lossy().to_string())
                        .unwrap_or_default();
                    if !filename.ends_with(".json") { continue; }

                    let cbor_path = path.with_extension("cbor");
                    if !cbor_path.exists() { continue; }

                    // Compare edge and citation counts: only remove JSON if CBOR has >= edges
                    // and no citation data would be lost.
                    let json_contents = std::fs::read_to_string(&path).ok();
                    let json_data = json_contents.as_ref()
                        .and_then(|c| serde_json::from_str::<crate::knowledge::GraphData>(c).ok());
                    let json_edges = json_data.as_ref().map(|d| d.edges.len()).unwrap_or(0);
                    let json_cited = json_data.as_ref()
                        .map(|d| d.edges.iter().filter(|(_, _, e)| !e.citations.is_empty()).count())
                        .unwrap_or(0);

                    let cbor_data = std::fs::read(&cbor_path).ok()
                        .and_then(|b| ciborium::de::from_reader::<crate::knowledge::GraphData, _>(&b[..]).ok());
                    let cbor_edges = cbor_data.as_ref().map(|d| d.edges.len()).unwrap_or(0);
                    let cbor_cited = cbor_data.as_ref()
                        .map(|d| d.edges.iter().filter(|(_, _, e)| !e.citations.is_empty()).count())
                        .unwrap_or(0);

                    if cbor_edges >= json_edges && cbor_edges > 0 && cbor_cited >= json_cited {
                        let _ = std::fs::remove_file(&path);
                        json_removed += 1;
                        log::info!("Removed legacy JSON: {} (CBOR has {} edges, JSON had {})",
                            filename, cbor_edges, json_edges);
                    }
                }
            }
            // Also check meta-graph JSON in memory root.
            let meta_json = memory_dir.join("knowledge.json");
            let meta_cbor = memory_dir.join("knowledge.cbor");
            if meta_json.exists() && meta_cbor.exists() {
                let mj_data = std::fs::read_to_string(&meta_json).ok()
                    .and_then(|c| serde_json::from_str::<crate::knowledge::GraphData>(&c).ok());
                let mj_edges = mj_data.as_ref().map(|d| d.edges.len()).unwrap_or(0);
                let mj_cited = mj_data.as_ref()
                    .map(|d| d.edges.iter().filter(|(_, _, e)| !e.citations.is_empty()).count())
                    .unwrap_or(0);
                let mc_data = std::fs::read(&meta_cbor).ok()
                    .and_then(|b| ciborium::de::from_reader::<crate::knowledge::GraphData, _>(&b[..]).ok());
                let mc_edges = mc_data.as_ref().map(|d| d.edges.len()).unwrap_or(0);
                let mc_cited = mc_data.as_ref()
                    .map(|d| d.edges.iter().filter(|(_, _, e)| !e.citations.is_empty()).count())
                    .unwrap_or(0);
                if mc_edges >= mj_edges && mc_edges > 0 && mc_cited >= mj_cited {
                    let _ = std::fs::remove_file(&meta_json);
                    json_removed += 1;
                    log::info!("Removed legacy meta-graph JSON (CBOR has {} edges)", mc_edges);
                }
            }

            let mut summary = format!("Reprocessed {} graph(s): backfilled, consolidated, orphans linked.", processed);
            if moved_count > 0 {
                summary.push_str(&format!("\nMoved {} stray graph file(s) into graphs/.", moved_count));
            }
            if cleaned_count > 0 {
                summary.push_str(&format!("\nCleaned up {} corrupted/temp file(s).", cleaned_count));
            }
            if json_removed > 0 {
                summary.push_str(&format!("\nRemoved {} legacy JSON graph file(s) (CBOR is source of truth).", json_removed));
            }
            Some(summary)
        },
        "/new" => {
            drop(bots);
            // Send a new-session request to the worker.
            registry.send_message(bot_name, chat_id,
                "Summarise our conversation so far in a few bullet points, then say 'Ready for a new conversation.'".into()).await;
            // The worker handles the new_session flag via the /new classification... but web bypasses that.
            // For now, just acknowledge.
            return true; // Let the message go through as a regular dispatch (the worker handles session reset)
        },
        "/usage" => {
            let stats = handle.stats.lock().ok();
            match stats {
                Some(map) if map.is_empty() => Some("No usage yet.".into()),
                Some(map) => {
                    let mut out = String::from("**Session statistics:**\n\n");
                    for (&cid, s) in map.iter() {
                        let uptime = s.started
                            .map(|t| {
                                let secs = t.elapsed().as_secs();
                                if secs < 60 { format!("{}s", secs) }
                                else if secs < 3600 { format!("{}m {}s", secs / 60, secs % 60) }
                                else { format!("{}h {}m", secs / 3600, (secs % 3600) / 60) }
                            })
                            .unwrap_or_else(|| "—".into());
                        if map.len() > 1 { out.push_str(&format!("*User {}:*\n", cid)); }
                        out.push_str(&format!("  Messages: {}\n  Input: {} chars\n  Output: {} chars\n  Session: {}\n",
                            s.messages, s.input_chars, s.output_chars, uptime));
                    }
                    Some(out)
                }
                None => Some("Stats unavailable.".into()),
            }
        },
        "/doctor" => {
            drop(bots);
            let checks = crate::run_doctor_checks();
            let mut out = String::from("**Anthill Doctor**\n\n");
            let mut issues = 0;
            for check in &checks {
                let icon = match check.status.as_str() {
                    "ok" => "✓",
                    "missing" if check.severity == "required" => { issues += 1; "✗" }
                    "missing" => "⚠",
                    _ => "○",
                };
                out.push_str(&format!("{} **{}** — {}\n", icon, check.name, check.detail));
                if check.status != "ok" && !check.help.is_empty() {
                    out.push_str(&format!("  → {}\n", check.help));
                }
            }
            out.push('\n');
            if issues > 0 {
                out.push_str(&format!("**{} required item(s) missing.**", issues));
            } else {
                out.push_str("**All required items present.**");
            }
            Some(out)
        },
        "/questions" => {
            let memory_dir = handle.working_dir.join("memory");
            drop(bots);
            let questions_file = memory_dir.join("questions.json");
            let queue = crate::ai_worker::QuestionsQueue::load(&questions_file);
            if queue.questions.is_empty() {
                Some("No pending questions from rumination.".into())
            } else {
                let mut text = format!("**{} pending question(s) from rumination:**\n\n", queue.questions.len());
                for (i, q) in queue.questions.iter().enumerate() {
                    text.push_str(&format!("{}. **{}**: {}\n", i + 1, q.topic, q.question));
                    if !q.context.is_empty() {
                        text.push_str(&format!("   _(context: {})_\n", q.context));
                    }
                }
                text.push_str("\nAnswer these naturally in conversation. They'll be cleared when you next send a message.");
                Some(text)
            }
        },
        s if s.starts_with("/ask ") => {
            let rest = s.strip_prefix("/ask ").unwrap_or("").trim();
            let parts: Vec<&str> = rest.splitn(2, ' ').collect();
            if parts.len() < 2 || parts[0].is_empty() || parts[1].is_empty() {
                Some("Usage: /ask <ant-name> <question>\nExample: /ask Gaea what do you know about circular economy?\n\nUse /ants to see available ANTs.".into())
            } else {
                let target_ant = parts[0].to_string();
                let question = parts[1].to_string();
                drop(bots);

                // Send the question to the target ANT's AI worker.
                // The target ANT will reason about it using its own expertise.
                // When the response is ready, it'll be forwarded back to this chat.
                let sent = registry.ask_ant(bot_name, &target_ant, chat_id, question.clone()).await;

                if sent {
                    Some(format!("Asking **{}** about: _{}_\n\nTheir response will appear here when ready.", target_ant, question))
                } else {
                    Some(format!("ANT '{}' not found or not running. Use /ants to see available ANTs.", target_ant))
                }
            }
        },
        "/export" => {
            drop(bots);
            Some("Click the **Export** button in the Graph tab to download a shareable HTML snapshot of this ANT's knowledge graph. The file opens in any browser — no server needed.".into())
        },
        "/ruminate" => {
            drop(bots);
            // Send multiple focused rumination tasks.
            let bots = registry.bots.read().await;
            if let Some(handle) = bots.get(bot_name) {
                let tasks: &[(&str, &str)] = &[
                    ("RUMINATION — REFUTATION",
                     "Read your topic graphs in memory/graphs/. Pick ONE important belief with \
                      moderate confidence (40-80%) and ATTEMPT TO REFUTE it.\n\n\
                      1. State the belief clearly\n\
                      2. Formulate specific ways it could be wrong\n\
                      3. Search for evidence that would disprove it\n\
                      4. If you found evidence that COULD disprove but DIDN'T → 'refutation_survived'\n\
                      5. If you found evidence that DOES disprove → 'refutation_failed'\n\
                      6. If you found NOTHING relevant → 'inconsequential_search' (NO change)\n\
                      7. Update the topic graph file"),
                    ("RUMINATION — UNDETERMINED CONNECTIONS",
                     "Read your topic graphs in memory/graphs/. Find edges with relation '?' — \
                      these are connections where the relationship hasn't been determined yet.\n\n\
                      1. Pick one '?' connection\n\
                      2. Look at what other edges connect to those nodes\n\
                      3. Determine what the relationship should be\n\
                      4. Replace the '?' edge with the actual relation, set basis to 'inferred'\n\
                      5. If you can't determine it, add a question to memory/questions.json\n\
                      6. Update the topic graph file"),
                    ("RUMINATION — STRENGTHEN AND IMPROVE",
                     "Read your topic graphs in memory/graphs/. Look for areas to improve:\n\n\
                      1. Find nodes with few connections — are relationships missing?\n\
                      2. Set beneficial_impact on edges where relevant (positive for ideas \
                         that benefit people and planet)\n\
                      3. Look for edges that should exist based on what you know\n\
                      4. Add any new conjectures with appropriate basis and confidence\n\
                      5. Update the topic graph files"),
                    ("RUMINATION — CITATION ANALYSIS",
                     "Verify, analyse and strengthen the citation network.\n\n\
                      Read the citations graph and ALL topic graphs.\n\n\
                      1. VERIFY: For each citation with a URL, FETCH it (check files/ first).\n\
                         If the page returns 404 or doesn't exist, REMOVE the citation from\n\
                         the citations graph AND from any edges that reference it. Broken URLs\n\
                         are likely fabricated — do not keep them.\n\
                      2. For verified citations, extract the TOP 3 CORE IDEAS. Store as summary.\n\
                      3. Follow upstream references to find more authoritative sources.\n\
                      4. Add 'corroborates'/'contradicts'/'cites' edges between related citations.\n\
                      5. Identify CORE CITATIONS — tag as 'core_source'.\n\
                      6. Link citations to topic graph edges using graph_add_citation.\n\
                      7. For edges with only ai_inference, search for real sources.\n\
                      8. WRITE ALL updated graph files."),
                ];

                for (title, body) in tasks {
                    let task_id = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .subsec_nanos();
                    let prompt = format!(
                        "{}\n\n{}\n\n\
                         IMPORTANT: Complete this specific task, update the graph files, \
                         output a brief summary of what you changed, and STOP. \
                         Do not ask follow-up questions.",
                        title, body
                    );
                    let _ = handle.request_tx.send(crate::ai_worker::CliRequest {
                        chat_id,
                        message: prompt,
                        new_session: true,
                        task_id,
                        source: "rumination".into(),
                    });
                }
            }
            Some("🧠 Starting 4 focused rumination tasks — refutation, connections, improvement, citations...".into())
        },
        "/citations" => {
            drop(bots);
            let bots = registry.bots.read().await;
            if let Some(handle) = bots.get(bot_name) {
                let task_id = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .subsec_nanos();
                let prompt = "CITATION ANALYSIS AND CONSOLIDATION\n\n\
                     Read the citations graph (memory/graphs/citations.cbor or .json). \
                     If it doesn't exist, create it. Also read ALL topic graphs in memory/graphs/.\n\n\
                     STEP 1 — Verify and analyse each citation source:\n\
                     For each citation node in the citations graph:\n\
                     1. If it has a URL: FETCH IT. If the page returns 404 or doesn't exist,\n\
                        the citation is BROKEN — remove it from the citations graph AND from\n\
                        any topic graph edges that reference it. Do NOT keep broken URLs.\n\
                     2. If the fetch succeeds, save the content to files/ for future reference\n\
                     3. Read the actual content and extract the TOP 3 CORE IDEAS\n\
                     4. Store these as the node's summary: 'Core ideas: (1) ... (2) ... (3) ...'\n\
                     5. Check what REFERENCES the source itself cites — follow upstream to find\n\
                        more authoritative sources (peer-reviewed papers, official reports)\n\
                     6. Add verified upstream references as new citation nodes\n\n\
                     CRITICAL: If a URL looks plausible but the page doesn't exist (404, timeout,\n\
                     or no relevant content), it was likely fabricated. REMOVE it immediately.\n\n\
                     STEP 2 — Find citation clusters and core sources:\n\
                     1. Compare core ideas across citations — which sources say similar things?\n\
                     2. Add 'corroborates' edges between citations that support the same ideas\n\
                     3. Add 'contradicts' edges between citations that disagree\n\
                     4. Add 'cites' edges when one source references another\n\
                     5. Identify CORE CITATIONS — sources that many others reference or that\n\
                        originated key ideas. Tag these with 'core_source' in their tags.\n\
                     6. Core sources should have higher quality scores.\n\n\
                     STEP 3 — Upgrade low-quality citations:\n\
                     1. For edges with low-quality citations (blog, website, ai_inference),\n\
                        check if a BETTER citation exists in the same family — one connected\n\
                        by 'corroborates' or 'cites' edges that shares the same core ideas\n\
                     2. Replace with the higher-quality source (keep the original as secondary)\n\
                     3. Prefer: peer_reviewed > official_report > book > news > blog > ai_inference\n\n\
                     STEP 4 — Link citations to topic graph edges:\n\
                     1. Using the core ideas extracted in Step 1, match citations to edges\n\
                     2. A citation supports an edge when its core ideas align with the claim\n\
                     3. Use graph_add_citation to attach citations to edges\n\
                     4. Prefer core citations over derivative ones\n\
                     5. For edges with only ai_inference citations, try to find real sources\n\
                     6. WRITE ALL updated graph files\n\n\
                     IMPORTANT: Complete this task, update ALL graph files, \
                     output a summary of what you found (core sources, clusters, citations added), and STOP. \
                     Do not ask follow-up questions.".to_string();
                let _ = handle.request_tx.send(crate::ai_worker::CliRequest {
                    chat_id,
                    message: prompt,
                    new_session: true,
                    task_id,
                    source: "rumination".into(),
                });
            }
            Some("📚 Starting citation consolidation — resolving unknown links and cross-referencing...".into())
        },
        "/reflect" => {
            // Consolidate all graphs: dedup, link orphans, backfill.
            let memory_dir = handle.working_dir.join("memory");
            drop(bots);

            use crate::store::KnowledgeStore;
            let store = crate::store::live::LiveKnowledgeStore::new(memory_dir);
            let mut processed = 0;
            let mut total_merged = 0;
            let mut total_edges_merged = 0;

            if let Ok(graphs) = store.list_graphs() {
                for g in &graphs {
                    if let Ok(report) = store.consolidate(&g.name) {
                        let _ = store.backfill_thurisaz(&g.name);
                        let _ = store.link_orphans(&g.name);
                        total_merged += report.nodes_merged;
                        total_edges_merged += report.edges_merged;
                        processed += 1;
                    }
                }
            }

            Some(format!(
                "Reflected on {} graph(s): {} nodes merged, {} edges merged, orphans linked.",
                processed, total_merged, total_edges_merged
            ))
        },
        _ => None,
    };

    if let Some(text) = response {
        // Broadcast via the global channel (WebSocket subscribes here).
        let _ = registry.global_tx.send(crate::registry::WsEvent::UserMessage {
            bot: bot_name.into(),
            chat_id,
            text: trimmed.into(),
            source: "web".into(),
        });
        let _ = registry.global_tx.send(crate::registry::WsEvent::Message {
            bot: bot_name.into(),
            chat_id,
            text,
            task_id: 0,
        });
        return true;
    }

    false
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
    /// Queue a follow-up message for a running task.
    #[serde(rename = "followup")]
    FollowUp {
        bot: String,
        task_id: u32,
        message: String,
    },
}

pub use crate::trust::now_secs;
