//! Bot runner — starts a single ANT's event loop.

use r2_engine::EventBus;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::{broadcast, mpsc};

use crate::{backup, claude_cli, config, plugins, registry, sentants};

/// Run a single ANT.
pub async fn run_bot(
    cfg: config::Config,
    bot_name: String,
    global_event_tx: Option<broadcast::Sender<registry::WsEvent>>,
    bot_registry: Option<Arc<registry::BotRegistry>>,
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
    let stats: claude_cli::StatsMap = Arc::new(Mutex::new(HashMap::new()));
    let tasks: claude_cli::TaskMap = Arc::new(Mutex::new(HashMap::new()));
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
            event_tx: event_tx.clone(),
            status: Arc::new(tokio::sync::RwLock::new(registry::BotStatusKind::Running)),
        };
        reg.bots.write().await.insert(bot_name.clone(), handle);
    }

    // AI plugin — holds all I/O state.
    let ai_plugin = plugins::claude_cli::ClaudeCliPlugin::new(
        1,
        Arc::clone(&response_queue),
        request_tx,
        tg_sender.clone(),
        Arc::clone(&tasks),
        Arc::clone(&stats),
        message_queue.clone(),
        cfg.claude.sync_channels,
    );
    let ai_plugin_id = bus.register_plugin(Box::new(ai_plugin));

    // Conductor sentant — pure FSM.
    let conductor = sentants::claude_cli::ClaudeCliSentant::new(ai_plugin_id);
    bus.register_sentant(Box::new(conductor));

    // Git backup.
    backup::ensure_git_repo(std::path::Path::new(&working_dir))?;

    let backup_working_dir = working_dir.clone();
    let worker_config = claude_cli::CliWorkerConfig {
        working_dir,
        memory_dir,
        repos_dir,
        system_prompt: cfg.claude.system_prompt.clone(),
        skip_permissions: cfg.claude.skip_permissions,
        sync_channels: cfg.claude.sync_channels,
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
