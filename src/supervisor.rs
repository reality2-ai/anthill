//! Supervisor — discovers, spawns, and monitors multiple anthill bot instances
//! as in-process tokio tasks. Runs the web server alongside.

use serde::Deserialize;
use std::net::SocketAddr;
use std::path::Path;
use tokio::task::JoinHandle;

use std::sync::Arc;

use crate::config::Config;
use crate::registry::{BotRegistry, BotStatusKind};

/// Supervisor configuration (supervisor.toml).
#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct SupervisorConfig {
    /// Subdirectory containing ant configs. Default: "ants"
    pub ants_dir: String,
    /// Restart crashed bots automatically.
    pub restart_on_crash: bool,
    /// Seconds to wait before restarting a crashed bot.
    pub restart_delay_secs: u64,
    /// Maximum consecutive restarts before giving up (0 = unlimited).
    pub max_restarts: u32,
    /// HTTP port for the web dashboard.
    pub http_port: u16,
    /// HTTP bind address.
    pub http_bind: String,
    /// Relay configuration — distributed deployment.
    pub relay: crate::relay::RelayConfig,
}

impl Default for SupervisorConfig {
    fn default() -> Self {
        Self {
            ants_dir: "ants".into(),
            restart_on_crash: true,
            restart_delay_secs: 5,
            max_restarts: 10,
            http_port: 3000,
            http_bind: "0.0.0.0".into(),
            relay: Default::default(),
        }
    }
}

/// Load supervisor config from a directory.
fn load_supervisor_config(dir: &Path) -> anyhow::Result<SupervisorConfig> {
    let config_path = dir.join("supervisor.toml");
    if config_path.exists() {
        let contents = std::fs::read_to_string(&config_path)?;
        Ok(toml::from_str(&contents)?)
    } else {
        log::info!("No supervisor.toml found, using defaults");
        Ok(SupervisorConfig::default())
    }
}

/// Discover ant directories (each must contain an ant.toml).
fn discover_ants(ants_dir: &Path) -> Vec<(String, std::path::PathBuf)> {
    let mut ants = Vec::new();
    let entries = match std::fs::read_dir(ants_dir) {
        Ok(e) => e,
        Err(e) => {
            log::error!("Cannot read ants directory {:?}: {}", ants_dir, e);
            return ants;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let ant_config = path.join("ant.toml");
            if ant_config.exists() {
                let name = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                ants.push((name, ant_config));
            }
        }
    }

    ants.sort_by(|a, b| a.0.cmp(&b.0));
    ants
}

/// Spawn a bot on a dedicated thread with a LocalSet (EventBus is !Send).
fn spawn_bot_task(
    name: String,
    config: Config,
    global_tx: tokio::sync::broadcast::Sender<crate::registry::WsEvent>,
    registry: Arc<BotRegistry>,
) -> JoinHandle<()> {
    let bot_name = name.clone();
    tokio::task::spawn_blocking(move || {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build() {
                Ok(rt) => rt,
                Err(e) => {
                    log::error!("[{}] failed to create runtime: {}", bot_name, e);
                    return;
                }
            };
        let local = tokio::task::LocalSet::new();
        local.block_on(&rt, async move {
            if let Err(e) = crate::bot::run_bot(
                config,
                bot_name.clone(),
                Some(global_tx),
                Some(registry),
            ).await {
                log::error!("[{}] bot exited with error: {}", bot_name, e);
            }
        });
    })
}

