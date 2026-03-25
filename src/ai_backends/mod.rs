//! Pluggable AI backend system.
//!
//! All AI engines implement the [`AiBackend`] trait.  The [`BackendRegistry`]
//! indexes them by ID and by category so the worker can select backends by
//! user-facing labels ("intellectual", "fast", "local") rather than by
//! hard-coded binary names.
//!
//! ## Adding a new backend
//!
//! 1. Create a new module in `src/ai_backends/` (e.g. `my_engine.rs`).
//! 2. Implement [`AiBackend`] for your struct.
//! 3. Register it in [`create_default_registry`] or via config.
//!
//! That's it — no changes to `ai_worker.rs` or `ai_plugin.rs` needed.

pub mod types;
pub mod registry;
pub mod cli_backend;
pub mod api_backend;
pub mod ollama_backend;
pub mod lmstudio_backend;
pub mod tool_proxy;

pub use types::*;
pub use registry::BackendRegistry;

use std::sync::Arc;

// ---------------------------------------------------------------------------
// Registry factory
// ---------------------------------------------------------------------------

/// Build a [`BackendRegistry`] from an ANT config.
///
/// 1. Auto-detects installed CLI backends.
/// 2. Registers API backends from `[ai.backends_config]`.
/// 3. Maps legacy `[claude].backends` entries to the new system.
/// 4. Applies category overrides from `[ai.categories]`.
pub fn build_registry(config: &crate::config::Config) -> BackendRegistry {
    let mut reg = BackendRegistry::new();

    // ── Phase 1: auto-detect CLI backends ──────────────────────────
    for (id, _name, available) in cli_backend::detect_cli_backends() {
        if available {
            let backend: Arc<dyn AiBackend> = match id {
                "claude-cli" => Arc::new(cli_backend::ClaudeCliBackend::new()),
                "codex-cli" => Arc::new(cli_backend::CodexCliBackend::new()),
                "gemini-cli" => Arc::new(cli_backend::GeminiCliBackend::new()),
                _ => continue,
            };
            reg.register(backend);
        }
    }

    // ── Phase 2: register backends from [ai.backends_config] ──────
    for (id, bc) in &config.ai.backends_config {
        let backend: Option<Box<dyn AiBackend>> = match bc.backend_type.as_str() {
            "ollama" => ollama_backend::create_from_config(id, bc),
            "lmstudio" => lmstudio_backend::create_from_config(id, bc),
            _ => api_backend::create_from_config(id, bc),
        };
        if let Some(b) = backend {
            reg.register(Arc::from(b));
        } else {
            log::warn!("Failed to create backend '{}' (type={})", id, bc.backend_type);
        }
    }

    // ── Phase 3: register local backends if installed but not in config ──
    // Auto-register Ollama with default model if not already configured.
    if reg.get("ollama-llama3-2").is_none() && reg.get("ollama-llama3").is_none() {
        let ollama = ollama_backend::OllamaBackend::new("llama3.2", None);
        reg.register(Arc::new(ollama));
    }

    // Auto-register LM Studio if installed (checks ~/.lmstudio/bin/lms, lms, etc.)
    if !reg.ids().iter().any(|id| id.starts_with("lmstudio")) {
        if crate::backends::BackendKind::LMStudio.is_installed() {
            let lms = lmstudio_backend::LmStudioBackend::new("default", None);
            reg.register(Arc::new(lms));
            log::info!("Auto-registered LM Studio backend (detected on system)");
        }
    }

    // ── Phase 4: apply category overrides from config ──────────────
    if !config.ai.categories.is_empty() {
        reg.apply_category_overrides(&config.ai.categories);
    }

    log::info!("Backend registry: {} backends registered ({})",
        reg.len(),
        reg.ids().join(", "));

    reg
}

/// Map a legacy backend name (from `[claude].backends`) to a registry ID.
///
/// For example: `"claude"` → `"claude-cli"`, `"ollama:llama3"` → `"ollama-llama3"`.
pub fn legacy_name_to_id(name: &str) -> String {
    match name {
        "claude" => "claude-cli".into(),
        "codex" => "codex-cli".into(),
        "gemini" => "gemini-cli".into(),
        other if other.starts_with("ollama") => {
            let model = other.strip_prefix("ollama:")
                .unwrap_or("llama3.2");
            format!("ollama-{}", model.replace(['/', ':', '.'], "-"))
        }
        other => other.to_string(),
    }
}

/// Detect all AI backends available on this system.
///
/// Uses the canonical `BackendKind` list from `backends/mod.rs` so the
/// web UI, doctor check, and runtime all agree.
/// Returns `(id, display_name, available)`.
pub fn detect_all_backends() -> Vec<(String, String, bool)> {
    crate::backends::ALL_BACKENDS.iter()
        .map(|k| (k.name().to_string(), k.display_name().to_string(), k.is_installed()))
        .collect()
}
