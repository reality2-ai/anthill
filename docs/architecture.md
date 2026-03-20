# Architecture

Anthill is built on the Reality2 (R2) sentant engine — the same event-driven architecture used for IoT sensor networks on ESP32 microcontrollers.

## Core principles

**Events carry decisions. Plugins carry data.**

This is the fundamental rule. If it fits in 256 bytes, it's a decision — put it in an event. If it's larger (message text, AI responses, file contents), it's data — put it in the plugin data plane.

The 256-byte event limit isn't a constraint to work around. It's the design. It enforces the separation between the two worlds:

- **Event bus** — small, deterministic, platform-independent. IDs, codes, state signals.
- **Plugin data plane** — large, platform-specific, I/O-bound. Shared queues, channels, network calls.

## Sentants (pure state machines)

Every sentant in Anthill is a pure FSM. No channels, no shared state, no I/O. Given the same sequence of events, a sentant always produces the same sequence of actions.

| Sentant | Role |
|---|---|
| Conductor | Classifies commands, dispatches to plugin calls. Routes /help, /status, /cancel, /followup, /new. |

A sentant's `handle_event` method does three things:
1. Match the event hash
2. Decide what to do (which plugin command, with what parameters)
3. Push `Action::plugin_call()` into the action buffer

That's it. No `send_telegram()`, no `pop_response()`, no mutex locks. Pure logic.

## Plugins (I/O adapters)

Plugins handle everything the sentants can't: network calls, file access, process spawning, data buffering.

| Plugin | Manages |
|---|---|
| AIPlugin | Worker dispatch, task tracking, stats, follow-up queue, message queue |
| TelegramPlugin | Bot API polling, message classification, outgoing sender, data plane queue |
| SlackPlugin | Socket Mode WebSocket, message classification, data plane queue |

Plugins communicate with each other through the **data plane** — shared `Arc<Mutex<VecDeque>>` queues, `mpsc` channels. For example:

- TelegramPlugin stores the full message text in a `MessageQueue`
- AIPlugin reads from that same queue when the sentant tells it to dispatch
- The event between them carries only `{cmd_type: 0, chat_id: 123}` — 12 bytes

## Worker and supervision

Each AI request spawns a **worker** — a tokio task running an AI backend process. Workers are supervised:

- **Watchdog** per worker: monitors stdout activity, warns at 2 minutes idle, kills at configurable timeout
- **Stderr capture**: read concurrently with stdout to prevent pipe deadlock, included in error messages
- **Process groups**: `process_group(0)` + `kill_on_drop` ensures cancel kills the entire process tree
- **Follow-up queue**: messages queued for running tasks, dispatched with session continuity when the task completes
- **Multi-backend fallback**: try each configured backend in order (Claude, Codex, Ollama); fall back on failure/rate limits

## Memory architecture

Each ANT has three memory systems:

### Knowledge graph (`memory/knowledge.json`)

A directed graph of entities and conjectural relationships, following **Popperian epistemology**. All edges are conjectures with confidence weights that strengthen through surviving refutation.

- **Cached in memory** — loaded once, reloaded when the AI modifies the file
- **Graph query API** — traversal (BFS), path-finding (shortest paths with cumulative confidence), kind filtering, uncertainty queries
- **Semantic retrieval** — Ollama embeddings (nomic-embed-text) enable semantic search over graph nodes; falls back to keyword extraction when Ollama is unavailable
- **MAGMA-inspired edge views** — edges carry semantic, temporal, causal, and entity classification metadata for multi-perspective graph queries
- **Temporal validity** — edges have `valid_from`/`valid_until` fields (Zep-inspired) for time-scoped knowledge
- **Provenance tracking** — edges carry a `source` field for "why do I believe this?" tracing
- **Community detection** — GraphRAG-inspired connected component analysis during consolidation identifies knowledge clusters
- **Episode entity linking** — episodes link to graph nodes via an `entities[]` field for cross-referencing narrative and structured knowledge
- **Context-aware prompt** — for small graphs, full render; for large graphs, entity extraction from user message + graph traversal (or semantic nearest-neighbour when embeddings available)
- **Consolidation** — periodic deduplication, parallel edge merging, chain collapsing, contradiction detection, community detection
- **Confidence decay** — time-based (24h idle trigger), not just request-count-based
- **Archiving** — low-confidence edges moved to `knowledge-archive.json`

### Episodic memory (`memory/episodes.json`)

Timestamped conversation summaries — what happened, who was involved, what was decided. The knowledge graph captures facts; episodes capture stories.

### Per-user memory (`memory/{chat_id}.md`)

Freeform notes about individual users — name, role, preferences.

## Analysis pipelines

Anthill includes AI-driven analysis pipelines built on Braun & Clarke's thematic analysis methodology (2022). The same pattern — familiarise, code, theme, review, refine, integrate — applies to different inputs:

| Pipeline | Input | Output |
|---|---|---|
| `/analyse <file>` | Any document | Entities, themes, relationships → knowledge graph |
| `/reflect` | The knowledge graph itself | Patterns, contradictions, consolidation → refined graph |
| `/specify <file>` | Source code | Behaviors, invariants → formal RFC 2119 specification |
| `/test-vectors <file>` | Code or spec | Test cases (normal, edge, error, security) → Rust `#[test]` stubs |

