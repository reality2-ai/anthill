//! Core types for the AI backend abstraction.

use std::collections::HashMap;
use tokio::sync::mpsc;

// ---------------------------------------------------------------------------
// Engine categories
// ---------------------------------------------------------------------------

/// User-facing categories for backend selection.
///
/// Users pick a category ("give me the cheapest model" or "use the best
/// reasoning model") and the registry resolves it to a concrete backend
/// list with fallback order.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineCategory {
    /// Lowest cost per token.
    CostEffective,
    /// Best reasoning / most capable.
    Intellectual,
    /// Fastest response time.
    Fast,
    /// On-premise only — no data leaves the machine.
    Local,
    /// Good mix of speed, cost, and capability.
    Balanced,
    /// Domain-specific (the inner string names the domain, e.g. "coding").
    Specialized(String),
}

impl std::fmt::Display for EngineCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CostEffective => write!(f, "cost_effective"),
            Self::Intellectual => write!(f, "intellectual"),
            Self::Fast => write!(f, "fast"),
            Self::Local => write!(f, "local"),
            Self::Balanced => write!(f, "balanced"),
            Self::Specialized(s) => write!(f, "specialized:{}", s),
        }
    }
}

impl EngineCategory {
    /// Parse a category string (case-insensitive).  Strings like
    /// `"specialized:coding"` are split on the colon.
    pub fn parse(s: &str) -> Option<Self> {
        let lower = s.trim().to_lowercase();
        match lower.as_str() {
            "cost_effective" | "cost-effective" | "cheap" => Some(Self::CostEffective),
            "intellectual" | "smart" | "reasoning" | "best" => Some(Self::Intellectual),
            "fast" | "quick" | "speed" => Some(Self::Fast),
            "local" | "private" | "on-premise" => Some(Self::Local),
            "balanced" | "default" => Some(Self::Balanced),
            other => {
                if let Some(domain) = other.strip_prefix("specialized:") {
                    Some(Self::Specialized(domain.to_string()))
                } else {
                    None
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Engine metadata
// ---------------------------------------------------------------------------

/// Static metadata about a backend's capabilities and cost profile.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EngineTags {
    /// Categories this engine belongs to.
    pub categories: Vec<EngineCategory>,
    /// Free-form capability strings: "code", "vision", "function-calling", …
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// 1 (cheapest) – 5 (most expensive).
    #[serde(default = "default_tier")]
    pub cost_tier: u8,
    /// 1 (fastest) – 5 (slowest).
    #[serde(default = "default_tier")]
    pub speed_tier: u8,
    /// 1 (basic) – 5 (best).
    #[serde(default = "default_tier")]
    pub quality_tier: u8,
}

fn default_tier() -> u8 { 3 }

impl Default for EngineTags {
    fn default() -> Self {
        Self {
            categories: vec![EngineCategory::Balanced],
            capabilities: Vec::new(),
            cost_tier: 3,
            speed_tier: 3,
            quality_tier: 3,
        }
    }
}

// ---------------------------------------------------------------------------
// Request / Response / Progress
// ---------------------------------------------------------------------------

/// A request to an AI backend.
#[derive(Debug, Clone)]
pub struct AiRequest {
    pub task_id: u32,
    pub chat_id: i64,
    /// The user (or system) message to process.
    pub message: String,
    /// System prompt (personality, knowledge graph context, etc.).
    pub system_prompt: String,
    /// Working directory for CLI backends.
    pub working_dir: String,
    /// Whether to skip permission prompts (CLI backends).
    pub skip_permissions: bool,
    /// Continue an existing session (CLI backends).
    pub continue_session: bool,
    /// Extra key-value context (future use).
    pub context: HashMap<String, String>,
}

/// Progress event streamed while a backend is working.
#[derive(Debug, Clone)]
pub struct AiProgress {
    pub task_id: u32,
    /// Type of progress: "tool_use", "thinking", "reading", "writing",
    /// "running", "question", "fallback", "warning", …
    pub kind: String,
    /// Human-readable detail.
    pub detail: String,
}

/// Final response from a backend.
#[derive(Debug, Clone)]
pub struct AiResponse {
    /// The text answer.
    pub text: String,
    /// Which backend produced this response.
    pub backend_id: String,
    /// Token counts if available: (input_tokens, output_tokens).
    pub tokens: Option<(usize, usize)>,
    /// Estimated cost in micro-dollars (1 USD = 1_000_000).
    pub cost_microdollars: Option<u64>,
}

/// Error from a backend execution.
#[derive(Debug, Clone)]
pub struct AiError {
    pub message: String,
    /// Is this error transient / retriable on a different backend?
    pub retriable: bool,
}

impl std::fmt::Display for AiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for AiError {}

impl AiError {
    pub fn permanent(msg: impl Into<String>) -> Self {
        Self { message: msg.into(), retriable: false }
    }
    pub fn retriable(msg: impl Into<String>) -> Self {
        Self { message: msg.into(), retriable: true }
    }

    /// Classify an error message to decide retriability.
    pub fn classify(text: &str) -> Self {
        let lower = text.to_lowercase();
        // Permanent errors — retrying on another backend won't help.
        if lower.contains("context length exceeded") || lower.contains("too many tokens") {
            return Self::permanent(text);
        }
        if lower.contains("invalid request") || lower.contains("invalid api") {
            return Self::permanent(text);
        }
        if lower.contains("authentication") || lower.contains("unauthorized") || lower.contains("forbidden") {
            return Self::permanent(text);
        }
        // Transient — another backend might succeed.
        if lower.contains("rate limit") || lower.contains("quota")
            || lower.contains("overloaded") || lower.contains("capacity")
            || lower.contains("503")
            || lower.contains("timeout") || lower.contains("timed out")
            || lower.contains("insufficient") || lower.contains("billing")
            || lower.contains("api error") || lower.contains("exceeded")
        {
            return Self::retriable(text);
        }
        // Unknown — assume permanent.
        Self::permanent(text)
    }
}

// ---------------------------------------------------------------------------
// The trait
// ---------------------------------------------------------------------------

/// Progress sender — backends call `send()` to stream updates.
pub type ProgressTx = mpsc::UnboundedSender<AiProgress>;

/// A pluggable AI backend.
///
/// Implementations live in sibling modules.  The trait is object-safe so
/// backends can be stored as `Arc<dyn AiBackend>`.
#[async_trait::async_trait]
pub trait AiBackend: Send + Sync + std::fmt::Debug {
    /// Unique identifier, e.g. `"claude-cli"`, `"openai-gpt4o"`.
    fn id(&self) -> &str;

    /// Human-readable display name.
    fn name(&self) -> &str;

    /// Metadata: categories, cost/speed/quality tiers.
    fn tags(&self) -> &EngineTags;

    /// Fast check — is the backend reachable / configured?
    async fn is_available(&self) -> bool;

    /// Execute a request.
    ///
    /// Progress events should be sent via `progress_tx`.  The channel is
    /// unbounded so the backend never blocks on progress delivery.
    async fn execute(
        &self,
        request: &AiRequest,
        progress_tx: ProgressTx,
    ) -> Result<AiResponse, AiError>;

    /// Estimate cost in micro-dollars for the given token counts.
    /// Returns 0 for local / free backends.
    fn estimate_cost(&self, _input_tokens: usize, _output_tokens: usize) -> u64 {
        0
    }
}

// ---------------------------------------------------------------------------
// Backend configuration (from TOML)
// ---------------------------------------------------------------------------

/// Per-backend configuration block in `ant.toml` or `backends.toml`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BackendConfig {
    /// Backend type: "cli", "openai", "anthropic", "ollama", "openai-compatible", "groq".
    #[serde(rename = "type")]
    pub backend_type: String,
    /// Model name / ID sent to the API.
    #[serde(default)]
    pub model: String,
    /// API base URL (for HTTP backends).
    #[serde(default)]
    pub api_base: String,
    /// Environment variable holding the API key.
    #[serde(default)]
    pub api_key_env: String,
    /// Direct API key (prefer `api_key_env` for security).
    #[serde(default)]
    pub api_key: String,
    /// Tags override.
    #[serde(default)]
    pub tags: Option<EngineTags>,
    /// Max tokens for the response.
    #[serde(default)]
    pub max_tokens: Option<usize>,
    /// Temperature (0.0–2.0).
    #[serde(default)]
    pub temperature: Option<f64>,
    /// Extra key-value options (backend-specific).
    #[serde(default)]
    pub extra: HashMap<String, String>,
}
