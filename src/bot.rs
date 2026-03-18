//! Bot runner — starts a single bot's event loop.
//!
//! Extracted from main.rs so it can be called from both standalone
//! mode and supervisor mode (in-process).

use r2_engine::EventBus;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::{broadcast, mpsc};

use crate::{backup, claude_cli, config, plugins, registry, sentants};

/// Handles for communicating with a running bot.
#[allow(dead_code)]
pub struct BotHandles {
    pub request_tx: mpsc::UnboundedSender<claude_cli::CliRequest>,
    pub stats: claude_cli::StatsMap,
    pub tasks: claude_cli::TaskMap,
    pub event_tx: broadcast::Sender<registry::WsEvent>,
}

/// Run a single bot. Returns the handles for external communication.
///
/// This spawns background tasks (claude worker, backup) and runs
/// the R2 event loop. It blocks until the bot shuts down.
pub async fn run_bot(
    cfg: config::Config,
    bot_name: String,
    global_event_tx: Option<broadcast::Sender<registry::WsEvent>>,
    bot_registry: Option<Arc<registry::BotRegistry>>,
) -> anyhow::Result<()> {
    let mode = if cfg.mode.is_empty() { "raw" } else { cfg.mode.as_str() };
    let use_ai_routing = mode == "ai" || mode == "claude";
    let rt = tokio::runtime::Handle::current();

    let mut bus = EventBus::new();

    // Shared message queue — data plane between Telegram and Claude plugins.
    let message_queue: plugins::telegram_bot::MessageQueue =
        Arc::new(Mutex::new(VecDeque::new()));

    // Telegram plugin (optional — only if token is configured).
    let telegram_token = cfg.telegram.token.clone()
        .or_else(|| std::env::var("TELOXIDE_TOKEN").ok());
    let (tg_sender, tg_id) = if let Some(token) = telegram_token {
        let telegram_plugin = plugins::telegram_bot::TelegramPlugin::new(
            0, &rt, token, cfg.telegram.allow.clone(), use_ai_routing, message_queue.clone(),
        );
        let sender = telegram_plugin.outgoing_sender();
        let id = bus.register_plugin(Box::new(telegram_plugin));
        log::info!("[{}] Telegram enabled", bot_name);
        (sender, Some(id))
    } else {
        let (tx, _rx) = mpsc::unbounded_channel();
        log::info!("[{}] Telegram disabled (no token) — web dashboard only", bot_name);
        (tx, None)
    };

    // Slack plugin (optional — only if both tokens are configured).
    if let (Some(bot_token), Some(app_token)) = (&cfg.slack.bot_token, &cfg.slack.app_token) {
        let slack_plugin = plugins::slack::SlackPlugin::new(
            4, &rt, bot_token.clone(), app_token.clone(), use_ai_routing, message_queue.clone(),
        );
        bus.register_plugin(Box::new(slack_plugin));
        log::info!("[{}] Slack enabled", bot_name);
    }

    match mode {
        "claude" => {
            let response_queue = Arc::new(Mutex::new(VecDeque::new()));
            let (request_tx, request_rx) = mpsc::unbounded_channel();
            let stats: claude_cli::StatsMap = Arc::new(Mutex::new(HashMap::new()));
            let tasks: claude_cli::TaskMap = Arc::new(Mutex::new(HashMap::new()));

            // Create event broadcast channel.
            let (event_tx, _) = broadcast::channel::<registry::WsEvent>(256);

            // Compute working directory.
            let working_dir = cfg
                .claude
                .working_dir
                .clone()
                .unwrap_or_else(|| {
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

            log::info!("[{}] working dir: {}", bot_name, working_dir);

            // Register with the bot registry (for web dashboard access).
            if let Some(ref reg) = bot_registry {
                let display_name = cfg.name.clone().unwrap_or_else(|| bot_name.clone());
                let handle = registry::BotHandle {
                    name: bot_name.clone(),
                    display_name,
                    working_dir: std::path::PathBuf::from(&working_dir),
                    request_tx: request_tx.clone(),
                    stats: Arc::clone(&stats),
                    tasks: Arc::clone(&tasks),
                    event_tx: event_tx.clone(),
                    status: Arc::new(tokio::sync::RwLock::new(registry::BotStatusKind::Running)),
                };
                reg.bots.write().await.insert(bot_name.clone(), handle);
            }

            // Claude CLI plugin — holds all I/O state.
            let cli_plugin = plugins::claude_cli::ClaudeCliPlugin::new(
                1,
                Arc::clone(&response_queue),
                request_tx,
                tg_sender.clone(),
                Arc::clone(&tasks),
                Arc::clone(&stats),
                message_queue.clone(),
                cfg.claude.sync_channels,
            );
            let cli_plugin_id = bus.register_plugin(Box::new(cli_plugin));

            // Claude CLI sentant — pure FSM, no I/O.
            let cli_sentant = sentants::claude_cli::ClaudeCliSentant::new(cli_plugin_id);
            bus.register_sentant(Box::new(cli_sentant));

            if let Some(id) = tg_id {
                let telegram = sentants::telegram::TelegramSentant::new(id);
                bus.register_sentant(Box::new(telegram));
            }

            backup::ensure_git_repo(std::path::Path::new(&working_dir))?;

            let backup_working_dir = working_dir.clone();
            let worker_config = claude_cli::CliWorkerConfig {
                working_dir,
                memory_dir,
                repos_dir,
                system_prompt: cfg.claude.system_prompt.clone(),
                skip_permissions: cfg.claude.skip_permissions,
            };

            // Forward events to the global broadcast if in supervisor mode.
            let worker_event_tx = global_event_tx.clone().or_else(|| Some(event_tx.clone()));

            tokio::spawn(claude_cli::claude_cli_worker(
                request_rx,
                response_queue,
                worker_config,
                stats,
                tg_sender,
                tasks,
                worker_event_tx,
                bot_name.clone(),
            ));

            if cfg.claude.backup_interval_hours > 0 {
                tokio::spawn(backup::backup_loop(
                    backup_working_dir,
                    cfg.claude.backup_interval_hours,
                    cfg.claude.backup_remote.clone(),
                ));
            }
        }

        "ai" => {
            let api_key = cfg.anthropic_api_key().ok_or_else(|| {
                anyhow::anyhow!("AI mode requires anthropic_api_key for bot '{}'", bot_name)
            })?;

            let pty_plugin = plugins::pty::PtyPlugin::new(2, &cfg.raw.shell);
            let pty_id = bus.register_plugin(Box::new(pty_plugin));

            let response_queue = Arc::new(Mutex::new(VecDeque::new()));
            let (request_tx, request_rx) = mpsc::unbounded_channel();

            let ai_plugin = plugins::ai::AiMediationPlugin::new(
                3,
                Arc::clone(&response_queue),
                request_tx,
                tg_sender,
                message_queue.clone(),
            );
            let ai_plugin_id = bus.register_plugin(Box::new(ai_plugin));

            let ai_sentant = sentants::ai::AiSentant::new(ai_plugin_id, pty_id);
            let terminal = sentants::terminal::TerminalSentant::new(pty_id);

            bus.register_sentant(Box::new(ai_sentant));
            bus.register_sentant(Box::new(terminal));
            if let Some(id) = tg_id {
                bus.register_sentant(Box::new(sentants::telegram::TelegramSentant::new(id)));
            }

            let model = cfg.ai.model.clone();
            tokio::spawn(crate::claude_worker::claude_worker(
                request_rx,
                response_queue,
                api_key,
                model,
            ));
        }

        _ => {
            // Raw mode — ChunkerPlugin + pure ChunkerSentant.
            let pty_plugin = plugins::pty::PtyPlugin::new(2, &cfg.raw.shell);
            let pty_id = bus.register_plugin(Box::new(pty_plugin));

            let chunker_plugin = plugins::chunker::ChunkerPlugin::new(3, tg_sender);
            let chunker_plugin_id = bus.register_plugin(Box::new(chunker_plugin));

            let terminal = sentants::terminal::TerminalSentant::new(pty_id);
            let chunker = sentants::chunker::ChunkerSentant::new(chunker_plugin_id);

            bus.register_sentant(Box::new(terminal));
            bus.register_sentant(Box::new(chunker));
            if let Some(id) = tg_id {
                bus.register_sentant(Box::new(sentants::telegram::TelegramSentant::new(id)));
            }
        }
    }

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
