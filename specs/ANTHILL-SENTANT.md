# ANTHILL-SENTANT: ANT Lifecycle, Properties, and Event Model

| Field      | Value                                                    |
|------------|----------------------------------------------------------|
| Version    | 0.1 Draft                                                |
| Date       | 2026-03-30                                               |
| Status     | Draft                                                    |
| Depends on | R2-SENTANT, R2-WIRE, R2-TRUST, R2-DEF, R2-PLUGIN        |
| Related    | ANTHILL-KNOWLEDGE, ANTHILL-WORKER, ANTHILL-THURISAZ      |

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT",
"SHOULD", "SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this
document are to be interpreted as described in [RFC 2119][rfc2119].

[rfc2119]: https://www.rfc-editor.org/rfc/rfc2119


## 1. Introduction

An ANT is an R2 sentant specialised for AI reasoning. It satisfies all
IPUCO+D properties defined in R2-SENTANT section 2 and adds the following
capabilities:

- **Persistent knowledge graph** -- a Popperian epistemic graph stored in
  CBOR and version-controlled with Git. Entities carry Bayesian confidence
  and typed evidence chains.
- **AI worker supervision** -- concurrent dispatch to multiple AI backends
  (Claude, Gemini, Codex, Ollama, LM Studio, Grok, DeepSeek, and others)
  with pluggable strategy-based selection.
- **Autonomous rumination** -- self-directed thinking during idle periods:
  refutation of existing beliefs, synthesis of transitive relationships,
  contradiction resolution, and open-ended initiative.
- **Inter-ANT communication** -- colony protocol for querying peer ANTs'
  knowledge graphs and exchanging messages via a shared event bus.


### 1.1 Terminology

| Term              | Definition                                                                     |
|-------------------|--------------------------------------------------------------------------------|
| ANT               | An R2 sentant of class `ai.reality2.ant` configured by an `ant.toml` file.     |
| Conductor         | The pure finite-state-machine sentant at the core of each ANT.                 |
| Worker            | A background task dispatching prompts to an AI backend and collecting output.   |
| Knowledge Store   | The validated, CBOR-backed epistemic graph accessible only through MCP tools.   |
| Rumination        | An autonomous cycle in which the ANT challenges, synthesises, or extends its knowledge graph while idle. |
| Colony            | The set of ANTs managed by a single supervisor, sharing an event bus.           |
| Working Directory | The filesystem root owned by an ANT, containing memory, files, repos, and reports. |
| Memory Directory  | A subdirectory of the working directory holding knowledge graphs, episodes, and per-user notes. |


### 1.2 Class Convention

ANTs use the class `ai.reality2.ant`. The conductor sentant registers with
the class hash `ai.reality2.relay.claude_cli` (FNV-1a/32).

Event names follow reverse-DNS notation. All events defined by this
specification use the prefix `relay.*` for intra-ANT events and `colony.*`
for inter-ANT events:

| Event              | FNV-1a Hash | Direction          |
|--------------------|-------------|--------------------|
| `relay.command`    | compile-time| Channel -> Conductor |
| `relay.ai_ready`   | compile-time| Worker -> Conductor  |
| `colony.query`     | compile-time| ANT -> ANT           |
| `colony.response`  | compile-time| ANT -> ANT           |


## 2. IPUCO+D Properties for ANTs

Each property defined by R2-SENTANT section 2 maps to the ANT context as
follows.

### 2.1 Immutable

An ANT's definition is fixed at load time. The conductor FSM, class hash,
event subscriptions, and plugin bindings are compiled into the binary and
MUST NOT change at runtime. Configuration values read from `ant.toml`
(system prompt, backend strategy, rumination parameters) are captured once
during the Load phase (section 7.1) and remain constant until the ANT is
restarted.

### 2.2 Persistent

An ANT exists from the moment its `ant.toml` is discovered by the
supervisor until it is explicitly unloaded (directory removed or
supervisor shutdown). Crash recovery is handled by the supervisor with
exponential backoff (section 7.5). The working directory and its contents
survive restarts.

### 2.3 Unique

There MUST be at most one ANT per directory name per colony. The
supervisor discovers ANT directories by scanning the configured `ants_dir`
and uses the directory name as the stable identifier for registry, history,
and event routing. Duplicate directory names are not possible within a
single filesystem tree.

### 2.4 Concurrent

