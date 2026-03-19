//! Configuration — loads from TOML file.

use serde::Deserialize;
use std::path::Path;

/// Top-level ANT config.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    /// Display name (shown in web UI). Defaults to directory name.
    pub name: Option<String>,

    /// Mode field kept for backward compatibility (ignored).
    #[serde(default)]
    #[allow(dead_code)]
    pub mode: String,

    pub telegram: TelegramConfig,
    pub slack: SlackConfig,
    pub claude: ClaudeConfig,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct TelegramConfig {
    /// Bot token. Falls back to TELOXIDE_TOKEN env var.
    pub token: Option<String>,
    /// Allowed chat IDs. Empty = allow all.
    pub allow: Vec<i64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct SlackConfig {
    /// Slack bot token (xoxb-...).
    pub bot_token: Option<String>,
    /// Slack app-level token for Socket Mode (xapp-...).
    pub app_token: Option<String>,
}

/// AI and workspace configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ClaudeConfig {
    /// Working directory. Defaults to ~/.config/anthill/ants/<id>/working.
    pub working_dir: Option<String>,
    /// Per-user memory directory (relative to working_dir).
    pub memory_dir: String,
    /// Cloned repos directory (relative to working_dir, excluded from backup).
    pub repos_dir: String,
    /// System prompt — defines the ANT's personality.
    pub system_prompt: Option<String>,
    /// Allow AI to run commands without permission prompts. Auto-set to true.
    pub skip_permissions: bool,
    /// Sync user messages across channels (web, Telegram, Slack). Default: false.
    pub sync_channels: bool,
    /// Encrypt memory/ and files/ in git backups. Default: false.
    pub encrypt_backups: bool,
    /// Auto-backup interval in hours (0 = disabled).
    pub backup_interval_hours: u32,
    /// Git remote for backup pushes (empty = local only).
    pub backup_remote: String,
}

impl Default for ClaudeConfig {
    fn default() -> Self {
        Self {
            working_dir: None,
            memory_dir: "memory".into(),
            repos_dir: "repos".into(),
            system_prompt: None,
            skip_permissions: true,
            sync_channels: false,
            encrypt_backups: false,
            backup_interval_hours: 0,
            backup_remote: String::new(),
        }
    }
}

impl Config {
    /// Load config from a TOML file.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        if !path.exists() {
            log::info!("No config file at {}, using defaults", path.display());
            return Ok(Self::default());
        }
        let contents = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&contents)?;
        Ok(config)
    }
}
