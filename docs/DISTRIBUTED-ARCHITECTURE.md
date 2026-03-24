# Anthill Distributed Architecture

**Version:** 1.0  
**Date:** 2026-03-25  
**Status:** Design Proposal

---

## 1. Overview

This document specifies the architectural evolution of Anthill to support:

1. **Pluggable AI Engines** — Easy addition of new AI backends with category-based selection
2. **Distributed Deployment** — Separate web frontend from reasoning engine(s)
3. **R2-TRUST Communication** — Secure, authenticated communication between components
4. **Per-ANT Configuration** — Each ANT can have different engine preferences

### 1.1 Motivation

Current limitations:
- AI backends are hardcoded CLI tools (claude, codex, gemini, ollama)
- Adding new backends requires code changes in multiple locations
- Web server and reasoning engine must run on same machine
- No support for engine categories (cost-effective, intellectual, fast, etc.)
- Limited extensibility for API-based backends

### 1.2 Goals

- **Extensibility**: Add new AI engines without modifying core code
- **Flexibility**: Per-ANT engine preferences, fallback chains, and runtime selection
- **Distribution**: Run web frontend on cloud VM, reasoning engines anywhere
- **Security**: R2-TRUST authentication for all inter-component communication
- **Performance**: Support for multiple concurrent AI backends per ANT

---

## 2. Architecture

### 2.1 Component Topology

All components — web server, reasoning engines, and browsers — are **devices
in the same colony trust group** (R2-TRUST).  This is intra-group
communication, not entanglement.  Entanglement is for bilateral peering
between *different* trust groups (R2-TRUST §7) and may be used in the
future for cross-colony collaboration.

```
┌─ Colony Trust Group (colony.key) ──────────────────────────────────┐
│                                                                     │
│  ┌─ Web Server (device: provisioned) ─────────────────────────┐    │
│  │  Serves dashboard, handles browser WebSocket connections   │    │
│  │  Relay WebSocket client → connects to reasoning engines    │    │
│  │  Routes requests transparently (local or remote ANTs)      │    │
│  └────────────────────────────┬───────────────────────────────┘    │
│                               │ relay WebSocket                     │
│                               │ (device credential HMAC auth)       │
│  ┌─ Reasoning Engine 1 ──────┼───────────────────────────────┐    │
│  │  (device: provisioned)    ←┘                               │    │
│  │  Runs ANT supervisor + AI workers                         │    │
│  │  Knowledge graph storage and rumination                   │    │
│  │  Relay WebSocket listener (port 3001)                     │    │
│  └────────────────────────────────────────────────────────────┘    │
│                                                                     │
│  ┌─ Reasoning Engine 2 (optional) ───────────────────────────┐    │
│  │  (device: provisioned)                                     │    │
│  │  Additional ANTs, different backends, GPU machine, etc.   │    │
│  └────────────────────────────────────────────────────────────┘    │
│                                                                     │
│  ┌─ Browser (device: provisioned via join code) ─────────────┐    │
│  │  WebSocket to web server, device credential HMAC auth     │    │
│  └────────────────────────────────────────────────────────────┘    │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
```

### 2.2 Trust Model

All devices in the colony share a single trust group identity (one
`colony.key`).  Each device — web server, reasoning engine, browser —
is provisioned via join code and receives an Ed25519 device credential.

**Authentication**: Device credentials are used for HMAC signing on
WebSocket connections (same mechanism for browsers and relay).  The
device presents its credential at WebSocket upgrade time; the colony
key holder verifies membership.

**Transport options** (all carry the same device-credential auth):
- **Tailscale VPN** (recommended): All devices on the same Tailscale
  network.  WireGuard encryption underneath.  Zero firewall config.
- **Direct IP + TLS**: Reasoning engine on public internet with
  `wss://`.  R2-TRUST prevents unauthorized access.
- **Localhost**: Both components on same machine.  `ws://127.0.0.1`.
  Auth still checked but transport encryption unnecessary.

**Not entanglement**: This is intra-group communication (R2-TRUST §3–§6).
Entanglement (R2-TRUST §7) is reserved for future cross-colony
collaboration — e.g. an ANT in one person's colony querying an ANT in
another person's colony.

### 2.3 Event Flow

User sends message on web dashboard:
```
1. Browser WebSocket → web server (device credential verified)
2. Web server looks up BotHandle for target ANT
3a. LOCAL ANT: sends CliRequest via in-process mpsc channel
3b. REMOTE ANT: sends RelayMessage::Chat over relay WebSocket
4. Reasoning engine receives, routes to local ANT's ai_worker
5. AI worker streams progress → WsEvent broadcast
5a. LOCAL: broadcast channel → web server → browser
5b. REMOTE: broadcast → relay WebSocket → web server broadcast → browser
6. Final result arrives the same way — browser sees no difference
```