/// Run the supervisor — in-process ant tasks + web server.
pub async fn run_supervisor(config_dir: &Path) -> anyhow::Result<()> {
    let sup_cfg = load_supervisor_config(config_dir)?;
    let ants_dir = config_dir.join(&sup_cfg.ants_dir);

    if !ants_dir.exists() {
        std::fs::create_dir_all(&ants_dir)?;
        log::info!("Created ants directory: {}", ants_dir.display());
    }

    let ant_configs = discover_ants(&ants_dir);
    if ant_configs.is_empty() {
        log::warn!(
            "No ants found in {}. Use the web dashboard to create one.",
            ants_dir.display()
        );
    }

    let registry = Arc::new(BotRegistry::new(ants_dir.clone()));

    log::info!(
        "Supervisor starting — {} ant(s) discovered",
        ant_configs.len()
    );

    // Load configs and spawn ants.
    let mut ant_tasks: Vec<(String, JoinHandle<()>, Config)> = Vec::new();

    for (dir_name, config_path) in &ant_configs {
        let cfg = Config::load(config_path)?;
        let display_name = cfg.name.clone().unwrap_or_else(|| dir_name.clone());
        log::info!("Starting ant '{}' ({}) — config: {}", display_name, dir_name, config_path.display());

        // Use dir_name as the stable id for registry, history, and events.
        let handle = spawn_bot_task(
            dir_name.clone(),
            cfg.clone(),
            registry.global_tx.clone(),
            Arc::clone(&registry),
        );

        ant_tasks.push((dir_name.clone(), handle, cfg));
    }

    // Create history store.
    let history_dir = config_dir.join("history");
    let history = crate::history::create_history_store(history_dir);

    // Spawn history recorder — listens to all bot events and persists messages.
    let history_recorder = history.clone();
    let mut history_rx = registry.global_tx.subscribe();
    tokio::spawn(async move {
        loop {
            match history_rx.recv().await {
                Ok(crate::registry::WsEvent::Message { ref bot, ref text, task_id, .. }) => {
                    if let Ok(mut h) = history_recorder.lock() {
                        h.append(bot, crate::history::ChatMessage {
                            role: "bot".into(),
                            text: text.clone(),
                            task_id,
                            timestamp: crate::web::now_secs(),
                        });
                    }
                }
                Ok(crate::registry::WsEvent::UserMessage { ref bot, ref text, .. }) => {
                    if let Ok(mut h) = history_recorder.lock() {
                        h.append(bot, crate::history::ChatMessage {
                            role: "user".into(),
                            text: text.clone(),
                            task_id: 0,
                            timestamp: crate::web::now_secs(),
                        });
                    }
                }
                Ok(_) => {} // ignore non-message events
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    log::warn!("History recorder lagged by {} events", n);
                }
                Err(_) => break,
            }
        }
    });

    // Load colony trust.
    let trust = crate::trust::load_colony_trust(config_dir)?;
    log::info!("Colony trust loaded — {} device(s) provisioned",
        trust.lock().map(|t| t.list_devices().len()).unwrap_or(0));

    // Reload channel — web server signals when new ants should be spawned.
    let (reload_tx, mut reload_rx) = tokio::sync::mpsc::channel::<()>(1);

    // Build global backend registry for the web server.
    let default_config = Config::default();
    let backend_registry = Arc::new(crate::ai_backends::build_registry(&default_config));

    // Start the web server.
    let bind: SocketAddr = format!("{}:{}", sup_cfg.http_bind, sup_cfg.http_port)
        .parse()
        .map_err(|e| anyhow::anyhow!("Invalid bind address '{}:{}': {}", sup_cfg.http_bind, sup_cfg.http_port, e))?;
    let web_registry = Arc::clone(&registry);
    tokio::spawn(crate::web::run_web_server(web_registry, history, trust.clone(), reload_tx, backend_registry, bind));

    log::info!("Web dashboard at http://{}", bind);

    // ── Relay: distributed deployment ──────────────────────────────
    if sup_cfg.relay.is_active() {
        if sup_cfg.relay.engine_listener {
            let relay_port = if sup_cfg.relay.engine_port > 0 {
                sup_cfg.relay.engine_port
            } else {
                3001
            };
            let relay_bind: SocketAddr = format!("{}:{}", sup_cfg.http_bind, relay_port)
                .parse()
                .unwrap_or_else(|_| SocketAddr::from(([0, 0, 0, 0], relay_port)));
            let engine_id = config_dir.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            tokio::spawn(crate::relay::run_engine_listener(
                relay_bind,
                Arc::clone(&registry),
                trust.clone(),
                engine_id,
            ));
            log::info!("Engine relay listener on {}", relay_bind);
        }

        if !sup_cfg.relay.remote_engines.is_empty() {
            let gateway = crate::relay::WebGateway::new(
                Arc::clone(&registry),
                trust.clone(),
            );
            let credential = sup_cfg.relay.credential.clone();
            let device_id = sup_cfg.relay.device_id.clone();
            let engines = sup_cfg.relay.remote_engines.clone();

            tokio::spawn(async move {
                for url in &engines {
                    match gateway.connect(url, &credential, &device_id).await {
                        Ok(()) => log::info!("Connected to remote engine: {}", url),
                        Err(e) => log::error!("Failed to connect to {}: {}", url, e),
                    }
                }
            });
        }
    }

    // Monitor loop.
    let mut restart_counts: std::collections::HashMap<String, u32> =
        std::collections::HashMap::new();

    loop {
        // Check for reload signal (non-blocking).
        if reload_rx.try_recv().is_ok() {
            // Remove finished tasks so stopped ANTs can be re-discovered.
            ant_tasks.retain(|(_, handle, _)| !handle.is_finished());

            let new_configs = discover_ants(&ants_dir);
            let running: Vec<String> = ant_tasks.iter().map(|(n, _, _)| n.clone()).collect();
            for (name, config_path) in &new_configs {
                if !running.contains(name) {
                    log::info!("Hot-adding ant '{}' — config: {}", name, config_path.display());
                    if let Ok(cfg) = Config::load(config_path) {
                        let handle = spawn_bot_task(
                            name.clone(),
                            cfg.clone(),
                            registry.global_tx.clone(),
                            Arc::clone(&registry),
                        );
                        ant_tasks.push((name.clone(), handle, cfg));
                    }
                }
            }
        }

        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        for (name, handle, cfg) in &mut ant_tasks {
            if handle.is_finished() {
                log::warn!("Ant '{}' has stopped", name);

                // Update registry status and notify web clients.
                {
                    let bots = registry.bots.read().await;
                    if let Some(bot_handle) = bots.get(name) {
                        *bot_handle.status.write().await = BotStatusKind::Stopped;
                    }
                }
                let _ = registry.global_tx.send(crate::registry::WsEvent::BotStatus {
                    bot: name.clone(),
                    status: "stopped".into(),
                });

                if !sup_cfg.restart_on_crash {
                    continue;
                }

                let count = restart_counts.entry(name.clone()).or_insert(0);
                *count += 1;

                if sup_cfg.max_restarts > 0 && *count > sup_cfg.max_restarts {
                    log::error!(
                        "Ant '{}' exceeded max restarts ({}), not restarting",
                        name, sup_cfg.max_restarts
                    );
                    continue;
                }

                // Exponential backoff: base × 2^(attempt-1), capped at 5 minutes.
                let delay = (sup_cfg.restart_delay_secs as f64
                    * 2_f64.powi((*count as i32 - 1).min(6)))
                    .min(300.0) as u64;
                log::info!(
                    "Restarting ant '{}' in {}s (attempt {})",
                    name, delay, count
                );
                tokio::time::sleep(std::time::Duration::from_secs(delay)).await;

                // Re-read config from disk so runtime changes (e.g. disabling
                // rumination) take effect on restart.
                let config_path = ants_dir.join(name.as_str()).join("ant.toml");
                let fresh_cfg = Config::load(&config_path).unwrap_or_else(|_| cfg.clone());
                *cfg = fresh_cfg.clone();

                *handle = spawn_bot_task(
                    name.clone(),
                    fresh_cfg,
                    registry.global_tx.clone(),
                    Arc::clone(&registry),
                );
                log::info!("Ant '{}' restarted (config reloaded from disk)", name);
                let _ = registry.global_tx.send(crate::registry::WsEvent::BotStatus {
                    bot: name.clone(),
                    status: "running".into(),
                });
            }
        }
    }
}