Multiple AI workers MAY execute simultaneously within a single ANT. Each
incoming message spawns an independent worker task. Rumination runs as a
separate concurrent activity alongside interactive workers. The event bus
processes events on a 50ms tick; workers and plugins operate in their own
tokio tasks.

### 2.5 Observable

ANT state is observable through:

- **Web dashboard** -- real-time status, task list, and chat via WebSocket.
- **Telegram bot** -- bidirectional messaging (optional).
- **Slack bot** -- bidirectional messaging via Socket Mode (optional).
- **Registry events** -- `WsEvent::BotStatus` broadcasts status changes
  (`running`, `stopped`) to all connected observers.
- **History store** -- all user and bot messages are persisted to the
  history directory with timestamps and task IDs.

### 2.6 Deterministic

Given the same sequence of events, the conductor FSM produces the same
sequence of actions. The conductor is a pure function from (state, event)
to (state, actions) with no internal randomness. AI backend responses are
inherently non-deterministic, but the state machine transitions that
dispatch and consume those responses are deterministic.


## 3. ANT Definition Format

An ANT is defined by an `ant.toml` file conforming to R2-DEF. The file
MUST be located at `<ants_dir>/<ant-id>/ant.toml` where `<ant-id>` is the
directory name used as the stable identifier.

### 3.1 Top-Level Fields

| Field  | Type            | Required | Default        | Description                        |
|--------|-----------------|----------|----------------|------------------------------------|
| `name` | string          | No       | directory name | Display name shown in the web UI.  |

### 3.2 `[telegram]` Section

| Field   | Type      | Required | Default        | Description                                       |
|---------|-----------|----------|----------------|---------------------------------------------------|
| `token` | string    | No       | `$TELOXIDE_TOKEN` | Telegram Bot API token.                         |
| `allow` | int array | No       | [] (allow all) | Allowed chat IDs. Empty permits all senders.      |

### 3.3 `[slack]` Section

| Field       | Type   | Required | Default | Description                                   |
|-------------|--------|----------|---------|-----------------------------------------------|
| `bot_token` | string | No       | --      | Slack bot token (`xoxb-...`).                 |
| `app_token` | string | No       | --      | Slack app-level token for Socket Mode (`xapp-...`). |

Both fields MUST be present to enable Slack integration.

### 3.4 `[claude]` Section

| Field                      | Type     | Required | Default                                    | Description |
|----------------------------|----------|----------|--------------------------------------------|-------------|
| `backend_strategy`         | enum     | No       | `cost_optimized`                           | Backend selection strategy: `cost_optimized`, `capability_optimized`, `speed_optimized`, `reliability_optimized`, `balanced`, or `manual(["..."])`. |
| `backends`                 | string[] | No       | []                                         | Deprecated. Legacy backend list for backward compatibility. |
| `working_dir`              | string   | No       | `~/.config/anthill/ants/<id>/working`      | Filesystem root for this ANT. |
| `memory_dir`               | string   | No       | `"memory"`                                 | Memory subdirectory (relative to `working_dir`). |
| `repos_dir`                | string   | No       | `"repos"`                                  | Cloned repositories subdirectory (relative to `working_dir`). |
| `system_prompt`            | string   | No       | --                                         | Custom personality prefix injected into the system prompt. |
| `skip_permissions`         | bool     | No       | `true`                                     | Allow AI to run commands without interactive permission prompts. |
| `sync_channels`            | bool     | No       | `false`                                    | Synchronise user messages across web, Telegram, and Slack. |
| `encrypt_backups`          | bool     | No       | `false`                                    | Encrypt memory/ and files/ in Git backups. |
| `backup_interval_hours`    | uint     | No       | `0` (disabled)                             | Auto-backup interval in hours. |
| `backup_remote`            | string   | No       | `""` (local only)                          | Git remote name for backup pushes. |
| `worker_timeout_secs`      | uint     | No       | `600`                                      | Kill a worker if no output for this many seconds. |
| `allow_base_code_changes`  | bool     | No       | `false`                                    | Allow the AI to modify files outside the working directory. |

### 3.5 `[claude.rumination]` Section