---

## 3. AI Engine Abstraction

### 3.1 Category System

AI engines are tagged with categories:

```rust
pub enum EngineCategory {
    CostEffective,      // Lowest cost per token
    Intellectual,       // Best reasoning capability
    Fast,              // Fastest response time
    Local,             // On-premise only (privacy)
    Specialized(String), // Domain-specific (coding, vision, etc.)
    Balanced,          // Good mix of speed/cost/capability
}

pub struct EngineTags {
    pub categories: Vec<EngineCategory>,
    pub capabilities: Vec<String>, // "code", "vision", "function-calling"
    pub cost_tier: u8,             // 1-5 (1=cheapest, 5=most expensive)
    pub speed_tier: u8,            // 1-5 (1=fastest, 5=slowest)
    pub quality_tier: u8,          // 1-5 (1=basic, 5=best)
}
```

### 3.2 Backend Trait

All AI engines implement a common trait:

```rust
#[async_trait]
pub trait AiBackend: Send + Sync {
    /// Unique identifier for this backend
    fn id(&self) -> &str;
    
    /// Display name
    fn name(&self) -> &str;
    
    /// Engine tags and categories
    fn tags(&self) -> &EngineTags;
    
    /// Check if backend is available
    async fn is_available(&self) -> bool;
    
    /// Execute a request with streaming progress
    async fn execute(
        &self,
        request: &AiRequest,
        progress_tx: mpsc::UnboundedSender<AiProgress>,
    ) -> Result<AiResponse, AiError>;
    
    /// Estimate cost for a request (in microdollars)
    fn estimate_cost(&self, input_tokens: usize, output_tokens: usize) -> u64;
}

pub struct AiRequest {
    pub task_id: u32,
    pub chat_id: i64,
    pub message: String,
    pub system_prompt: Option<String>,
    pub working_dir: PathBuf,
    pub context: HashMap<String, String>,
    pub new_session: bool,
}

pub struct AiProgress {
    pub task_id: u32,
    pub progress_type: ProgressType,
    pub detail: String,
}

pub enum ProgressType {
    Thinking,
    ToolUse(String),
    Reading(String),
    Writing(String),
    Running(String),
}

pub struct AiResponse {
    pub task_id: u32,
    pub text: String,
    pub tokens_used: Option<(usize, usize)>, // (input, output)
    pub cost_microdollars: Option<u64>,
}
```

### 3.3 Backend Registry

```rust
pub struct BackendRegistry {
    backends: HashMap<String, Arc<dyn AiBackend>>,
    categories: HashMap<EngineCategory, Vec<String>>,
}

impl BackendRegistry {
    /// Register a new backend
    pub fn register(&mut self, backend: Arc<dyn AiBackend>);
    
    /// Find backends by category
    pub fn find_by_category(&self, category: &EngineCategory) -> Vec<Arc<dyn AiBackend>>;
    
    /// Find backend by ID
    pub fn get(&self, id: &str) -> Option<Arc<dyn AiBackend>>;
    
    /// Get all available backends
    pub async fn available_backends(&self) -> Vec<Arc<dyn AiBackend>>;
}
```

### 3.4 Backends to Implement

#### Phase 1: CLI-based (existing)
- [x] Claude CLI
- [x] Codex CLI
- [x] Gemini CLI
- [x] Ollama (HTTP API + embeddings)

#### Phase 2: Direct API
- [ ] OpenAI GPT-4/GPT-4o (API)
- [ ] Anthropic Claude API (direct, not CLI)
- [ ] Google Gemini API (direct, not CLI)
- [ ] DeepSeek API
- [ ] Groq API
- [ ] Together AI API
- [ ] OpenRouter (unified API)
- [ ] OpenCode (TBD - need specification)

#### Phase 3: Local Inference
- [ ] llama.cpp (local, fast)
- [ ] LocalAI (OpenAI-compatible local)
- [ ] vLLM (high-throughput local)

---

## 4. Configuration System

### 4.1 Per-ANT Engine Configuration

Each ANT's `ant.toml` specifies engine preferences:

