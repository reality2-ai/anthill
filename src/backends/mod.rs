//! Unified AI backend abstraction.
//!
//! This module provides backend detection and shared types for AI backends.
//! Current backends: Claude, Codex, Gemini, Ollama, OpenCode.

#![allow(dead_code)]

use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct BackendConfig {
    pub working_dir: String,
    pub memory_dir: PathBuf,
    pub repos_dir: PathBuf,
    pub skip_permissions: bool,
    pub allow_base_code_changes: bool,
    pub continue_session: bool,
}

impl BackendConfig {
    pub fn new(
        working_dir: String,
        memory_dir: PathBuf,
        repos_dir: PathBuf,
        skip_permissions: bool,
        allow_base_code_changes: bool,
        continue_session: bool,
    ) -> Self {
        Self {
            working_dir,
            memory_dir,
            repos_dir,
            skip_permissions,
            allow_base_code_changes,
            continue_session,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProgressEvent {
    pub kind: String,
    pub detail: String,
}

#[derive(Debug, Clone)]
pub struct BackendResponse {
    pub text: String,
    pub backend_name: String,
}

#[derive(Debug)]
pub enum BackendError {
    Retriable { message: String },
    NonRetriable { message: String },
    Empty,
}

impl BackendError {
    pub fn message(&self) -> String {
        match self {
            BackendError::Retriable { message } => message.clone(),
            BackendError::NonRetriable { message } => message.clone(),
            BackendError::Empty => "Empty response".to_string(),
        }
    }

    pub fn is_retriable(&self) -> bool {
        matches!(self, BackendError::Retriable { .. })
    }
}

impl std::fmt::Display for BackendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message())
    }
}

impl std::error::Error for BackendError {}

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

impl BackendKind {
    fn name(&self) -> &'static str {
        match self {
            BackendKind::Claude => "claude",
            BackendKind::Codex => "codex",
            BackendKind::Gemini => "gemini",
            BackendKind::Ollama => "ollama",
            BackendKind::OpenCode => "opencode",
            BackendKind::Grok => "grok",
            BackendKind::DeepSeek => "deepseek",
            BackendKind::LMStudio => "lmstudio",
        }
    }

    fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "claude" => Some(BackendKind::Claude),
            "codex" => Some(BackendKind::Codex),
            "gemini" => Some(BackendKind::Gemini),
            "ollama" => Some(BackendKind::Ollama),
            "opencode" => Some(BackendKind::OpenCode),
            "grok" => Some(BackendKind::Grok),
            "deepseek" => Some(BackendKind::DeepSeek),
            "lmstudio" | "lm-studio" | "lm_studio" => Some(BackendKind::LMStudio),
            s if s.starts_with("ollama:") => Some(BackendKind::Ollama),
            s if s.starts_with("grok:") => Some(BackendKind::Grok),
            s if s.starts_with("deepseek:") => Some(BackendKind::DeepSeek),
            s if s.starts_with("lmstudio:") => Some(BackendKind::LMStudio),
            _ => None,
        }
    }

    pub fn is_installed(&self) -> bool {
        let cmd = match self {
            BackendKind::Claude => "claude",
            BackendKind::Codex => "codex",
            BackendKind::Gemini => "gemini",
            BackendKind::Ollama => "ollama",
            BackendKind::OpenCode => "opencode",
            BackendKind::Grok => "grok",
            BackendKind::DeepSeek => "deepseek",
            BackendKind::LMStudio => "lm-studio",
        };
        std::process::Command::new("which")
            .arg(cmd)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}

pub fn detect_backends() -> Vec<(String, bool)> {
    let backends = [
        ("claude", BackendKind::Claude.is_installed()),
        ("codex", BackendKind::Codex.is_installed()),
        ("gemini", BackendKind::Gemini.is_installed()),
        ("ollama", BackendKind::Ollama.is_installed()),
        ("opencode", BackendKind::OpenCode.is_installed()),
        ("grok", BackendKind::Grok.is_installed()),
        ("deepseek", BackendKind::DeepSeek.is_installed()),
        ("lmstudio", BackendKind::LMStudio.is_installed()),
    ];

    backends
        .iter()
        .filter(|(_, installed)| *installed)
        .map(|(name, _)| (name.to_string(), true))
        .collect()
}