| Field                      | Type     | Required | Default  | Description |
|----------------------------|----------|----------|----------|-------------|
| `enabled`                  | bool     | No       | `false`  | Enable the rumination engine. |
| `interval_secs`            | uint     | No       | `7200`   | Minimum interval between rumination cycles (seconds). |
| `min_idle_secs`            | uint     | No       | `300`    | Minimum idle time before a cycle may begin. |
| `topics`                   | string[] | No       | []       | Topic graphs to focus on. Empty means all topics. |
| `refutation_enabled`       | bool     | No       | `true`   | Challenge existing beliefs by attempting refutation. |
| `synthesis_enabled`        | bool     | No       | `true`   | Conjecture transitive relationships across entities. |
| `contradiction_resolution` | bool     | No       | `true`   | Pit conflicting beliefs against each other. |
| `initiative_enabled`       | bool     | No       | `false`  | Open-ended self-improvement and initiative. |

### 3.6 `[ai]` Section

When present, the `[ai]` section takes precedence over
`claude.backend_strategy` for backend selection. The `[claude]` section
remains authoritative for workspace paths, system prompt, and other
non-AI settings.

| Field                       | Type                  | Required | Default | Description |
|-----------------------------|-----------------------|----------|---------|-------------|
| `default_category`          | string                | No       | `""`    | Default engine category (e.g. `"balanced"`, `"intellectual"`, `"fast"`). |
| `backends`                  | string[]              | No       | []      | Explicit backend ID list with fallback order. |
| `categories`                | map[string, string[]] | No       | {}      | Named categories mapping to ordered backend ID lists. |
| `allow_runtime_selection`   | bool                  | No       | `false` | Allow users to override engine selection per-request. |
| `max_cost_per_request_usd`  | float                 | No       | `0.0`   | Maximum cost per request in USD (0 = unlimited). |
| `max_daily_cost_usd`        | float                 | No       | `0.0`   | Maximum daily cost in USD (0 = unlimited). |
| `backends_config`           | map[string, object]   | No       | {}      | Per-backend configuration blocks. |


## 4. Conductor FSM

The conductor is the ANT's core finite-state machine. It is a pure
`Sentant` implementation: no I/O, no channels, no shared state. It
receives small CBOR event payloads (< 256 bytes), makes decisions, and
emits `Action::plugin_call()` to the AI plugin.

### 4.1 States

The conductor defines a single logical state:

| StateId | Name    | Description                              |
|---------|---------|------------------------------------------|
| 0       | `ready` | Accepting events. All dispatch is immediate. |

The conductor is stateless by design -- concurrency and task tracking are
delegated to the AI plugin and worker pool. The FSM always returns to
`ready` after processing an event.

> **Note:** Higher-level states (idle, processing, ruminating) are
> observable at the plugin and worker layer, not at the conductor level.
> The conductor does not block on AI responses.

### 4.2 Events

The conductor subscribes to two event hashes:

| Event            | Payload Schema                                         | Description |
|------------------|--------------------------------------------------------|-------------|
| `relay.command`  | `{ 0: uint(cmd_type), 1: uint(chat_id), 2?: uint(cancel_task_id) }` | A classified user command from any input channel. |
| `relay.ai_ready` | `{ 0: uint(kind), 1: uint(chat_id) }`                 | An AI worker has completed; response is queued. |

### 4.3 Command Codes

When a `relay.command` event arrives, the conductor reads key 0 from the
CBOR payload to determine the command type:

| Code | Constant           | Command        | Action                                        |
|------|--------------------|----------------|-----------------------------------------------|
| 0x00 | `CMD_TYPE_MESSAGE` | (plain text)   | Dispatch message to AI worker via `CMD_DISPATCH`. |
| 0x01 | `CMD_TYPE_HELP`    | `/help`        | Send help text via `CMD_HELP`.                |
| 0x02 | `CMD_TYPE_ANTS`    | `/ants`        | List running workers via `CMD_ANTS`.          |
| 0x03 | `CMD_TYPE_USAGE`   | `/usage`       | Show session statistics via `CMD_USAGE`.      |
| 0x04 | `CMD_TYPE_CANCEL`  | `/cancel [id]` | Cancel a specific task via `CMD_CANCEL`. Reads key 2 for the target task ID. |
| 0x05 | `CMD_TYPE_CANCEL_ALL` | `/cancel all` | Cancel all running tasks via `CMD_CANCEL_ALL`. |
| 0x06 | `CMD_TYPE_NEW`     | `/new`         | Start fresh session via `CMD_NEW_SESSION`.    |
| 0x07 | `CMD_TYPE_STATUS`  | `/status`      | Show live worker status via `CMD_STATUS`.     |
| 0x08 | `CMD_TYPE_FOLLOWUP`| `/followup`    | Queue follow-up for running task via `CMD_FOLLOWUP`. |
| 0x09 | `CMD_TYPE_ANALYSE` | `/analyse`     | Thematic analysis of a file via `CMD_ANALYSE`. |
| 0x0A | `CMD_TYPE_REFLECT` | `/reflect`     | Meta-analysis of the knowledge graph via `CMD_REFLECT`. |
| 0x0B | `CMD_TYPE_SPECIFY` | `/specify`     | Generate specification from code via `CMD_SPECIFY`. |
| 0x0C | `CMD_TYPE_TEST_VECTORS` | `/test-vectors` | Generate test vectors via `CMD_TEST_VECTORS`. |
| 0x0D | `CMD_TYPE_RUMINATE`| `/ruminate`    | Trigger manual rumination cycle via `CMD_RUMINATE`. |

