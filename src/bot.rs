//! Bot runner — starts a single ANT's event loop.

use r2_engine::EventBus;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::{broadcast, mpsc};

use crate::{backup, ai_worker, config, plugins, registry, sentants};

/// Run a single ANT.
pub async fn run_bot(
    cfg: config::Config,
    bot_name: String,
    global_event_tx: Option<broadcast::Sender<registry::WsEvent>>,
    bot_registry: Option<Arc<registry::BotRegistry>>,
    ant_bus: Option<Arc<crate::ant_bus::AntBus>>,
) -> anyhow::Result<()> {
    let rt = tokio::runtime::Handle::current();
    let mut bus = EventBus::new();

    // Shared message queue — data plane between input plugins and AI plugin.
    let message_queue: plugins::telegram_bot::MessageQueue =
        Arc::new(Mutex::new(VecDeque::new()));

    // Telegram plugin (optional).
    let telegram_token = cfg.telegram.token.clone()
        .or_else(|| std::env::var("TELOXIDE_TOKEN").ok());
    let (tg_sender, _tg_id) = if let Some(token) = telegram_token {
        let telegram_plugin = plugins::telegram_bot::TelegramPlugin::new(
            0, &rt, token, cfg.telegram.allow.clone(), message_queue.clone(),
        );
        let sender = telegram_plugin.outgoing_sender();
        let id = bus.register_plugin(Box::new(telegram_plugin));
        log::info!("[{}] Telegram enabled", bot_name);
        (sender, Some(id))
    } else {
        let (tx, _rx) = mpsc::unbounded_channel();
        log::info!("[{}] Telegram disabled — web dashboard only", bot_name);
        (tx, None)
    };

    // Slack plugin (optional).
    if let (Some(bot_token), Some(app_token)) = (&cfg.slack.bot_token, &cfg.slack.app_token) {
        let slack_plugin = plugins::slack::SlackPlugin::new(
            2, &rt, bot_token.clone(), app_token.clone(), message_queue.clone(),
        );
        bus.register_plugin(Box::new(slack_plugin));
        log::info!("[{}] Slack enabled", bot_name);
    }

    // Core: AI plugin + conductor sentant.
    let response_queue = Arc::new(Mutex::new(VecDeque::new()));
    let (request_tx, request_rx) = mpsc::unbounded_channel();
    let stats: ai_worker::StatsMap = Arc::new(Mutex::new(HashMap::new()));
    let tasks: ai_worker::TaskMap = Arc::new(Mutex::new(HashMap::new()));
    let follow_ups: ai_worker::FollowUpQueue = Arc::new(Mutex::new(HashMap::new()));
    let (event_tx, _) = broadcast::channel::<registry::WsEvent>(256);

    // Working directory.
    let working_dir = cfg.claude.working_dir.clone().unwrap_or_else(|| {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        format!("{}/.config/anthill/ants/{}/working", home, bot_name)
    });

    let memory_dir = std::path::Path::new(&working_dir).join(&cfg.claude.memory_dir);
    let repos_dir = std::path::Path::new(&working_dir).join(&cfg.claude.repos_dir);
    let files_dir = std::path::Path::new(&working_dir).join("files");

    std::fs::create_dir_all(&working_dir)?;
    std::fs::create_dir_all(&memory_dir)?;
    std::fs::create_dir_all(&repos_dir)?;
    std::fs::create_dir_all(&files_dir)?;

    // Auto-configure MCP server for all CLI backends in this ANT's working directory.
    // This ensures the graph tools (graph_add_node, graph_add_edge, etc.) are always
    // available when Claude/Gemini/Codex run in this directory.
    let mem_dir_str = memory_dir.to_string_lossy().to_string();
    ensure_mcp_settings(&working_dir, &mem_dir_str);

    log::info!("[{}] working dir: {}", bot_name, working_dir);

    // Register with the bot registry (for web dashboard).
    if let Some(ref reg) = bot_registry {
        let display_name = cfg.name.clone().unwrap_or_else(|| bot_name.clone());
        let handle = registry::BotHandle {
            name: bot_name.clone(),
            display_name,
            working_dir: std::path::PathBuf::from(&working_dir),
            request_tx: request_tx.clone(),
            stats: Arc::clone(&stats),
            tasks: Arc::clone(&tasks),
            follow_ups: Arc::clone(&follow_ups),
            event_tx: event_tx.clone(),
            status: Arc::new(tokio::sync::RwLock::new(registry::BotStatusKind::Running)),
        };
        reg.bots.write().await.insert(bot_name.clone(), handle);
    }

    // AI plugin — holds all I/O state.
    let ai_plugin = plugins::ai_plugin::AiPlugin::new(
        1,
        Arc::clone(&response_queue),
        request_tx.clone(),
        tg_sender.clone(),
        Arc::clone(&tasks),
        Arc::clone(&stats),
        Arc::clone(&follow_ups),
        message_queue.clone(),
        cfg.claude.sync_channels,
    );
    let ai_plugin_id = bus.register_plugin(Box::new(ai_plugin));

    // Conductor sentant — pure FSM.
    let conductor = sentants::conductor::ConductorSentant::new(ai_plugin_id);
    bus.register_sentant(Box::new(conductor));

    // Git backup.
    backup::ensure_git_repo(std::path::Path::new(&working_dir))?;

    let backup_working_dir = working_dir.clone();
    let maintenance_memory_dir = memory_dir.clone();
    // Build the AI backend registry from config.
    let backend_registry = crate::ai_backends::build_registry(&cfg);

    let worker_config = ai_worker::CliWorkerConfig {
        working_dir,
        memory_dir,
        repos_dir,
        system_prompt: cfg.claude.system_prompt.clone(),
        skip_permissions: cfg.claude.skip_permissions,
        sync_channels: cfg.claude.sync_channels,
        backends: cfg.claude.backends.clone(),
        worker_timeout_secs: cfg.claude.worker_timeout_secs,
        allow_base_code_changes: cfg.claude.allow_base_code_changes,
        backend_registry: Some(backend_registry),
        ai_config: if cfg.ai.is_configured() { Some(cfg.ai.clone()) } else { None },
    };

    // Forward events to the global broadcast if in supervisor mode.
    let worker_event_tx = global_event_tx.clone().or_else(|| Some(event_tx.clone()));

    // Clone channels for the maintenance daemon before they're moved to the worker.
    let maintenance_tasks = Arc::clone(&tasks);
    let maintenance_request_tx = request_tx.clone();
    let maintenance_event_tx = global_event_tx.clone().or_else(|| Some(event_tx.clone()));

    let worker_request_tx = request_tx.clone();
    tokio::spawn(ai_worker::ai_worker_loop(
        request_rx,
        response_queue,
        worker_config,
        stats,
        tg_sender,
        tasks,
        follow_ups,
        request_tx,
        worker_event_tx,
        bot_name.clone(),
    ));

    // Register on the AntBus and spawn a listener for inter-ANT events.
    if let Some(ref bus) = ant_bus {
        let display_name = cfg.name.clone().unwrap_or_else(|| bot_name.clone());
        let mut bus_rx = bus.register(&bot_name, &display_name).await;
        let bot_name_clone = bot_name.clone();
        let req_tx = worker_request_tx.clone();

        tokio::spawn(async move {
            while let Some(event) = bus_rx.recv().await {
                match event.event_name.as_str() {
                    "colony.query" => {
                        // Another ANT is asking us a question via @mention.
                        let from = event.data.get("from").and_then(|f| f.as_str()).unwrap_or("unknown");
                        let question = event.data.get("question").and_then(|q| q.as_str()).unwrap_or("");
                        let context = event.data.get("context").and_then(|c| c.as_str()).unwrap_or("");

                        let colony_message = format!(
                            "COLONY QUERY from {}\n\n\
                             Context:\n{}\n\n\
                             Question:\n{}\n\n\
                             Respond with your knowledge and reasoning. Be specific about \
                             confidence levels. Your response will be evaluated critically. \
                             Complete your response and STOP.",
                            from,
                            if context.is_empty() { "(no context)" } else { context },
                            question
                        );

                        let task_id = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .subsec_nanos();

                        // Store the reply address so the worker can reply when done.
                        // For now, send as a CliRequest and handle the reply in the
                        // response path.
                        let source = if let Some(ref reply) = event.reply_to {
                            format!("colony:{}:{}", reply.from_ant, reply.chat_id)
                        } else {
                            format!("colony:{}:0", from)
                        };

                        let _ = req_tx.send(ai_worker::CliRequest {
                            chat_id: -2,
                            message: colony_message,
                            new_session: true,
                            task_id,
                            source,
                        });

                        log::info!("[{}] Received colony.query from @{}: {}",
                            bot_name_clone, from,
                            if question.len() > 80 { &question[..80] } else { question });
                    }
                    _ => {
                        log::debug!("[{}] Unhandled bus event: {}", bot_name_clone, event.event_name);
                    }
                }
            }
            log::info!("[{}] AntBus listener exited", bot_name_clone);
        });
    }

    if cfg.claude.backup_interval_hours > 0 {
        let backup_credential = if cfg.claude.encrypt_backups {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
            let key_path = format!("{}/.config/anthill/colony.key", home);
            std::fs::read_to_string(&key_path).unwrap_or_default().trim().to_string()
        } else {
            String::new()
        };
        tokio::spawn(backup::backup_loop(
            backup_working_dir,
            cfg.claude.backup_interval_hours,
            cfg.claude.backup_remote.clone(),
            backup_credential,
        ));
    }

    tokio::spawn(crate::maintenance::maintenance_loop(crate::maintenance::MaintenanceConfig {
        memory_dir: maintenance_memory_dir,
        consolidation_interval: std::time::Duration::from_secs(900),  // 15 minutes
        cross_link_interval: std::time::Duration::from_secs(21600),    // 6 hours
        ant_name: bot_name.clone(),
        request_tx: Some(maintenance_request_tx),
        tasks: Some(maintenance_tasks),
        rumination: cfg.claude.rumination.clone(),
        event_tx: maintenance_event_tx,
    }));

    bus.init_all();
    log::info!("[{}] running", bot_name);

    // Main event loop — 50ms tick.
    let mut last_tick = Instant::now();
    loop {
        let elapsed = last_tick.elapsed().as_millis() as u32;
        last_tick = Instant::now();

        bus.poll_plugins();
        bus.advance_time(elapsed);
        bus.tick();

        let _ = bus.drain_outbound();
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    }
}

