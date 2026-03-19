//! anthill — AI-powered Telegram bots backed by Claude Code, built on R2.
//!
//! Modes:
//!   anthill --config bot.toml              # single bot
//!   anthill --supervise ~/.config/anthill # multi-bot + web dashboard

mod backup;
mod bot;
mod claude_cli;
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
#[command(name = "anthill", about = "Anthill — a colony for ANTS (Autonomous iNTelligenceS)")]
struct Args {
    /// Run as supervisor — manage ANTS + web dashboard.
    #[arg(long)]
    supervise: Option<PathBuf>,

    /// Path to config file (advanced: run a single ANT directly).
    #[arg(long, default_value = "anthill.toml", hide = true)]
    config: PathBuf,

    /// Mode override: raw, ai, or claude.
    #[arg(long, hide = true)]
    mode: Option<String>,

    /// Shell override (raw/ai modes).
    #[arg(long, hide = true)]
    shell: Option<String>,

    /// Allowed Telegram chat IDs (comma-separated).
    #[arg(long, value_delimiter = ',', hide = true)]
    allow: Option<Vec<i64>>,

    /// Shorthand for --mode claude.
    #[arg(long, hide = true)]
    claude: bool,

    /// Shorthand for --mode ai.
    #[arg(long, hide = true)]
    ai: bool,

    /// Claude model override (ai mode).
    #[arg(long, hide = true)]
    ai_model: Option<String>,

    /// Working directory override (claude mode).
    #[arg(long, hide = true)]
    claude_dir: Option<String>,

    /// Generate a join code for a new device to join the colony.
    #[arg(long, default_missing_value = "~/.config/anthill", num_args = 0..=1)]
    join_code: Option<PathBuf>,

    /// Export the colony key (for backup to 1Password, etc).
    /// Use --export-key --qr to display as a QR code.
    #[arg(long)]
    export_key: bool,

    /// Show export as QR code (scan into password manager).
    #[arg(long)]
    qr: bool,

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
                let key = key.trim();
                if args.qr {
                    println!();
                    println!("  Scan this QR code with your password manager:");
                    println!();
                    print_qr(key);
                    println!();
                    println!("  The key will not be shown as text.");
                    println!("  If you need the raw key, use --export-key without --qr.");
                } else {
                    println!();
                    println!("  Colony key: {}", key);
                    println!();
                    println!("  Store this in a password manager (1Password, Bitwarden, etc).");
                    println!("  Or use --export-key --qr to scan as a QR code.");
                }
                println!();
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

    // Supervisor mode (default if no utility command).
    let config_dir = args.supervise.unwrap_or_else(|| {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        PathBuf::from(format!("{}/.config/anthill", home))
    });
    log::info!("Starting — config dir: {}", config_dir.display());
    supervisor::run_supervisor(&config_dir).await
}

/// Print a QR code to the terminal using Unicode block characters.
fn print_qr(data: &str) {
    use qrcode::QrCode;

    let code = match QrCode::new(data.as_bytes()) {
        Ok(c) => c,
        Err(e) => {
            println!("  Failed to generate QR code: {}", e);
            return;
        }
    };

    let width = code.width();
    let modules = code.to_colors();

    // Use Unicode half-blocks: each character represents two vertical pixels.
    // ██ = both black, ▀▀ = top black, ▄▄ = bottom black, "  " = both white.
    // Add quiet zone (2 modules border).
    let qz = 2;
    let total_w = width + qz * 2;
    let total_h = width + qz * 2;

    for y in (0..total_h).step_by(2) {
        print!("  "); // indent
        for x in 0..total_w {
            let top = if y >= qz && y < qz + width && x >= qz && x < qz + width {
                modules[(y - qz) * width + (x - qz)] == qrcode::Color::Dark
            } else {
                false
            };
            let bot = if y + 1 >= qz && y + 1 < qz + width && x >= qz && x < qz + width {
                modules[(y + 1 - qz) * width + (x - qz)] == qrcode::Color::Dark
            } else {
                false
            };
            match (top, bot) {
                (true, true) => print!("█"),
                (true, false) => print!("▀"),
                (false, true) => print!("▄"),
                (false, false) => print!(" "),
            }
        }
        println!();
    }
}