Unknown command types MUST be treated as plain messages and dispatched to
the AI worker.

### 4.4 AI Plugin Command Constants

The conductor communicates with the AI plugin via numeric command codes
passed in `Action::plugin_call()`:

| Code | Constant          | Description                               |
|------|-------------------|-------------------------------------------|
| 0x01 | `CMD_DISPATCH`   | Dispatch a message to the AI backend.     |
| 0x02 | `CMD_CANCEL`     | Cancel a task by ID.                      |
| 0x03 | `CMD_CANCEL_ALL` | Cancel all running tasks.                 |
| 0x04 | `CMD_HELP`       | Send the help text to the user.           |
| 0x05 | `CMD_ANTS`       | Send the running task list.               |
| 0x06 | `CMD_USAGE`      | Send usage statistics.                    |
| 0x07 | `CMD_REPLY`      | Pop a completed response and deliver it.  |
| 0x08 | `CMD_NEW_SESSION`| Start a new conversation session.         |
| 0x09 | `CMD_STATUS`     | Show live status of workers.              |
| 0x0A | `CMD_FOLLOWUP`   | Queue a follow-up for a running task.     |
| 0x0B | `CMD_ANALYSE`    | Thematic analysis of a file.              |
| 0x0C | `CMD_REFLECT`    | Meta-analysis / reflect on knowledge.     |
| 0x0D | `CMD_SPECIFY`    | Generate a specification from code.       |
| 0x0E | `CMD_TEST_VECTORS`| Generate test vectors from code/spec.    |
| 0x0F | `CMD_RUMINATE`   | Trigger rumination cycle.                 |
| 0x10 | `CMD_QUESTIONS`  | Show pending questions from rumination.   |
| 0x11 | `CMD_CITATIONS`  | Run citation consolidation task.          |

### 4.5 Response Flow

When `relay.ai_ready` fires, the conductor emits `CMD_REPLY` to the AI
plugin, which pops the response from its queue and delivers it to the
originating channel (Telegram, Slack, or web).


## 5. Plugin Bindings

An ANT binds the following R2 plugins. Each plugin communicates with the
conductor via the R2 event bus and with peer plugins via shared data
planes (typed channels).

### 5.1 `ai.reality2.ai` -- AI Worker Dispatch

- Receives `CMD_DISPATCH` and spawns concurrent AI backend tasks.
- Tracks running tasks (`TaskMap`), per-user statistics (`StatsMap`), and
  a follow-up queue (`FollowUpQueue`).
- Pushes completed responses to a `VecDeque<CliResponse>` polled by the
  event loop.
- Fires `relay.ai_ready` when a response is available.

### 5.2 `ai.reality2.knowledge` -- Knowledge Graph (MCP Tool Server)

- Runs as an MCP server over stdio (JSON-RPC, protocol version
  `2024-11-05`).
- Exposes validated graph operations: `graph_add_node`, `graph_add_edge`,
  `graph_add_citation`, `graph_update_evidence`, `graph_strengthen`,
  `graph_weaken`, `graph_contradict`, `graph_query_about`,
  `graph_query_uncertain`, `graph_list_nodes`.
- All writes go through the `ValidatedKnowledgeStore` trait -- the AI
  MUST NOT write graph files directly.
- The MCP server is auto-configured in `.claude/settings.json` at ANT
  startup.

### 5.3 `ai.reality2.telegram` -- Telegram Bot (Optional)

