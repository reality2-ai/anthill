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

/// Rumination engine configuration — autonomous thinking during idle time.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RuminationConfig {
    /// Enable the rumination engine. Default: false.
    pub enabled: bool,
    /// Minimum interval between rumination cycles (seconds). Default: 7200 (2 hours).
    pub interval_secs: u64,
    /// Topic graphs to focus on. Empty = all topics.
    pub topics: Vec<String>,
    /// Enable active refutation — challenge existing beliefs. Default: true.
    pub refutation_enabled: bool,
    /// Enable idea synthesis — conjecture transitive relationships. Default: true.
    pub synthesis_enabled: bool,
    /// Enable contradiction resolution — pit conflicting beliefs against each other. Default: true.
    pub contradiction_resolution: bool,
    /// Enable autonomous initiative — open-ended self-improvement. Default: false.
    pub initiative_enabled: bool,
    /// Minimum idle time before ruminating (seconds). Default: 300.
    pub min_idle_secs: u64,
}

impl Default for RuminationConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_secs: 7200,
            topics: Vec::new(),
            refutation_enabled: true,
            synthesis_enabled: true,
            contradiction_resolution: true,
            initiative_enabled: false,
            min_idle_secs: 300,
        }
    }
}

/// Backend selection strategy.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BackendStrategy {
    /// Prefer cheapest backend (Ollama > DeepSeek > Gemini > Claude)
    #[default]
    CostOptimized,
    /// Prefer most capable backend (Claude > Grok > Gemini > Codex > DeepSeek)
    CapabilityOptimized,
    /// Prefer fastest response (Ollama > LM Studio > cloud)
    SpeedOptimized,
    /// Prefer most reliable (based on recent success rate)
    ReliabilityOptimized,
    /// Balanced mix of cost and capability
    Balanced,
    /// Manual list of backends in priority order
    Manual(Vec<String>),
}

impl BackendStrategy {
    /// Get ordered list of backends based on strategy.
    pub fn get_backends(&self, available: &[(String, bool)]) -> Vec<String> {
        let available: Vec<&str> = available
            .iter()
            .filter(|(_, installed)| *installed)
            .map(|(name, _)| name.as_str())
            .collect();

        match self {
            BackendStrategy::CostOptimized => {
                let mut backends: Vec<_> = available
                    .iter()
                    .filter(|&&b| {
                        matches!(b, "ollama" | "lmstudio" | "deepseek" | "gemini" | "claude")
                    })
                    .copied()
                    .collect();
                backends.sort_by_key(|b| match *b {
                    "ollama" | "lmstudio" => 0,
                    "deepseek" => 1,
                    "gemini" => 2,
                    "claude" => 3,
                    "grok" => 4,
                    "codex" => 5,
                    "opencode" => 6,
                    _ => 99,
                });
                backends.into_iter().map(|s| s.to_string()).collect()
            }
            BackendStrategy::CapabilityOptimized => {
                let mut backends: Vec<_> = available
                    .iter()
                    .filter(|&&b| {
                        matches!(
                            b,
                            "claude" | "grok" | "gemini" | "codex" | "deepseek" | "ollama"
                        )
                    })
                    .copied()
                    .collect();
                backends.sort_by_key(|b| match *b {
                    "claude" => 0,
                    "grok" => 1,
                    "gemini" => 2,
                    "codex" => 3,
                    "deepseek" => 4,
                    "ollama" | "lmstudio" => 5,
                    _ => 99,
                });
                backends.into_iter().map(|s| s.to_string()).collect()
            }
            BackendStrategy::SpeedOptimized => {
                let mut backends: Vec<_> = available
                    .iter()
                    .filter(|&&b| {
                        matches!(
                            b,
                            "ollama" | "lmstudio" | "deepseek" | "gemini" | "claude" | "grok"
                        )
                    })
                    .copied()
                    .collect();
                backends.sort_by_key(|b| match *b {
                    "ollama" | "lmstudio" => 0,
                    "deepseek" => 1,
                    "gemini" => 2,
                    "claude" | "grok" => 3,
                    _ => 99,
                });
                backends.into_iter().map(|s| s.to_string()).collect()
            }
            BackendStrategy::ReliabilityOptimized => {
                available.iter().map(|s| s.to_string()).collect()
            }
            BackendStrategy::Balanced => {
                let mut backends: Vec<_> = available.iter().copied().collect();
                backends.sort_by_key(|b| match *b {
                    "ollama" | "lmstudio" => 0,
                    "deepseek" => 1,
                    "gemini" => 2,
                    "claude" => 3,
                    "grok" => 4,
                    "codex" => 5,
                    "opencode" => 6,
                    _ => 99,
                });
                backends.into_iter().map(|s| s.to_string()).collect()
            }
            BackendStrategy::Manual(backends) => backends.clone(),
        }
    }

