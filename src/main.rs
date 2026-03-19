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
mod trust;
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

    /// Generate a join code for a new device to join the colony.
    #[arg(long, default_missing_value = "~/.config/anthill", num_args = 0..=1)]
    join_code: Option<PathBuf>,

    /// Export the colony key (for backup to 1Password, etc).
    #[arg(long)]
    export_key: bool,

    /// Import a colony key (restore from backup).
    #[arg(long)]
    import_key: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();
    let args = Args::parse();

    // Export colony key.
    if args.export_key {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        let key_path = format!("{}/.config/anthill/colony.key", home);
        match std::fs::read_to_string(&key_path) {
            Ok(key) => {
                println!();
                println!("  Colony key: {}", key.trim());
                println!();
                println!("  Store this in a password manager (1Password, Bitwarden, etc).");
                println!("  This key encrypts backups and derives all device credentials.");
                println!("  If lost, you cannot decrypt backups or regenerate credentials.");
                println!();
            }
            Err(_) => {
                println!("  No colony key found. Run anthill --supervise first to generate one.");
            }
        }
        return Ok(());
    }

    // Import colony key.
    if let Some(ref key) = args.import_key {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        let config_dir = format!("{}/.config/anthill", home);
        std::fs::create_dir_all(&config_dir)?;
        let key_path = format!("{}/colony.key", config_dir);

        if std::path::Path::new(&key_path).exists() {
            println!();
            println!("  WARNING: colony.key already exists at {}", key_path);
            println!("  Overwriting will invalidate all existing device credentials.");
            println!("  Existing devices will need new join codes.");
            println!();
            print!("  Overwrite? (y/N): ");
            use std::io::Write;
            std::io::stdout().flush()?;
            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;
            if !input.trim().eq_ignore_ascii_case("y") {
                println!("  Cancelled.");
                return Ok(());
            }
        }

        std::fs::write(&key_path, key.trim())?;
        println!();
        println!("  Colony key imported to {}", key_path);
        println!("  Restart anthill to use the new key.");
        println!();
        return Ok(());
    }

    // Generate join code.
    if let Some(ref raw_dir) = args.join_code {
        let config_dir = if raw_dir.starts_with("~") {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
            PathBuf::from(raw_dir.to_string_lossy().replacen("~", &home, 1))
        } else {
            raw_dir.clone()
        };
        let mut trust = trust::ColonyTrust::load(&config_dir)?;
        let code = trust.generate_join_code();
        println!();
        println!("  Join code:  {}", code);
        println!("  Expires in: 5 minutes");
        println!();
        println!("  Enter this in the Anthill web dashboard to join the colony.");
        println!();
        return Ok(());
    }

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