- Enabled when `[telegram].token` is set or `$TELOXIDE_TOKEN` is present.
- Pushes incoming messages to the shared `MessageQueue` data plane.
- Receives outgoing messages via an `mpsc::UnboundedSender<(i64, String)>`.
- Enforces the allow-list from `[telegram].allow`.

### 5.4 `ai.reality2.slack` -- Slack Bot (Optional)

- Enabled when both `[slack].bot_token` and `[slack].app_token` are set.
- Connects via Slack Socket Mode.
- Pushes incoming messages to the shared `MessageQueue` data plane.

### 5.5 `ai.reality2.web` -- Web Dashboard Channel

- Always enabled. The supervisor runs an HTTP server with WebSocket
  support.
- Provides real-time chat, task status, configuration editing, and
  history browsing.
- Broadcasts `WsEvent` messages for all status changes and responses.


## 6. Working Directory Structure

Each ANT owns a working directory with the following layout. The
implementation MUST create `memory/`, `files/`, and `repos/`
subdirectories at startup if they do not exist.

```
<working_dir>/
+-- .claude/settings.json    # MCP server auto-config (generated at startup)
+-- .git/                    # Git repository for version-controlled backups
+-- memory/
|   +-- knowledge.cbor       # Meta-graph (top-level entity index)
|   +-- graphs/              # Topic-specific graphs
|   |   +-- <topic>.cbor     # One CBOR file per topic graph
|   +-- episodes.json        # Episodic memory (timestamped conversation summaries)
|   +-- thinking_process.md  # Self-evolved methodology (the ANT can modify this)
|   +-- questions.json       # Pending questions for the human from rumination
|   +-- rumination_log.json  # Rumination cycle history
|   +-- reputation.json      # Source reputation registry
|   +-- colony_inbox/        # Messages received from other ANTs
|   +-- colony_outbox/       # Messages queued for other ANTs
|   +-- <chat_id>.md         # Per-user freeform memory notes
+-- files/                   # User-uploaded files and cached citations
+-- repos/                   # Cloned Git repositories (excluded from backup)
+-- reports/                 # Generated reports (analyses, specifications, etc.)
```

### 6.1 MCP Auto-Configuration

At startup, the ANT writes `.claude/settings.json` to register the MCP
knowledge graph server. This ensures that all CLI-based AI backends
operating in the working directory can discover and call the graph tools
without manual setup. The server is launched as:

```
anthill --mcp-server --memory-dir <memory_dir>
```

### 6.2 Git Backup

The working directory is initialised as a Git repository at startup
(`backup::ensure_git_repo`). When `backup_interval_hours > 0`, the
maintenance loop commits changes at the configured interval. If
`encrypt_backups` is true, the `memory/` and `files/` directories are
encrypted before commit. If `backup_remote` is set, commits are pushed
to the named remote.


## 7. ANT Lifecycle

### 7.1 Load

1. The supervisor discovers `<ants_dir>/<ant-id>/ant.toml`.
2. Configuration is parsed via `Config::load()`. Missing fields receive
   defaults per section 3.
3. The working directory tree (section 6) is created if absent.
4. MCP settings are written to `.claude/settings.json`.
5. The Git repository is initialised or verified.
6. The AI plugin is constructed with shared queues (response, message,
   tasks, stats, follow-ups).
7. The conductor sentant is registered on the R2 event bus with the AI
   plugin's `PluginId`.
8. The AI worker loop is spawned as a background tokio task.
9. The maintenance loop is spawned for backup, rumination, and
   housekeeping.
10. The ANT is registered in the `BotRegistry` with status `running`.

### 7.2 Run

The event bus runs a polling loop. On each 50ms tick:

1. Plugins are polled for incoming events (Telegram messages, Slack
   messages, web commands, completed AI responses).
2. Events are dispatched to subscribed sentants.
3. The conductor processes events and emits plugin-call actions.
4. Actions are executed, advancing the state of workers and channels.

### 7.3 Stop

1. Running AI worker tasks are aborted.
2. The ANT is removed from the `BotRegistry`.
3. A `WsEvent::BotStatus { status: "stopped" }` is broadcast.
4. The tokio task completes.

### 7.4 Restart

1. The supervisor detects the stopped task via `JoinHandle::is_finished()`.
2. Configuration is re-read from disk (`Config::load()` on the original
   `ant.toml` path), so any changes made while the ANT was running take
   effect.
3. A new bot task is spawned with the fresh configuration.
4. A `WsEvent::BotStatus { status: "running" }` is broadcast.

