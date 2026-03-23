//! anthill — AI-powered Telegram bots backed by Claude Code, built on R2.
//!
//! Modes:
//!   anthill --config bot.toml              # single bot
//!   anthill --supervise ~/.config/anthill # multi-bot + web dashboard

mod backup;
mod bot;
mod ai_worker;
mod config;
mod dateutil;
mod epistemic;
mod export;
mod events;
mod history;
mod knowledge;
mod store;
mod thematic;
mod maintenance;
mod mcp;
mod ollama;
mod reputation;
mod specgen;
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

    /// Generate a join QR code — scan with a phone to open the web app and join.
    #[arg(long, default_missing_value = "~/.config/anthill", num_args = 0..=1)]
    qr_join: Option<PathBuf>,

    /// Hostname override for QR join URL (default: auto-detect).
    #[arg(long)]
    hostname: Option<String>,

    /// Run diagnostics — check all prerequisites, config, and services.
    #[arg(long)]
    doctor: bool,

    /// Run as MCP server (knowledge graph tools for Claude Code).
    #[arg(long)]
    mcp_server: bool,

    /// Memory directory for MCP server mode.
    #[arg(long)]
    memory_dir: Option<PathBuf>,

    /// Migrate and fix all knowledge graphs in a directory.
    /// Fixes invalid enum values, converts non-graph files, cleans up corrupted files.
    #[arg(long)]
    migrate_graphs: Option<PathBuf>,

    /// Convert all JSON knowledge graphs to CBOR format.
    /// Reads JSON, writes .cbor files, keeps JSON as backup.
    #[arg(long)]
    migrate_to_cbor: Option<PathBuf>,

    /// Export an ANT's knowledge graphs as a self-contained HTML file.
    /// Opens in any browser — 3D graph, search, click-to-explore. No server needed.
    #[arg(long)]
    export_graph: bool,

    /// ANT name for --export-graph.
    #[arg(long)]
    ant: Option<String>,

    /// Output file for --export-graph (default: <ant>-knowledge.html).
    #[arg(long, short)]
    output: Option<PathBuf>,

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

    // QR join — generate join code + QR with URL.
    if let Some(ref raw_dir) = args.qr_join {
        let config_dir = if raw_dir.starts_with("~") {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
            PathBuf::from(raw_dir.to_string_lossy().replacen("~", &home, 1))
        } else {
            raw_dir.clone()
        };

        // Load supervisor config for the HTTP port.
        let sup_toml = config_dir.join("supervisor.toml");
        let port: u16 = if sup_toml.exists() {
            let contents = std::fs::read_to_string(&sup_toml)?;
            toml::from_str::<toml::Value>(&contents)
                .ok()
                .and_then(|v| v.get("http_port")?.as_integer())
                .map(|p| p as u16)
                .unwrap_or(3000)
        } else {
            3000
        };

        // Detect hostname.
        let hostname = args.hostname.clone().unwrap_or_else(|| {
            std::process::Command::new("hostname")
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|h| h.trim().to_string())
                .unwrap_or_else(|| "localhost".into())
        });

        let mut trust_state = trust::ColonyTrust::load(&config_dir)?;
        let code = trust_state.generate_join_code();
        let url = format!("http://{}:{}/#join={}", hostname, port, code);

        println!();
        println!("  Scan this QR code to join the colony:");
        println!();
        print_qr(&url);
        println!();
        println!("  URL: {}", url);
        println!("  Code: {}  (expires in 5 minutes)", code);
        println!();
        return Ok(());
    }

    // Migrate graphs — fix invalid values, clean up.
    if let Some(ref dir) = args.migrate_graphs {
        store::migration::migrate_all(dir);
        return Ok(());
    }

    // Export graph as self-contained HTML.
    if args.export_graph {
        let ant_name = args.ant.as_deref().unwrap_or_else(|| {
            eprintln!("Error: --ant <name> is required for --export-graph");
            std::process::exit(1);
        });
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        let ants_dir = PathBuf::from(&home).join(".config/anthill/ants");

        // Read ant.toml to find memory dir.
        let ant_dir = ants_dir.join(ant_name);
        let config_path = ant_dir.join("ant.toml");
        let memory_dir = if config_path.exists() {
            if let Ok(contents) = std::fs::read_to_string(&config_path) {
                if let Ok(cfg) = toml::from_str::<config::Config>(&contents) {
                    if let Some(wd) = &cfg.claude.working_dir {
                        PathBuf::from(wd).join(&cfg.claude.memory_dir)
                    } else {
                        ant_dir.join("working").join("memory")
                    }
                } else {
                    ant_dir.join("working").join("memory")
                }
            } else {
                ant_dir.join("working").join("memory")
            }
        } else {
            ant_dir.join("working").join("memory")
        };

        let output = args.output.unwrap_or_else(|| PathBuf::from(format!("{}-knowledge.html", ant_name)));

        if let Err(e) = export::export_ant_graphs(&memory_dir, ant_name, &output, None, true) {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }

        return Ok(());
    }

    // Convert JSON graphs to CBOR format.
    if let Some(ref dir) = args.migrate_to_cbor {
        store::migration::migrate_to_cbor(dir);
        return Ok(());
    }

    // MCP server mode — knowledge graph tools for Claude Code.
    if args.mcp_server {
        let memory_dir = args.memory_dir.unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
            PathBuf::from(format!("{}/.config/anthill/ants/default/working/memory", home))
        });
        mcp::run_mcp_server(memory_dir);
        return Ok(());
    }

    // Doctor — diagnostic check.
    if args.doctor {
        run_doctor().await;
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

/// Run diagnostic checks and report status.
/// Output is structured so it can be reused by the web API.
pub fn run_doctor_checks() -> Vec<DoctorCheck> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let config_dir = format!("{}/.config/anthill", home);

    let mut checks = Vec::new();

    // --- Required ---

    // Rust toolchain (already running if we got here, but check for cargo for rebuilds).
    checks.push(check_command("cargo", &["--version"],
        "Rust toolchain", "required",
        "https://rustup.rs/",
    ));

    // AI Backends.
    checks.push(check_command("claude", &["--version"],
        "Claude Code (AI backend)", "recommended",
        "https://docs.anthropic.com/en/docs/claude-code",
    ));
    checks.push(check_command("codex", &["--version"],
        "OpenAI Codex (AI backend)", "optional",
        "https://github.com/openai/codex",
    ));

    // Ollama.
    let ollama_check = check_command("ollama", &["--version"],
        "Ollama (local AI + embeddings)", "recommended",
        "https://ollama.com/download",
    );
    checks.push(ollama_check.clone());

    // If Ollama is installed, check for models.
    if ollama_check.status == "ok" {
        checks.push(check_ollama_model("nomic-embed-text",
            "Embedding model (semantic search)", "recommended",
            "Run: ollama pull nomic-embed-text",
        ));
        checks.push(check_ollama_model("llama3.2",
            "Chat model (local AI backend)", "optional",
            "Run: ollama pull llama3.2",
        ));
    }

    // Git (for backups).
    checks.push(check_command("git", &["--version"],
        "Git (workspace backups)", "required",
        "https://git-scm.com/downloads",
    ));

    // Document analysis tools.
    checks.push(check_command("pdftotext", &["-v"],
        "pdftotext (PDF analysis)", "optional",
        "Install: sudo apt install poppler-utils (or brew install poppler)",
    ));
    checks.push(check_command("pandoc", &["--version"],
        "pandoc (Word doc analysis)", "optional",
        "https://pandoc.org/installing.html",
    ));

    // Tailscale.
    checks.push(check_command("tailscale", &["version"],
        "Tailscale (secure network access)", "recommended",
        "https://tailscale.com/download",
    ));

    // --- Config ---

    // Config directory.
    let config_exists = std::path::Path::new(&config_dir).exists();
    checks.push(DoctorCheck {
        name: "Config directory".into(),
        status: if config_exists { "ok" } else { "missing" }.into(),
        detail: config_dir.clone(),
        severity: "required".into(),
        help: format!("Run: mkdir -p {}/ants", config_dir),
    });

    // Colony key.
    let key_path = format!("{}/colony.key", config_dir);
    let key_exists = std::path::Path::new(&key_path).exists();
    checks.push(DoctorCheck {
        name: "Colony key".into(),
        status: if key_exists { "ok" } else { "missing" }.into(),
        detail: if key_exists { "colony.key exists".into() } else { "Will be generated on first run".into() },
        severity: "info".into(),
        help: "Auto-generated on first anthill --supervise".into(),
    });

    // ANTs.
    let ants_dir = format!("{}/ants", config_dir);
    let ant_count = if std::path::Path::new(&ants_dir).exists() {
        std::fs::read_dir(&ants_dir)
            .map(|entries| entries.flatten()
                .filter(|e| e.path().join("ant.toml").exists())
                .count())
            .unwrap_or(0)
    } else { 0 };
    checks.push(DoctorCheck {
        name: "ANTS configured".into(),
        status: if ant_count > 0 { "ok" } else { "none" }.into(),
        detail: format!("{} ANT(s) found", ant_count),
        severity: "info".into(),
        help: "Create from the web dashboard or: mkdir -p ~/.config/anthill/ants/my-ant && cp config-example/ants/dev-assistant/ant.toml ~/.config/anthill/ants/my-ant/".into(),
    });

    // Devices.
    let devices_path = format!("{}/devices.toml", config_dir);
    let device_count = if std::path::Path::new(&devices_path).exists() {
        std::fs::read_to_string(&devices_path)
            .map(|c| c.matches("[devices.").count())
            .unwrap_or(0)
    } else { 0 };
    checks.push(DoctorCheck {
        name: "Devices provisioned".into(),
        status: if device_count > 0 { "ok" } else { "none" }.into(),
        detail: format!("{} device(s)", device_count),
        severity: "info".into(),
        help: "Run: anthill --qr-join".into(),
    });

    // --- Service ---

    // systemd service (Linux only).
    #[cfg(target_os = "linux")]
    {
        let service_active = std::process::Command::new("systemctl")
            .args(["is-active", "--quiet", "anthill"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        checks.push(DoctorCheck {
            name: "systemd service".into(),
            status: if service_active { "ok" } else { "inactive" }.into(),
            detail: if service_active { "anthill.service running".into() } else { "Not running".into() },
            severity: "info".into(),
            help: "Run: sudo systemctl enable --now anthill".into(),
        });
    }

    checks
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DoctorCheck {
    pub name: String,
    pub status: String,   // "ok", "missing", "none", "inactive", "error"
    pub detail: String,
    pub severity: String, // "required", "recommended", "optional", "info"
    pub help: String,
}

fn check_command(cmd: &str, args: &[&str], name: &str, severity: &str, help_url: &str) -> DoctorCheck {
    let result = std::process::Command::new(cmd)
        .args(args)
        .output();
    match result {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout)
                .lines().next().unwrap_or("").trim().to_string();
            DoctorCheck {
                name: name.into(),
                status: "ok".into(),
                detail: if version.is_empty() { "installed".into() } else { version },
                severity: severity.into(),
                help: help_url.into(),
            }
        }
        _ => DoctorCheck {
            name: name.into(),
            status: "missing".into(),
            detail: format!("{} not found", cmd),
            severity: severity.into(),
            help: help_url.into(),
        },
    }
}

fn check_ollama_model(model: &str, name: &str, severity: &str, help: &str) -> DoctorCheck {
    let result = std::process::Command::new("ollama")
        .args(["list"])
        .output();
    match result {
        Ok(output) if output.status.success() => {
            let list = String::from_utf8_lossy(&output.stdout);
            if list.contains(model) {
                DoctorCheck {
                    name: name.into(),
                    status: "ok".into(),
                    detail: format!("{} available", model),
                    severity: severity.into(),
                    help: help.into(),
                }
            } else {
                DoctorCheck {
                    name: name.into(),
                    status: "missing".into(),
                    detail: format!("{} not pulled", model),
                    severity: severity.into(),
                    help: help.into(),
                }
            }
        }
        _ => DoctorCheck {
            name: name.into(),
            status: "error".into(),
            detail: "Could not list Ollama models".into(),
            severity: severity.into(),
            help: help.into(),
        },
    }
}

async fn run_doctor() {
    let checks = run_doctor_checks();

    println!();
    println!("  Anthill Doctor");
    println!("  ══════════════");
    println!();

    let mut issues = 0;
    for check in &checks {
        let icon = match check.status.as_str() {
            "ok" => "✓",
            "missing" => "✗",
            "none" => "○",
            "inactive" => "○",
            _ => "?",
        };
        let color_status = match check.status.as_str() {
            "ok" => format!("\x1b[32m{}\x1b[0m", icon),         // green
            "missing" if check.severity == "required" => {
                issues += 1;
                format!("\x1b[31m{}\x1b[0m", icon)              // red
            }
            "missing" => {
                format!("\x1b[33m{}\x1b[0m", icon)              // yellow
            }
            _ => icon.to_string(),
        };

        println!("  {} {} — {}", color_status, check.name, check.detail);
        if check.status != "ok" && !check.help.is_empty() {
            println!("    → {}", check.help);
        }
    }

    println!();
    if issues > 0 {
        println!("  \x1b[31m{} required item(s) missing.\x1b[0m", issues);
    } else {
        println!("  \x1b[32mAll required items present.\x1b[0m");
    }
    println!();
}