    /// Classify a message to determine the task type.
    pub fn classify_message(message: &str) -> TaskType {
        let msg_lower = message.to_lowercase();
        let msg_len = message.len();

        // Code-related patterns
        if msg_lower.contains("write code")
            || msg_lower.contains("implement")
            || msg_lower.contains("function")
            || msg_lower.contains("debug")
            || msg_lower.contains("fix this")
            || msg_lower.contains("refactor")
            || msg_lower.contains(".py")
            || msg_lower.contains(".rs")
            || msg_lower.contains(".js")
            || msg_lower.contains(".ts")
            || msg_lower.contains("git ")
            || msg_lower.contains("test ")
            || msg_lower.contains("import ")
        {
            return TaskType::Coding;
        }

        // Analysis/reasoning patterns
        if msg_lower.contains("analyze")
            || msg_lower.contains("why")
            || msg_lower.contains("how does")
            || msg_lower.contains("explain")
            || msg_lower.contains("compare")
            || msg_lower.contains("what's the difference")
            || msg_lower.contains("evaluate")
            || msg_lower.contains("assess")
        {
            return TaskType::Reasoning;
        }

        // Simple query - short messages
        if msg_len < 100 && !msg_lower.starts_with('/') {
            return TaskType::Simple;
        }

        // Default based on length
        if msg_len > 2000 {
            TaskType::Complex
        } else {
            TaskType::General
        }
    }
}

/// Task type classification for dynamic backend selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskType {
    /// Simple question or command
    Simple,
    /// General conversation
    General,
    /// Complex reasoning or analysis
    Reasoning,
    /// Coding task
    Coding,
    /// Very long or complex task
    Complex,
}

/// Dynamic backend selection based on task type.
impl BackendStrategy {
    /// Get best backend for a specific task type.
    pub fn backend_for_task(&self, task: TaskType) -> &'static str {
        match task {
            TaskType::Simple => {
                // Prefer fastest/cheapest for simple tasks
                "ollama"
            }
            TaskType::General => {
                // Balanced choice
                "ollama"
            }
            TaskType::Reasoning => {
                // Need strong reasoning - prefer Claude or Grok
                "claude"
            }
            TaskType::Coding => {
                // Coding tasks - prefer specialized coders
                "deepseek"
            }
            TaskType::Complex => {
                // Complex tasks need best capability
                "claude"
            }
        }
    }
}

/// AI and workspace configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ClaudeConfig {
    /// Backend selection strategy.
    /// Default: cost_optimized
    pub backend_strategy: BackendStrategy,
    /// Deprecated: Use backend_strategy instead.
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
    /// Worker timeout in seconds — kill if no output for this long. Default: 600 (10 min).
    pub worker_timeout_secs: u64,
    /// Allow the AI to modify files outside the working directory (e.g. Anthill source code).
    /// Default: false — the AI can only modify files within its workspace and repos/.
    #[serde(default)]
    pub allow_base_code_changes: bool,
    /// Rumination engine — autonomous thinking during idle time.
    pub rumination: RuminationConfig,
}

impl Default for ClaudeConfig {
    fn default() -> Self {
        Self {
            backend_strategy: BackendStrategy::default(),
            backends: vec![],
            working_dir: None,
            memory_dir: "memory".into(),
            repos_dir: "repos".into(),
            system_prompt: None,
            skip_permissions: true,
            sync_channels: false,
            encrypt_backups: false,
            backup_interval_hours: 0,
            backup_remote: String::new(),
            worker_timeout_secs: 600,
            allow_base_code_changes: false,
            rumination: RuminationConfig::default(),
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