```toml
name = "Dev Assistant"

[ai]
# Primary selection strategy
selection_strategy = "category"  # or "explicit", "cost-optimized", "quality-first"

# Default category for requests without explicit selection
default_category = "balanced"

# Explicit backend list with fallback order (alternative to categories)
# backends = ["claude", "openai-gpt4", "deepseek"]

# Category preferences - which backend for each category
[ai.categories]
cost_effective = ["deepseek", "groq", "ollama:llama3"]
intellectual = ["claude-opus", "openai-gpt4", "claude-sonnet"]
fast = ["groq-llama3", "openai-gpt4o-mini", "gemini-flash"]
local = ["ollama:llama3", "ollama:codellama"]
balanced = ["claude-sonnet", "openai-gpt4o", "gemini-pro"]
specialized = { coding = ["claude-sonnet", "openai-gpt4", "codex"], vision = ["openai-gpt4-vision", "gemini-vision"] }

# Cost limits
[ai.limits]
max_cost_per_request_usd = 0.50  # $0.50 max per request
max_daily_cost_usd = 10.0         # $10 max per day

# Backend-specific configuration
[ai.backends.openai]
api_key_env = "OPENAI_API_KEY"
model = "gpt-4o"
temperature = 0.7
max_tokens = 4096

[ai.backends.anthropic]
api_key_env = "ANTHROPIC_API_KEY"
model = "claude-sonnet-4"
max_tokens = 8192

[ai.backends.ollama]
base_url = "http://localhost:11434"
model = "llama3:70b"

[ai.backends.groq]
api_key_env = "GROQ_API_KEY"
model = "llama-3.1-70b-versatile"

# Runtime selection - allow user to override per request
allow_runtime_selection = true
```

### 4.2 Global Backend Configuration

In supervisor config or separate `backends.toml`:

```toml
# Global backend definitions - can be overridden per-ANT

[backends.openai-gpt4]
type = "openai"
model = "gpt-4"
api_base = "https://api.openai.com/v1"
tags = { categories = ["intellectual", "balanced"], cost_tier = 4, speed_tier = 3, quality_tier = 5 }

[backends.openai-gpt4o]
type = "openai"
model = "gpt-4o"
tags = { categories = ["fast", "balanced"], cost_tier = 3, speed_tier = 4, quality_tier = 4 }

[backends.claude-opus]
type = "anthropic"
model = "claude-opus-4"
tags = { categories = ["intellectual"], cost_tier = 5, speed_tier = 2, quality_tier = 5 }

[backends.claude-sonnet]
type = "anthropic"
model = "claude-sonnet-4-5"
tags = { categories = ["intellectual", "balanced"], cost_tier = 3, speed_tier = 3, quality_tier = 5 }

[backends.deepseek]
type = "openai-compatible"
model = "deepseek-coder"
api_base = "https://api.deepseek.com/v1"
tags = { categories = ["cost_effective"], cost_tier = 1, speed_tier = 3, quality_tier = 3 }

[backends.groq-llama3]
type = "groq"
model = "llama-3.1-70b-versatile"
tags = { categories = ["fast", "cost_effective"], cost_tier = 1, speed_tier = 5, quality_tier = 3 }

[backends.ollama-llama3]
type = "ollama"
model = "llama3:70b"
base_url = "http://localhost:11434"
tags = { categories = ["local", "cost_effective"], cost_tier = 1, speed_tier = 3, quality_tier = 3 }
```

### 4.3 Runtime Selection

Users can override engine selection per-request:

```
# In Telegram/Web
/model intellectual  # Use best reasoning model
/model fast         # Use fastest model
/model local        # Use local-only model
/model claude-opus  # Use specific backend

# Then send message
Tell me about quantum computing
```

---

## 5. Distributed Communication

### 5.1 R2 Events

Event definitions for Anthill distributed system:

```rust
// Anthill relay events (hash with FNV-1a)
pub const EVENT_ANTHILL_REQUEST: u64 = fnv1a("anthill.request");
pub const EVENT_ANTHILL_RESPONSE: u64 = fnv1a("anthill.response");
pub const EVENT_ANTHILL_PROGRESS: u64 = fnv1a("anthill.progress");
pub const EVENT_ANTHILL_STATUS: u64 = fnv1a("anthill.status");
pub const EVENT_ANTHILL_CANCEL: u64 = fnv1a("anthill.cancel");

// Event payloads (CBOR-encoded)
pub struct AnthillRequest {
    pub ant_id: String,
    pub task_id: u32,
    pub chat_id: i64,
    pub message: String,
    pub source: String,  // "web", "telegram", "slack"
    pub category: Option<String>,  // "intellectual", "fast", etc.
}

pub struct AnthillProgress {
    pub ant_id: String,
    pub task_id: u32,
    pub progress_type: String,
    pub detail: String,
}

pub struct AnthillResponse {
    pub ant_id: String,
    pub task_id: u32,
    pub chat_id: i64,
    pub text: String,
    pub backend_used: String,
    pub cost_usd: Option<f64>,
}
```