### 7.5 Crash Recovery

The supervisor implements crash recovery with exponential backoff:

- **Base delay:** `restart_delay_secs` (default 5 seconds).
- **Backoff:** base * 2^(attempt - 1), capped at 300 seconds (5 minutes).
- **Maximum restarts:** `max_restarts` (default 10, 0 = unlimited).
- **Config reload:** each restart re-reads `ant.toml` from disk.

When `max_restarts` is exceeded, the supervisor logs an error and stops
attempting to restart the ANT. The ANT remains in `stopped` status until
the supervisor is signalled to reload (e.g. via the web dashboard).


## 8. System Prompt Construction

The system prompt is assembled by `build_system_prompt()` with a total
budget of **16,384 bytes** (`MAX_SYSTEM_PROMPT`). It is constructed once
per AI worker dispatch and consists of fixed and dynamic sections.

### 8.1 Fixed Sections

Fixed sections are included unconditionally and consume a variable but
bounded amount of space:

1. **File access restriction** -- when `allow_base_code_changes` is false,
   a policy statement restricting writes to the working directory.
2. **Knowledge graph restriction** -- a critical notice that all graph
   operations MUST go through MCP tools. Lists all available tool names.
3. **Custom system prompt** -- the `[claude].system_prompt` value, if set.
4. **Workspace preamble** -- working directory path and repos directory.
5. **Memory preamble** -- memory file locations and citation rules.
6. **Methodology preamble** -- included only for analytical commands
   (`/analyse`, `/reflect`, `/specify`, `/test-vectors`), saving
   approximately 1KB for regular requests.
7. **Thinking process** -- contents of `memory/thinking_process.md`
   (self-evolved methodology), truncated to 2,048 bytes.
8. **Colony directory** -- a pre-populated list of peer ANTs and their
   topic graphs, so the AI need not call `list_colony_ants` on every
   request.

### 8.2 Dynamic Sections

After fixed sections are placed, the remaining budget is allocated
proportionally:

| Section          | Budget Share | Source File(s)                        |
|------------------|-------------|---------------------------------------|
| Knowledge graph  | 70%         | `memory/knowledge.cbor`, `memory/graphs/*.cbor` |
| User memory      | 15%         | `memory/<chat_id>.md`                 |
| Episodes         | 15%         | `memory/episodes.json`                |

Each section is truncated at its budget boundary with an ellipsis
indicator. If the knowledge graph context exceeds its allocation, a
note about semantic search via embeddings is appended.

### 8.3 Overflow

If the assembled prompt exceeds `MAX_SYSTEM_PROMPT`, a warning is logged
but the prompt is still used. Implementations SHOULD monitor for overflow
warnings and adjust custom prompt length or knowledge graph density
accordingly.


## 9. Conformance

An implementation conforms to ANTHILL-SENTANT if it satisfies all of the
following requirements:

1. **IPUCO+D properties** -- The implementation MUST satisfy the Immutable,
   Persistent, Unique, Concurrent, Observable, and Deterministic properties
   as defined in section 2.

2. **Conductor FSM** -- The implementation MUST provide a conductor sentant
   that processes all command codes defined in section 4.3 and emits the
   corresponding plugin-call actions defined in section 4.4. Unknown
   command codes MUST be treated as plain messages.

3. **ANT definition format** -- The implementation MUST accept `ant.toml`
   files conforming to the schema in section 3, applying the specified
   defaults for all optional fields.

4. **Plugin bindings** -- The implementation MUST bind the `ai.reality2.ai`
   and `ai.reality2.knowledge` plugins. The `ai.reality2.telegram`,
   `ai.reality2.slack`, and `ai.reality2.web` plugins are OPTIONAL but
   MUST conform to their specified interfaces when present.

5. **Working directory structure** -- The implementation MUST create and
   maintain the directory layout defined in section 6, including MCP
   auto-configuration at startup.

6. **System prompt construction** -- The implementation MUST construct
   system prompts within the 16,384-byte budget using the allocation
   strategy defined in section 8.

7. **Crash recovery** -- The implementation MUST support crash recovery
   with configuration reload as defined in section 7.5. The exponential
   backoff parameters MUST be configurable via the supervisor
   configuration.

8. **Event encoding** -- All event payloads between the conductor and
   plugins MUST use CBOR encoding with integer keys as defined in
   section 4.2.
