//! Configuration — loads from TOML file.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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

    /// AI engine configuration — pluggable backend selection.
    ///
    /// When present, this takes precedence over `claude.backend_strategy`
    /// for backend selection.  The `[claude]` section remains authoritative
    /// for workspace paths, system prompt, and other non-AI settings.
    #[serde(default)]
    pub ai: AiConfig,
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

impl std::fmt::Display for BackendStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BackendStrategy::CostOptimized => write!(f, "cost_optimized"),
            BackendStrategy::CapabilityOptimized => write!(f, "capability_optimized"),
            BackendStrategy::SpeedOptimized => write!(f, "speed_optimized"),
            BackendStrategy::ReliabilityOptimized => write!(f, "reliability_optimized"),
            BackendStrategy::Balanced => write!(f, "balanced"),
            BackendStrategy::Manual(backends) => write!(f, "{}", backends.join(",")),
        }
    }
}

// NOTE: BackendStrategy enum is kept for backward-compat deserialization of
// existing ant.toml files. All runtime backend selection now goes through
// the BackendRegistry + AiConfig system. The web UI maps legacy strategies
// to EngineCategory at load time.

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

// ---------------------------------------------------------------------------
// AI engine configuration (new pluggable system)
// ---------------------------------------------------------------------------

/// Per-ANT AI engine configuration.
///
/// Controls how this ANT selects AI backends.  Fully optional — if absent,
/// falls back to `[claude].backend_strategy` for backward compatibility.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AiConfig {
    /// Default category for requests without explicit selection.
    /// E.g. "balanced", "intellectual", "fast", "local", "cost_effective".
    pub default_category: String,

    /// Explicit backend ID list with fallback order.
    #[serde(default)]
    pub backends: Vec<String>,

    /// Category → ordered backend ID list.
    #[serde(default)]
    pub categories: HashMap<String, Vec<String>>,

    /// Allow users to override engine selection per-request via `/model`.
    #[serde(default)]
    pub allow_runtime_selection: bool,

    /// Maximum cost per request in USD (0 = unlimited).
    #[serde(default)]
    pub max_cost_per_request_usd: f64,

    /// Maximum daily cost in USD (0 = unlimited).
    #[serde(default)]
    pub max_daily_cost_usd: f64,

    /// Per-backend configuration blocks.
    #[serde(default)]
    pub backends_config: HashMap<String, crate::ai_backends::types::BackendConfig>,
}

impl AiConfig {
    /// Returns true if this section has been explicitly configured.
    pub fn is_configured(&self) -> bool {
        !self.default_category.is_empty()
            || !self.backends.is_empty()
            || !self.categories.is_empty()
            || !self.backends_config.is_empty()
    }

    /// Resolve which backends to try for a given request.
    pub fn resolve_backends(&self, selector: &str) -> Vec<String> {
        let effective = if selector.is_empty() {
            if !self.default_category.is_empty() {
                &self.default_category
            } else if !self.backends.is_empty() {
                return self.backends.clone();
            } else {
                return Vec::new();
            }
        } else {
            selector
        };

        if let Some(ids) = self.categories.get(effective) {
            return ids.clone();
        }

        vec![effective.to_string()]
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
        // backends defaults to empty vec (strategy-based selection is preferred)
        assert!(parsed.claude.backends.is_empty());
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
            ..Default::default()
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
        assert!(cfg.claude.backends.is_empty());
        assert!(cfg.claude.skip_permissions);
        assert!(cfg.telegram.token.is_none());
    }

    #[test]
    fn load_missing_file_returns_defaults() {
        let path = std::path::PathBuf::from("/tmp/anthill-test-nonexistent/ant.toml");
        let cfg = Config::load(&path).unwrap();
        assert!(cfg.name.is_none());
        assert!(cfg.claude.backends.is_empty());
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

    #[test]
    fn ai_config_roundtrips() {
        let toml_str = r#"
name = "AI Test"

[ai]
default_category = "intellectual"
allow_runtime_selection = true
max_cost_per_request_usd = 0.5

[ai.categories]
fast = ["groq-llama3", "openai-gpt4o-mini"]
intellectual = ["claude-cli", "anthropic-claude-opus"]
"#;
        let cfg: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.ai.default_category, "intellectual");
        assert!(cfg.ai.allow_runtime_selection);
        assert!((cfg.ai.max_cost_per_request_usd - 0.5).abs() < 0.001);
        assert_eq!(
            cfg.ai.categories.get("fast").unwrap(),
            &vec!["groq-llama3", "openai-gpt4o-mini"]
        );
        assert!(cfg.ai.is_configured());
    }

    #[test]
    fn ai_config_resolve_backends() {
        let ai = AiConfig {
            default_category: "fast".into(),
            categories: {
                let mut m = HashMap::new();
                m.insert("fast".into(), vec!["groq".into(), "openai".into()]);
                m
            },
            ..Default::default()
        };
        let resolved = ai.resolve_backends("");
        assert_eq!(resolved, vec!["groq", "openai"]);
    }

    #[test]
    fn empty_ai_config_is_not_configured() {
        let cfg: Config = toml::from_str("name = \"Test\"").unwrap();
        assert!(!cfg.ai.is_configured());
    }

    #[test]
    fn web_save_roundtrip_with_category() {
        // Simulate exactly what the web UI PUT handler builds.
        let cfg = Config {
            name: Some("Test".into()),
            ai: AiConfig {
                default_category: "intellectual".into(),
                ..Default::default()
            },
            ..Default::default()
        };

        let toml_str = toml::to_string_pretty(&cfg).unwrap();
        eprintln!("--- Serialized TOML ---\n{}\n---", toml_str);

        // Verify the [ai] section appears.
        assert!(toml_str.contains("[ai]"), "TOML must contain [ai] section");
        assert!(
            toml_str.contains("intellectual"),
            "TOML must contain 'intellectual'"
        );

        // Parse back and verify.
        let parsed: Config = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.ai.default_category, "intellectual");
        assert!(
            parsed.ai.is_configured(),
            "is_configured() must be true after roundtrip"
        );
    }
}
