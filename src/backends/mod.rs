//! Backend detection — enumerates known AI backends and checks availability.
//!
//! This module provides `BackendKind` (the canonical list of backends)
//! and `detect_backends()`.  Execution is handled by `ai_backends/`.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    Claude,
    Codex,
    Gemini,
    Ollama,
    OpenCode,
    Grok,
    DeepSeek,
    LMStudio,
}

/// All known backend kinds in canonical order.
pub const ALL_BACKENDS: &[BackendKind] = &[
    BackendKind::Claude,
    BackendKind::Codex,
    BackendKind::Gemini,
    BackendKind::Ollama,
    BackendKind::OpenCode,
    BackendKind::Grok,
    BackendKind::DeepSeek,
    BackendKind::LMStudio,
];

impl BackendKind {
    /// Short name (used in config files and as the CLI binary name).
    pub fn name(&self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Gemini => "gemini",
            Self::Ollama => "ollama",
            Self::OpenCode => "opencode",
            Self::Grok => "grok",
            Self::DeepSeek => "deepseek",
            Self::LMStudio => "lmstudio",
        }
    }

    /// Human-readable display name.
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Claude => "Claude Code",
            Self::Codex => "OpenAI Codex",
            Self::Gemini => "Google Gemini",
            Self::Ollama => "Ollama (local)",
            Self::OpenCode => "OpenCode",
            Self::Grok => "Grok",
            Self::DeepSeek => "DeepSeek",
            Self::LMStudio => "LM Studio",
        }
    }

    /// Parse from a string (case-insensitive).
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "claude" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            "gemini" => Some(Self::Gemini),
            "ollama" => Some(Self::Ollama),
            "opencode" => Some(Self::OpenCode),
            "grok" => Some(Self::Grok),
            "deepseek" => Some(Self::DeepSeek),
            "lmstudio" | "lm-studio" | "lm_studio" => Some(Self::LMStudio),
            s if s.starts_with("ollama:") => Some(Self::Ollama),
            s if s.starts_with("grok:") => Some(Self::Grok),
            s if s.starts_with("deepseek:") => Some(Self::DeepSeek),
            s if s.starts_with("lmstudio:") => Some(Self::LMStudio),
            _ => None,
        }
    }

    /// Check if the backend's CLI binary is installed.
    pub fn is_installed(&self) -> bool {
        let cmd = match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Gemini => "gemini",
            Self::Ollama => "ollama",
            Self::OpenCode => "opencode",
            Self::Grok => "grok",
            Self::DeepSeek => "deepseek",
            Self::LMStudio => "lm-studio",
        };
        std::process::Command::new("which")
            .arg(cmd)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}

/// Detect which backends are installed.  Returns `(name, installed)` for
/// all installed backends.
pub fn detect_backends() -> Vec<(String, bool)> {
    ALL_BACKENDS
        .iter()
        .filter(|k| k.is_installed())
        .map(|k| (k.name().to_string(), true))
        .collect()
}
