//! Configuration — loads from TOML file.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// Top-level ANT config.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct TelegramConfig {
    /// Bot token. Falls back to TELOXIDE_TOKEN env var.
    pub token: Option<String>,
    /// Allowed chat IDs. Empty = allow all.
    pub allow: Vec<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SlackConfig {
    /// Slack bot token (xoxb-...).
    pub bot_token: Option<String>,
    /// Slack app-level token for Socket Mode (xapp-...).
    pub app_token: Option<String>,
}

/// AI and workspace configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ClaudeConfig {
    /// AI backends: ["claude"], ["codex"], ["claude", "codex"], etc.
    /// When multiple are listed, responses are returned from whichever finishes first.
    /// Default: ["claude"]
    pub backends: Vec<String>,
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
            backends: vec!["claude".into()],
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_roundtrips_through_toml() {
        let cfg = Config::default();
        let toml_str = toml::to_string_pretty(&cfg).unwrap();
        let parsed: Config = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.claude.backends, vec!["claude".to_string()]);
        assert_eq!(parsed.claude.memory_dir, "memory");
        assert!(parsed.claude.skip_permissions);
        assert!(!parsed.claude.sync_channels);
        assert!(!parsed.claude.encrypt_backups);
    }

    #[test]
    fn config_with_all_fields_roundtrips() {
        let cfg = Config {
            name: Some("Test Ant".into()),
            mode: String::new(),
            telegram: TelegramConfig {
                token: Some("123:ABC".into()),
                allow: vec![100, 200],
            },
            slack: SlackConfig {
                bot_token: Some("xoxb-test".into()),
                app_token: Some("xapp-test".into()),
            },
            claude: ClaudeConfig {
                backends: vec!["claude".into(), "codex".into()],
                working_dir: Some("/tmp/test".into()),
                system_prompt: Some("You are helpful.".into()),
                sync_channels: true,
                encrypt_backups: true,
                backup_interval_hours: 6,
                backup_remote: "origin".into(),
                ..Default::default()
            },
        };

        let toml_str = toml::to_string_pretty(&cfg).unwrap();
        let parsed: Config = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.name, Some("Test Ant".into()));
        assert_eq!(parsed.telegram.token, Some("123:ABC".into()));
        assert_eq!(parsed.telegram.allow, vec![100, 200]);
        assert_eq!(parsed.slack.bot_token, Some("xoxb-test".into()));
        assert_eq!(parsed.claude.backends, vec!["claude", "codex"]);
        assert_eq!(parsed.claude.working_dir, Some("/tmp/test".into()));
        assert!(parsed.claude.sync_channels);
        assert!(parsed.claude.encrypt_backups);
        assert_eq!(parsed.claude.backup_interval_hours, 6);
    }

    #[test]
    fn partial_toml_fills_defaults() {
        let toml_str = r#"
name = "Minimal"
"#;
        let cfg: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.name, Some("Minimal".into()));
        assert_eq!(cfg.claude.backends, vec!["claude".to_string()]);
        assert!(cfg.claude.skip_permissions);
        assert!(cfg.telegram.token.is_none());
    }

    #[test]
    fn load_missing_file_returns_defaults() {
        let path = std::path::PathBuf::from("/tmp/anthill-test-nonexistent/ant.toml");
        let cfg = Config::load(&path).unwrap();
        assert!(cfg.name.is_none());
        assert_eq!(cfg.claude.backends, vec!["claude".to_string()]);
    }

    #[test]
    fn load_real_file_roundtrips() {
        let dir = std::env::temp_dir().join("anthill-test-config-rt");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ant.toml");

        let cfg = Config {
            name: Some("RoundTrip".into()),
            ..Default::default()
        };
        let toml_str = toml::to_string_pretty(&cfg).unwrap();
        std::fs::write(&path, &toml_str).unwrap();

        let loaded = Config::load(&path).unwrap();
        assert_eq!(loaded.name, Some("RoundTrip".into()));

        std::fs::remove_dir_all(&dir).ok();
    }
}