### 5.2 Web Gateway (Sentant + Plugin)

The web gateway runs its own R2 event bus with:

**web-gateway-sentant**: Routes web requests to reasoning engines
- States: `idle` → `routing` → `awaiting_response` → `idle`
- Subscribes: `#anthill.response`, `#anthill.progress`, `#anthill.status`
- Emits: `#anthill.request`, `#anthill.cancel`

**websocket-plugin**: Handles browser WebSocket connections
- Converts WebSocket messages to R2 events
- Authenticates with R2-TRUST device credentials
- Broadcasts progress to connected clients

**relay module** (`relay::WebGateway`): Manages WebSocket connections to
remote reasoning engines
- Authenticates with device credential at connect time
- Registers proxy `BotHandle`s in the local registry
- Forwards `CliRequest` → `RelayMessage::Chat`
- Receives `WsEvent`s and re-broadcasts locally
- Heartbeat every 30s, auto-reconnect on failure

### 5.3 Reasoning Node

Reasoning nodes run the full ANT supervisor with an additional relay
listener (`relay::run_engine_listener`):

- Accepts WebSocket connections from web servers
- Authenticates the connecting device as a colony member
- Sends `AntList` (which bots are hosted here)
- Relays local `WsEvent` broadcast back to web server
- Processes incoming `Chat`, `Cancel`, `FollowUp` messages

### 5.4 Relay Protocol

Intra-trust-group relay (not entanglement):

1. **Discovery**: Static config in `supervisor.toml` under `[relay]`
2. **Authentication**: Device credential presented at WebSocket upgrade
   - Colony key holder verifies membership (same as browser auth)
   - Session trusted after successful authentication
3. **Request Flow**:
   - Web server serializes `RelayMessage::Chat` as JSON
   - Wrapped in signed envelope (device_id, timestamp, HMAC)
   - Sent over WebSocket to reasoning engine
   - Engine deserializes, routes to local ANT via `request_tx`
4. **Response Flow**:
   - Reasoning engine subscribes to local `global_tx` broadcast
   - Each `WsEvent` wrapped in `RelayMessage::Event`
   - Sent back over relay WebSocket
   - Web server deserializes, re-broadcasts to its own `global_tx`
   - Browser WebSocket clients receive events as normal
5. **Heartbeat**: Every 30s, `RelayMessage::Heartbeat`

---

## 6. Implementation Plan

### Phase 1: Backend Abstraction (Week 1-2)

- [x] Design `AiBackend` trait and related types
- [ ] Create `BackendRegistry` with category indexing
- [ ] Refactor existing backends to implement trait:
  - [ ] `ClaudeCliBackend`
  - [ ] `CodexCliBackend`
  - [ ] `GeminiCliBackend`
  - [ ] `OllamaBackend`
- [ ] Update `ai_worker.rs` to use registry
- [ ] Add backend detection and registration at startup

### Phase 2: API Backends (Week 3-4)

- [ ] Implement `OpenAiBackend` with streaming support
- [ ] Implement `AnthropicApiBackend` with streaming
- [ ] Implement `GeminiApiBackend` with streaming
- [ ] Implement `DeepSeekBackend`
- [ ] Implement `GroqBackend`
- [ ] Implement `TogetherAiBackend`
- [ ] Implement `OpenRouterBackend` (unified API)
- [ ] Add cost estimation and tracking for all backends

### Phase 3: Configuration (Week 5)

- [ ] Extend `ant.toml` schema with `[ai]` section
- [ ] Create global `backends.toml` schema
- [ ] Implement category-based backend selection
- [ ] Add runtime selection via `/model` command
- [ ] Add cost tracking and limits per ANT

### Phase 4: Distributed Architecture (Week 6-8)

- [ ] Create `r2-anthill-relay` crate with event definitions
- [ ] Implement `web-gateway-sentant` and entanglement plugin
- [ ] Implement `reasoning-gateway-sentant`
- [ ] Add R2-TRUST bilateral entanglement support
- [ ] Create deployment configs for split architecture
- [ ] Document deployment scenarios

### Phase 5: Testing & Documentation (Week 9-10)

- [ ] Unit tests for all backends
- [ ] Integration tests for category selection
- [ ] End-to-end tests for distributed deployment
- [ ] Performance benchmarks
- [ ] Update user documentation
- [ ] Create deployment guide for cloud + local setup

---

## 7. Deployment Scenarios

### Scenario 1: Single Machine (Current)

All components run on one machine:
```
localhost:
  - Web server (port 3000)
  - Supervisor with ANTs
  - AI workers
  - Knowledge graphs
```