/// Ensure the ANT's working directory has MCP settings for all CLI backends
/// that support MCP (Claude, Gemini, Codex). This makes the graph tools
/// (graph_add_node, graph_add_edge, etc.) always available.
///
/// Configures:
/// - `.claude/settings.json` — Claude Code
/// - `.gemini/settings.json` — Gemini CLI
///
/// Only writes if the file doesn't exist or doesn't already contain the
/// anthill-graph MCP server config.
fn ensure_mcp_settings(working_dir: &str, memory_dir: &str) {
    let anthill_bin = std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "anthill".to_string());

    let mcp_server_config = serde_json::json!({
        "command": anthill_bin,
        "args": ["--mcp-server", "--memory-dir", memory_dir]
    });

    // Configure for each CLI backend that supports MCP.
    let configs = [
        (".claude", "settings.json"),
        (".gemini", "settings.json"),
    ];

    for (dir_name, file_name) in &configs {
        write_mcp_settings_file(
            working_dir,
            dir_name,
            file_name,
            &mcp_server_config,
        );
    }
}

/// Write or merge an MCP server config into a settings.json file.
fn write_mcp_settings_file(
    working_dir: &str,
    dir_name: &str,
    file_name: &str,
    mcp_server_config: &serde_json::Value,
) {
    let config_dir = std::path::Path::new(working_dir).join(dir_name);
    let settings_path = config_dir.join(file_name);

    // Check if settings already exist and contain our MCP config.
    if settings_path.exists() {
        if let Ok(contents) = std::fs::read_to_string(&settings_path) {
            if contents.contains("anthill-graph") {
                return; // Already configured.
            }
            // Settings exist but without our MCP server — merge it in.
            if let Ok(mut settings) = serde_json::from_str::<serde_json::Value>(&contents) {
                if let Some(obj) = settings.as_object_mut() {
                    let mcp_servers = obj.entry("mcpServers")
                        .or_insert_with(|| serde_json::json!({}));
                    if let Some(servers) = mcp_servers.as_object_mut() {
                        servers.insert("anthill-graph".to_string(), mcp_server_config.clone());
                    }
                    if let Ok(json) = serde_json::to_string_pretty(&settings) {
                        let _ = std::fs::write(&settings_path, json);
                        log::info!("Added anthill-graph MCP server to {}", settings_path.display());
                    }
                }
                return;
            }
        }
    }

    // No settings file — create one.
    let _ = std::fs::create_dir_all(&config_dir);
    let settings = serde_json::json!({
        "mcpServers": {
            "anthill-graph": mcp_server_config
        }
    });
    if let Ok(json) = serde_json::to_string_pretty(&settings) {
        let _ = std::fs::write(&settings_path, &json);
        log::info!("Created MCP settings at {}", settings_path.display());
    }
}