The key insight: **codes** in thematic analysis map to **graph nodes** (entities, behaviors), **themes** map to **higher-level concept nodes**, and **relationships** map to **conjectural edges** with confidence levels. This makes the thematic analysis → Popperian graph pipeline natural.

## Trust group security

The colony implements R2-TRUST provisioning with Ed25519 device identity:

1. **Colony key** — Ed25519 signing key, generated on first run (`colony.key`)
2. **Join codes** — 48-bit single-use tokens (`xxxx-xxxx-xxxx`), valid 5 minutes
3. **Device credentials** — Ed25519 private key seed, issued at join time
4. **Auth middleware** — every API call verified via `X-Credential` header
5. **WebSocket signing** — HMAC-SHA256 envelopes for transport integrity

The server is the **queen** — it exists the moment Anthill starts. Browsers and phones are **viewers** that join via join codes or QR scans.

## Event flow

![Event Flow](https://mermaid.ink/img/c2VxdWVuY2VEaWFncmFtCiAgICBwYXJ0aWNpcGFudCBVc2VyIGFzIFVzZXIKICAgIHBhcnRpY2lwYW50IFRQIGFzIFRlbGVncmFtUGx1Z2luCiAgICBwYXJ0aWNpcGFudCBNUSBhcyBNZXNzYWdlUXVldWUKICAgIHBhcnRpY2lwYW50IEJ1cyBhcyBFdmVudCBCdXMKICAgIHBhcnRpY2lwYW50IFMgYXMgQ29uZHVjdG9yCiAgICBwYXJ0aWNpcGFudCBDUCBhcyBBSVBsdWdpbgogICAgcGFydGljaXBhbnQgVyBhcyBXb3JrZXIrV2F0Y2hkb2cKICAgIHBhcnRpY2lwYW50IEMgYXMgQ2xhdWRlIENvZGUKICAgIHBhcnRpY2lwYW50IEtHIGFzIEtub3dsZWRnZSBHcmFwaAoKICAgIFVzZXItPj5UUDogZXhwbGFpbiB0aGlzIGNvZGUKICAgIFRQLT4+TVE6IHN0b3JlIGZ1bGwgdGV4dAogICAgVFAtPj5CdXM6IFJFTEFZX0NPTU1BTkQgY21kOjAgY2hhdDoxMjMKICAgIEJ1cy0+PlM6IGV2ZW50IDEyIGJ5dGVzCiAgICBTLT4+QnVzOiBwbHVnaW5fY2FsbCBDTURfRElTUEFUQ0gKICAgIEJ1cy0+PkNQOiBleGVjdXRlIENNRF9ESVNQQVRDSAogICAgQ1AtPj5NUTogcG9wIGZ1bGwgdGV4dAogICAgQ1AtPj5XOiBDbGlSZXF1ZXN0CiAgICBXLT4+S0c6IGxvYWQgcmVsZXZhbnQgY29udGV4dAogICAgVy0+PkM6IGNsYXVkZSAtcCAod2l0aCBrbm93bGVkZ2UgZ3JhcGggaW4gcHJvbXB0KQogICAgTm90ZSBvdmVyIEM6IFdvcmtpbmcuLi4KICAgIEMtLT4+Vzogc3RyZWFtLWpzb24gcHJvZ3Jlc3MKICAgIFctLT4+Q1A6IFRhc2tQcm9ncmVzcyBldmVudHMKICAgIEMtPj5XOiByZXN1bHQgdGV4dAogICAgVy0+PktHOiB1cGRhdGUgY29uamVjdHVyZXMKICAgIFctPj5DUDogQ2xpUmVzcG9uc2UKICAgIENQLT4+QnVzOiBSRUxBWV9BSV9SRUFEWQogICAgQnVzLT4+UzogZXZlbnQgMTIgYnl0ZXMKICAgIFMtPj5CdXM6IHBsdWdpbl9jYWxsIENNRF9SRVBMWQogICAgQnVzLT4+Q1A6IGV4ZWN1dGUgQ01EX1JFUExZCiAgICBDUC0+PlRQOiByZXNwb25zZSB2aWEgZGF0YSBwbGFuZQogICAgVFAtPj5Vc2VyOiBmb3JtYXR0ZWQgcmVzcG9uc2UK)

The sentant touches zero bytes of message text. It only routes IDs. The knowledge graph is consulted before each request (relevant context injected into the prompt) and updated after each response (AI maintains conjectures).

## Supervisor

In production mode (`--supervise`), the supervisor:

1. Discovers ANT configs in the `ants/` directory
2. Spawns each ANT on a dedicated thread (EventBus is `!Send`)
3. Starts the web server with auth middleware
4. Starts the history recorder (listens to broadcast events)
5. Monitors ANT tasks, restarts crashed ones with exponential backoff
6. Periodically consolidates knowledge graphs and archives stale conjectures

ANTS register their handles with a shared `BotRegistry`, which the web server reads for the dashboard. ANTS that exist on disk but aren't running show as "Configured" in the UI.

## Why R2?

The same architecture that coordinates sensor readings from ESP32 accelerometers now coordinates AI conversations from phones. The sentant model works because:

- **Determinism** — sentants are testable in isolation, no mock I/O needed
- **Portability** — the same sentant code could run on an ESP32, a Linux server, or an Elixir node
- **Separation** — changing the Telegram plugin to a Matrix plugin doesn't touch any sentant code
- **Security** — trust group provisioning is the same ceremony for a phone and a microcontroller
- **Scale** — the event bus handles any number of sentants; plugins scale independently
