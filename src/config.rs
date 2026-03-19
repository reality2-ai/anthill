//! Configuration — loads from TOML file, with CLI and env var overrides.

use serde::Deserialize;
use std::path::Path;

/// Top-level config.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    /// Display name for this ant (shown in web UI). Defaults to directory name.
    pub name: Option<String>,
    /// Mode: "raw", "ai", or "claude".
    pub mode: String,

    pub telegram: TelegramConfig,
    pub slack: SlackConfig,
    pub raw: RawConfig,
    pub ai: AiConfig,
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

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct RawConfig {
    /// Shell to spawn.
    pub shell: String,
}

impl Default for RawConfig {
    fn default() -> Self {
        Self {
            shell: "/bin/bash".into(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct AiConfig {
    /// Claude model for AI mediation.
    pub model: String,
    /// Anthropic API key. Falls back to ANTHROPIC_API_KEY env var.
    pub anthropic_api_key: Option<String>,
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            model: "claude-sonnet-4-20250514".into(),
            anthropic_api_key: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ClaudeConfig {
    /// Working directory for `claude -p`. Defaults to current directory.
    pub working_dir: Option<String>,
    /// Directory for per-user memory files ({chat_id}.md).
    /// Relative to working_dir. Default: "memory"
    pub memory_dir: String,
    /// Directory for cloned git repositories.
    /// Relative to working_dir. Default: "repos"
    pub repos_dir: String,
    /// System prompt prepended to every invocation.
    pub system_prompt: Option<String>,
    /// Skip permission prompts (allows Claude to run commands without approval).
    /// Only enable if access is restricted via telegram.allow.
    pub skip_permissions: bool,
    /// Sync user messages across channels (telegram, slack, web).
    /// When true, a message sent via Telegram also appears in Slack and the web dashboard.
    /// Default: false (security — don't leak messages across channels).
    pub sync_channels: bool,
    /// Encrypt memory/ and files/ in git backups using the colony trust key.
    /// Safe for public repos. Default: false.
    pub encrypt_backups: bool,
    /// Auto-backup: commit working dir changes to git periodically.
    /// Set to 0 to disable. Default: 0 (disabled).
    pub backup_interval_hours: u32,
    /// Git remote to push backups to (e.g. "origin"). Empty = local only.
    pub backup_remote: String,
}

impl Default for ClaudeConfig {
    fn default() -> Self {
        Self {
            working_dir: None,
            memory_dir: "memory".into(),
            repos_dir: "repos".into(),
            system_prompt: None,
            skip_permissions: false,
            sync_channels: false,
            encrypt_backups: false,
            backup_interval_hours: 0,
            backup_remote: String::new(),
        }
    }
}

impl Config {
    /// Load config from a TOML file. Returns default config if file doesn't exist.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        if !path.exists() {
            log::info!("No config file at {}, using defaults", path.display());
            return Ok(Self::default());
        }

        let contents = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&contents)?;
        Ok(config)
    }

    /// Resolve the Anthropic API key (config file → env var).
    pub fn anthropic_api_key(&self) -> Option<String> {
        self.ai
            .anthropic_api_key
            .clone()
            .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
    }
}