No changes needed - backward compatible.

### Scenario 2: Cloud Web + Local Reasoning

Web frontend in cloud, reasoning at home:
```
cloud-vm (Public IP):
  - Web gateway (port 443, HTTPS)
  - R2-TRUST trust group
  
home-server (Behind NAT, Tailscale):
  - Reasoning node with ANTs
  - AI workers (Claude CLI, Ollama)
  - Knowledge graphs
  
Transport: Tailscale VPN, direct IP+TLS, or localhost
```

### Scenario 3: Multi-Reasoning-Node

One web server, multiple reasoning engines (all same colony):
```
cloud-vm:
  - Web server (relay connects to both engines)
  
reasoning-node-1 (GPU server):
  - ANTs for coding tasks
  - Ollama with large models
  - relay.engine_listener = true
  
reasoning-node-2 (CPU server):
  - ANTs for general tasks
  - Claude CLI, API backends
  - relay.engine_listener = true
  
All three are devices in the same colony trust group.
```

---

## 8. Security Considerations

### 8.1 Single Colony Trust Group

- All components (web server, reasoning engines, browsers) are devices
  in the **same** colony trust group
- One `colony.key` — the Ed25519 root of trust
- Devices provision via join codes (5-minute expiry)
- Revoking a device immediately disconnects it

### 8.2 Device Authentication
- Replay protection via timestamp (60s max age)
- Keep-alive ensures liveness

### 8.3 API Key Management

- API keys never in events (stay in reasoning node config)
- Each ANT can have different API keys
- Keys loaded from environment or secure vault

### 8.4 Transport Security

- TCP connections over Tailscale (WireGuard encryption)
- Or TLS for internet-facing connections
- HMAC layer prevents MITM even on trusted transports

---

## 9. Migration Path

### Existing Users (Single Machine)

No changes required. New backend system is backward compatible.

Optional: Update `ant.toml` to use new `[ai]` section for category preferences.

### Distributed Deployment

All machines share the same colony.  The colony key lives on one machine
(the "key holder"); other machines are provisioned as devices via join codes.

1. **Set up colony** (on the reasoning engine machine):
   ```bash
   anthill --supervise   # creates colony.key on first run
   ```

2. **Provision the web server as a device**:
   ```bash
   # On reasoning engine:
   anthill --generate-join-code
   # → prints: a1b2-c3d4-e5f6

   # On web server:
   anthill --join <code>
   # → provisions device, saves credential
   ```

3. **Configure relay on reasoning engine** (`supervisor.toml`):
   ```toml
   [relay]
   engine_listener = true
   engine_port = 3001
   ```

4. **Configure relay on web server** (`supervisor.toml`):
   ```toml
   [relay]
   remote_engines = ["ws://reasoning-host:3001/relay"]
   credential = "<device credential hex>"
   device_id = "<device public key hex>"
   ```

5. **Start both**:
   ```bash
   # Reasoning engine (has ANTs):
   anthill --supervise

   # Web server (no local ANTs, just dashboard):
   anthill --supervise
   ```

   The web server connects to the reasoning engine via relay, discovers
   its ANTs, and registers proxy handles.  Browsers connecting to the
   web server see all ANTs — local and remote — in one dashboard.

---

## 10. Future Enhancements

### 10.1 Multi-Modal Support

- Vision models (GPT-4V, Gemini Vision)
- Audio models (Whisper, speech synthesis)
- Unified request interface

### 10.2 Backend Orchestration

- Smart routing based on current load
- Cost optimization across backends
- Quality-cost trade-off analysis

### 10.3 Knowledge Graph Sync

- Replicate knowledge graphs across reasoning nodes
- Conflict resolution for distributed updates
- CRDT-based eventual consistency

### 10.4 Federation

- ANTs from different colonies can collaborate
- Cross-colony knowledge sharing with permissions
- Marketplace for AI compute time

---

## 11. References

- [R2-INTRO](../../../r2-specifications/specs/r2-core/R2-INTRO.md) — Reality2 overview
- [R2-RELAY](../../../r2-specifications/specs/r2-core/R2-RELAY.md) — AI-augmented personal mesh
- [R2-SENTANT](../../../r2-specifications/specs/r2-core/R2-SENTANT.md) — Sentant lifecycle and properties
- [R2-TRUST](../../../r2-specifications/specs/r2-core/R2-AUTH.md) — Trust groups and entanglement
- [R2-WIRE](../../../r2-specifications/specs/r2-core/R2-WIRE.md) — Wire protocol
- [Anthill README](../README.md) — Current architecture

---

**Status**: Design proposal — ready for review and implementation
