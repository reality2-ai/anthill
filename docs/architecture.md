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
| Conductor | Classifies commands, dispatches to plugin calls. Routes /help, /status, /cancel, /followup, /new, /ruminate, /questions. |

A sentant's `handle_event` method does three things:
1. Match the event hash
2. Decide what to do (which plugin command, with what parameters)
3. Push `Action::plugin_call()` into the action buffer

That's it. No `send_telegram()`, no `pop_response()`, no mutex locks. Pure logic.

## Plugins (I/O adapters)

Plugins handle everything the sentants can't: network calls, file access, process spawning, data buffering.

| Plugin | Manages |
|---|---|
| AIPlugin | Worker dispatch, task tracking, stats, follow-up queue, message queue, rumination triggers |
| TelegramPlugin | Bot API polling, message classification, outgoing sender, data plane queue |
| SlackPlugin | Socket Mode WebSocket, message classification, data plane queue |

Plugins communicate with each other through the **data plane** — shared `Arc<Mutex<VecDeque>>` queues, `mpsc` channels. For example:

- TelegramPlugin stores the full message text in a `MessageQueue`
- AIPlugin reads from that same queue when the sentant tells it to dispatch
- The event between them carries only `{cmd_type: 0, chat_id: 123}` — 12 bytes

## Knowledge store architecture

The knowledge system is layered with clear boundaries:

```
Consumers (MCP tools, Web API, Rumination, Maintenance)
    |
    v
KnowledgeStore trait — validated writes, anti-bias enforcement
    |
    v
LiveKnowledgeStore — in-memory cache (RwLock<HashMap<String, KnowledgeGraph>>)
    |
    v
GraphEngine (petgraph) — Bayesian updates, queries, consolidation
    |
    v
CborGitBackend — CBOR serialisation, atomic writes, auto-commit to git
```

### The KnowledgeStore trait

All consumers — MCP tools, the web API, the maintenance engine, and the rumination system — interact through the `KnowledgeStore` trait. This trait enforces:

- **Validated writes** — `ValidatedNode`, `ValidatedEdge`, and `ValidatedEvidence` types enforce field constraints at construction time. If you have one of these types, it is guaranteed valid. Invalid data is rejected with a clear error message through MCP.
- **Bayesian updates** — the `update_evidence()` method applies typed evidence with predefined Bayes factors, reputation-adjusted via the Thurisaz engine.
- **Anti-confirmation bias** — every evidence update checks for consecutive-confirmation patterns, evidence diversity, and suspicious one-sided trails. Warnings are returned to the caller.
- **Auto-commit** — every mutation is persisted to CBOR and auto-committed to git with a descriptive message.

The AI cannot edit graph files directly. This is structural, not policy.

### CBOR+Git backend

Graphs are stored as CBOR binary files (via ciborium), approximately 46% smaller than equivalent JSON. The backend:

- **Atomic writes** — write to tmp file, fsync, rename. Prevents corruption on power loss.
- **Auto-commit** — every `save_graph()` call stages the memory directory and commits to git.
- **Legacy compatibility** — reads from JSON files when no CBOR file exists, enabling migration from older versions.
- **Multiple graphs** — meta-graph at `memory/knowledge.cbor`, topic graphs at `memory/graphs/<topic>.cbor`.

### Bayesian epistemic engine (Thurisaz)

Confidence updates use log-odds for numerical stability:

```
log_odds' = log_odds + ln(BF_adjusted)
```

Where the Bayes factor is adjusted for source reputation:

```
BF_adjusted = BF_base ^ (0.5 + 0.5 * reputation)
```

This means:
- Untrusted sources (r=0): BF is square-rooted (dampened)
- Neutral sources (r=0.5): BF^0.75
- Fully trusted (r=1.0): full BF

After each update, two anti-confirmation-bias mechanisms are applied:

1. **Evidence diversity ceiling** — confidence is capped based on how many different types of evidence appear in the trail:
   - 1 type: max 70%
   - 2 types: max 85%
   - 3 types: max 92%
   - 4+ types: max 99%

2. **Consecutive-confirmation dampening** — if the last 5+ evidence entries are all positive (BF > 1.0), the update is dampened by pulling confidence back toward its pre-update value.

### Fading foundations

Beliefs decay toward p=0.5 (log-odds=0) over time:

```
log_odds(t) = log_odds(t_last) * 2^(-elapsed / half_life)
```

Half-lives vary by decay category (Fact: 30 days, Decision: 14 days, Observation: 7 days, Inference: 3 days, Assumption: 1 day). This resolves Agrippa's trilemma: epistemic chains converge without requiring absolute foundations (Peijnenburg & Atkinson, 2017).

## Worker and supervision

Each AI request spawns a **worker** — a tokio task running an AI backend process. Workers are supervised:

- **Watchdog** per worker: monitors stdout activity, warns at 2 minutes idle, kills at configurable timeout
- **Stderr capture**: read concurrently with stdout to prevent pipe deadlock, included in error messages
- **Process groups**: `process_group(0)` + `kill_on_drop` ensures cancel kills the entire process tree
- **Follow-up queue**: messages queued for running tasks, dispatched with session continuity when the task completes
- **Multi-backend fallback**: try each configured backend in order (Claude, Codex, Gemini, Ollama); fall back on failure/rate limits with error classification (retriable vs non-retriable)
- **Task state machine**: Running -> Completed/Cancelled/Failed lifecycle tracking
- **System prompt budgeting**: 16KB cap with priority allocation (70% knowledge graph, 15% user memory, 15% episodes); methodology instructions only for analytical commands

