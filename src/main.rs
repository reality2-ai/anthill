//! anthill — AI-powered Telegram bots backed by Claude Code, built on R2.
//!
//! Modes:
//!   anthill --config bot.toml              # single bot
//!   anthill --supervise ~/.config/anthill # multi-bot + web dashboard

mod backup;
mod bot;
mod claude_cli;
mod claude_worker;
mod config;
mod events;
mod history;
mod plugins;
mod registry;
mod sentants;
mod supervisor;
mod web;

use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "anthill", about = "Anthill — AI-powered Telegram bots")]
struct Args {
    /// Path to config file (single bot mode).
    #[arg(long, default_value = "anthill.toml")]
    config: PathBuf,

    /// Mode override: raw, ai, or claude.
    #[arg(long)]
    mode: Option<String>,

    /// Shell override (raw/ai modes).
    #[arg(long)]
    shell: Option<String>,

    /// Allowed Telegram chat IDs (comma-separated).
    #[arg(long, value_delimiter = ',')]
    allow: Option<Vec<i64>>,

    /// Shorthand for --mode claude.
    #[arg(long)]
    claude: bool,

    /// Shorthand for --mode ai.
    #[arg(long)]
    ai: bool,

    /// Claude model override (ai mode).
    #[arg(long)]
    ai_model: Option<String>,

    /// Working directory override (claude mode).
    #[arg(long)]
    claude_dir: Option<String>,

    /// Run as supervisor — manage multiple bots + web dashboard.
    #[arg(long)]
    supervise: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();
    let args = Args::parse();

    // Supervisor mode.
    if let Some(ref config_dir) = args.supervise {
        log::info!("Supervisor mode — config dir: {}", config_dir.display());
        return supervisor::run_supervisor(config_dir).await;
    }

    // Single bot mode.
    let mut cfg = config::Config::load(&args.config)?;

    // CLI overrides.
    if let Some(m) = &args.mode {
        cfg.mode = m.clone();
    }
    if args.claude {
        cfg.mode = "claude".into();
    }
    if args.ai {
        cfg.mode = "ai".into();
    }
    if cfg.mode.is_empty() {
        cfg.mode = "raw".into();
    }
    if let Some(s) = &args.shell {
        cfg.raw.shell = s.clone();
    }
    if let Some(a) = &args.allow {
        cfg.telegram.allow = a.clone();
    }
    if let Some(m) = &args.ai_model {
        cfg.ai.model = m.clone();
    }
    if let Some(d) = &args.claude_dir {
        cfg.claude.working_dir = Some(d.clone());
    }

    log::info!("anthill starting — mode={}", cfg.mode);

    bot::run_bot(cfg, "standalone".into(), None, None).await
}