## Memory architecture

Each ANT has three memory systems plus supporting files:

### Knowledge store (CBOR+Git)

A directed graph of entities and conjectural relationships, following **Popperian epistemology** with **Bayesian confidence dynamics** (Thurisaz engine). All edges carry:

- **Log-odds confidence** — numerical stability for sequential updates
- **Evidence log** — full audit trail of typed evidence with Bayes factors
- **Justificatory chain** — provenance: "why do I believe this?"
- **Decay category** — controls how quickly this belief fades without fresh evidence
- **Beneficial impact** — fitness modifier for Darwinian competition
- **Corroboration strength** — how strongly this edge is supported by its network neighbourhood
- **Competition group** — which competing hypotheses this edge belongs to

Features:
- **Cached in memory** — loaded once via `LiveKnowledgeStore`, accessed through `RwLock` for concurrent reads
- **Graph query API** — traversal (BFS), path-finding (shortest paths with cumulative confidence), kind filtering, uncertainty queries, justification chains
- **Hybrid retrieval** — combines Ollama embeddings (nomic-embed-text) for semantic similarity with TF-IDF keyword matching; falls back to keyword-only when Ollama is unavailable
- **MAGMA-inspired edge views** — edges carry semantic, temporal, causal, and entity classification metadata for multi-perspective graph queries
- **Temporal validity** — edges have `valid_from`/`valid_until` fields (Zep-inspired) for time-scoped knowledge
- **Community detection** — GraphRAG-inspired connected component analysis during consolidation identifies knowledge clusters
- **Consolidation** — periodic deduplication (with Levenshtein fuzzy matching), parallel edge merging (MAX confidence), chain collapsing, contradiction detection, community detection
- **Corruption recovery** — on parse failure, preserves corrupted file and recovers from archive
- **Archiving** — low-confidence edges moved to archive files

### Episodic memory (`episodes.json`)

Timestamped conversation summaries — what happened, who was involved, what was decided. The knowledge graph captures facts; episodes capture stories.

### Per-user memory (`{chat_id}.md`)

Freeform notes about individual users — name, role, preferences.

## Rumination engine

The maintenance thread runs a periodic rumination cycle when the ANT is idle. The cycle includes:

1. Compute corroboration strength across all topic graphs
2. Synthesis — create transitive edges (A->B->C => A->C)
3. Undetermined connection investigation ('?' edges)
4. Darwinian competition — pit competing hypotheses against each other
5. Cross-domain pattern transfer between topic graphs
6. Active refutation — challenge important but uncertain edges
7. Contradiction resolution
8. Autonomous initiative — identify knowledge gaps
9. Meta-rumination — review and evolve the thinking process itself

After each cycle: consolidation, orphan linking, git commit.

## Analysis pipelines

Anthill includes AI-driven analysis pipelines built on Braun & Clarke's thematic analysis methodology (2022). The same pattern — familiarise, code, theme, review, refine, integrate — applies to different inputs:

| Pipeline | Input | Output |
|---|---|---|
| `/analyse <file>` | Any document | Entities, themes, relationships -> knowledge graph |
| `/reflect` | The knowledge graph itself | Patterns, contradictions, consolidation -> refined graph |
| `/specify <file>` | Source code | Behaviors, invariants -> formal RFC 2119 specification |
| `/test-vectors <file>` | Code or spec | Test cases (normal, edge, error, security) -> Rust `#[test]` stubs |

The key insight: **codes** in thematic analysis map to **graph nodes** (entities, behaviors), **themes** map to **higher-level concept nodes**, and **relationships** map to **conjectural edges** with confidence levels. This makes the thematic analysis -> Popperian graph pipeline natural.

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

The sentant touches zero bytes of message text. It only routes IDs. The knowledge store is consulted before each request (relevant context injected into the prompt) and updated after each response (AI applies evidence through MCP tools).

## Supervisor

In production mode (`--supervise`), the supervisor:

1. Discovers ANT configs in the `ants/` directory
2. Spawns each ANT on a dedicated thread (EventBus is `!Send`)
3. Starts the web server with auth middleware
4. Starts the history recorder (listens to broadcast events)
5. Monitors ANT tasks, restarts crashed ones with true exponential backoff (2^attempt, capped at 5 min)
6. Periodically runs rumination cycles and consolidation
7. Auto-commits knowledge state after each rumination cycle

ANTS register their handles with a shared `BotRegistry`, which the web server reads for the dashboard. ANTS that exist on disk but aren't running show as "Configured" in the UI.

## Why R2?

The same architecture that coordinates sensor readings from ESP32 accelerometers now coordinates AI conversations from phones. The sentant model works because:

- **Determinism** — sentants are testable in isolation, no mock I/O needed
- **Portability** — the same sentant code could run on an ESP32, a Linux server, or an Elixir node
- **Separation** — changing the Telegram plugin to a Matrix plugin doesn't touch any sentant code
- **Security** — trust group provisioning is the same ceremony for a phone and a microcontroller
- **Scale** — the event bus handles any number of sentants; plugins scale independently
