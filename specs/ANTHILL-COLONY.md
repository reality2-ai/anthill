# ANTHILL-COLONY: Colony Supervisor and ANT Lifecycle

| Field      | Value                                                        |
|------------|--------------------------------------------------------------|
| Version    | 0.1 Draft                                                    |
| Date       | 2026-03-30                                                   |
| Status     | Draft                                                        |
| Depends on | ANTHILL-SENTANT, ANTHILL-TRUST                               |
| Related    | ANTHILL-WORKER, ANTHILL-COMMS, ANTHILL-FEDERATION             |

> The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT",
> "SHOULD", "SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this
> document are to be interpreted as described in RFC 2119.

---

## 1. Introduction

A colony is the top-level runtime unit of Anthill. It consists of a single
supervisor process that discovers, spawns, monitors, and restarts one or
more ANTs (Autonomous iNTelligenceS). The supervisor also hosts the web
dashboard HTTP server and an inter-ANT event bus (the AntBus) through which
colony members communicate.

### 1.1 Scope

This specification defines:

- The supervisor configuration schema (`supervisor.toml`).
- The ANT discovery algorithm.
- The ANT spawning model, including thread isolation and runtime layout.
- Crash detection, exponential backoff restart, and restart limits.
- The reload protocol for hot-adding and hot-restarting ANTs.
- The Bot Registry that exposes running ANT state to the web server.
- The event broadcasting architecture for real-time dashboard updates.
- The per-ANT maintenance loop (consolidation, cross-linking, rumination).

### 1.2 Terminology

| Term              | Definition                                                                                          |
|-------------------|-----------------------------------------------------------------------------------------------------|
| Colony            | A group of ANTs managed by a single supervisor process.                                              |
| Supervisor        | The top-level async loop that discovers, spawns, monitors, and restarts ANTs.                        |
| ANT               | Autonomous iNTelligenceS. A single AI agent running as an isolated task within the colony.           |
| Bot Registry      | A shared data structure that maps ANT names to their runtime handles, enabling web server interaction.|
| AntBus            | The inter-ANT event bus through which colony members exchange queries and knowledge.                 |
| Reload Signal     | An mpsc message from the web server requesting the supervisor to re-discover and re-spawn ANTs.      |
| Hot-add           | Spawning a newly discovered ANT without restarting the supervisor or existing ANTs.                  |

### 1.3 Design Principles

1. **Isolation.** Each ANT runs on a dedicated OS thread with its own
   single-threaded tokio runtime. This isolation ensures that a panic or
   blocking operation in one ANT cannot stall others.

2. **Crash resilience.** The supervisor MUST detect ANT termination and
   restart crashed ANTs automatically, subject to configurable limits and
   exponential backoff.

3. **Live reconfiguration.** ANT configuration is re-read from disk on
   every restart. The reload protocol allows adding new ANTs and restarting
   existing ANTs with changed configuration without stopping the supervisor.

4. **Observable.** All lifecycle transitions (start, stop, crash, restart)
   MUST be broadcast as events so that dashboards and adapters can reflect
   colony state in real time.

---

## 2. Supervisor Configuration

The supervisor reads its configuration from `supervisor.toml` in the colony
root directory. If this file does not exist, the supervisor MUST use the
default values specified below.

### 2.1 Schema

```toml
# supervisor.toml

# Directory containing ANT subdirectories, relative to the colony root.
# Each subdirectory that contains an ant.toml is treated as an ANT.
# Default: "ants"
ants_dir = "ants"

# Automatically restart ANTs that crash or exit unexpectedly.
# Default: true
restart_on_crash = true

# Base delay in seconds before restarting a crashed ANT.
# Used as the base for exponential backoff (see Section 5).
# Default: 5
restart_delay_secs = 5

# Maximum consecutive restarts before the supervisor gives up on an ANT.
# A value of 0 means unlimited restarts.
# Default: 10
max_restarts = 10

# TCP port for the web dashboard HTTP server.
# Default: 3000
http_port = 3000

# Bind address for the web dashboard HTTP server.
# Default: "0.0.0.0"
http_bind = "0.0.0.0"

# [relay] — distributed deployment configuration (see ANTHILL-FEDERATION).
```

### 2.2 Requirements

- The supervisor MUST parse `supervisor.toml` using TOML format.
- All fields MUST have defaults as specified above; a missing field MUST NOT
  cause a parse error.
- The `ants_dir` path MUST be resolved relative to the directory containing
  `supervisor.toml`.
- If `ants_dir` does not exist on disk, the supervisor MUST create it.
- The `http_bind` and `http_port` values MUST be combined to form a valid
  `SocketAddr`. The supervisor MUST report an error and exit if the address
  is invalid or the port cannot be bound.

---

## 3. ANT Discovery

### 3.1 Algorithm

The `discover_ants` function scans the `ants_dir` directory for ANT
definitions. The algorithm is as follows:

1. Read all entries in `ants_dir` using a directory listing.
2. For each entry that is a directory, check whether it contains a file
   named `ant.toml`.
3. If `ant.toml` exists, the directory name becomes the ANT's stable
   identifier and the path to `ant.toml` becomes its configuration source.
4. Entries that are not directories, or directories without `ant.toml`,
   MUST be silently ignored.
5. The resulting list MUST be sorted lexicographically by ANT identifier
   (directory name) to ensure deterministic startup order.

### 3.2 Error Handling

- If `ants_dir` cannot be read (e.g., permission denied), the supervisor
  MUST log an error and return an empty ANT list. It MUST NOT terminate.
- If no ANTs are discovered, the supervisor MUST log a warning and continue
  running. The web dashboard remains available for creating new ANTs.

### 3.3 Hot-Add via Reload

When the supervisor receives a reload signal (Section 6), it re-runs the
discovery algorithm. Any newly discovered ANT identifier that is not
already present in the running task set MUST be spawned as a new task.
This mechanism allows operators to create a new ANT directory and trigger
a reload without restarting the supervisor.

---

## 4. ANT Spawning

### 4.1 Thread Isolation Model

Each ANT MUST be spawned on a dedicated OS thread using
`tokio::task::spawn_blocking`. Within that thread, the implementation MUST
create a new single-threaded tokio runtime (`Runtime::new_current_thread`)
and a `LocalSet`. This design is REQUIRED because the R2 `EventBus` is
`!Send` -- it cannot be shared across threads -- and therefore MUST run on
a `LocalSet` bound to a single-threaded runtime.

A conforming implementation MUST ensure that:

- Each ANT's tokio runtime is independent; a blocked or panicking ANT MUST
  NOT prevent other ANTs from making progress.
- The `JoinHandle` returned by `spawn_blocking` is retained by the
  supervisor for crash detection (Section 5).

### 4.2 The `run_bot` Sequence

When an ANT is spawned, the `run_bot` function executes the following
steps in order:

1. **Create directories.** The working directory, memory directory
   (`memory/` relative to working dir), repos directory (`repos/` relative
   to working dir), and files directory (`files/` within working dir) MUST
   be created if they do not exist. The working directory defaults to
   `~/.config/anthill/ants/<id>/working` if not specified in `ant.toml`.

2. **Ensure MCP settings.** The function MUST configure MCP (Model Context
   Protocol) server settings in the ANT's working directory so that graph
   tools (`graph_add_node`, `graph_add_edge`, etc.) are available to all
   CLI-based AI backends. Settings files are written to:
   - `.claude/settings.json` (Claude Code)
   - `.gemini/settings.json` (Gemini CLI)

   If a settings file already exists and contains the `anthill-graph` MCP
   server entry, it MUST NOT be overwritten. If the file exists without the
   entry, the entry MUST be merged into the existing `mcpServers` object.

3. **Register with BotRegistry.** A `BotHandle` MUST be inserted into the
   shared `BotRegistry` (Section 7) under the ANT's directory name. The
   handle carries the display name, working directory path, request channel
   sender, shared stats, task map, follow-up queue, per-ANT event sender,
   and an initial status of `Running`.

4. **Create EventBus.** An R2 `EventBus` MUST be created. The Telegram
   plugin, Slack plugin (both OPTIONAL), AI plugin, and conductor sentant
   MUST be registered on this bus. A shared message queue bridges input
   plugins and the AI plugin on the data plane.

5. **Spawn AI worker.** The AI worker loop MUST be spawned as an async
   task. It receives requests via an unbounded mpsc channel and dispatches
   them to configured AI backends (see ANTHILL-WORKER).

6. **Register on AntBus.** If the AntBus is available, the ANT MUST
   register itself and spawn a listener task that handles `colony.query`
   events from other ANTs. Colony queries are forwarded to the AI worker
   with `chat_id = -2` and a source string of the form
   `colony:<from_ant>:<chat_id>`.

7. **Spawn backup loop.** If `backup_interval_hours > 0` in the ANT's
   configuration, a Git backup task MUST be spawned. The backup loop
   commits the working directory to a local Git repository at the
   configured interval, optionally pushing to a remote and optionally
   encrypting sensitive directories.

8. **Spawn maintenance loop.** A maintenance task MUST be spawned for every
   ANT (see Section 9). This task handles consolidation, cross-linking,
   and rumination independently of user interaction.

9. **Initialise EventBus.** `bus.init_all()` MUST be called to initialise
   all registered plugins.

10. **Enter main loop.** The ANT enters an infinite loop with a 50ms tick
    interval. Each tick MUST:
    - Call `bus.poll_plugins()` to collect inbound events from plugins.
    - Call `bus.advance_time(elapsed_ms)` to update timers.
    - Call `bus.tick()` to process the event queue and advance sentant state.
    - Call `bus.drain_outbound()` to deliver outbound events to plugins.
    - Sleep for 50 milliseconds.

### 4.3 Per-ANT Event Channel

Each ANT creates a per-ANT broadcast channel with a capacity of 256
entries. Events on this channel are forwarded to the global broadcast
channel when the ANT runs in supervisor mode. The per-ANT channel exists
so that single-ANT (non-supervisor) deployments can still observe events.

---

## 5. Crash Recovery

### 5.1 Detection

The supervisor's monitor loop runs every 2 seconds. On each iteration, the
supervisor MUST check every retained `JoinHandle` by calling
`is_finished()`. A handle that reports `true` indicates that the
corresponding ANT has exited -- either cleanly or due to a panic.

### 5.2 Status Notification

When a stopped ANT is detected, the supervisor MUST:

1. Update the ANT's `BotStatusKind` in the BotRegistry to `Stopped`.
2. Broadcast a `BotStatus` event with `status: "stopped"` on the global
   channel.

### 5.3 Restart Policy

If `restart_on_crash` is `false`, the supervisor MUST NOT attempt to
restart the stopped ANT. The ANT remains in `Stopped` status.

If `restart_on_crash` is `true`, the supervisor MUST apply the following
restart logic:

1. **Increment restart counter.** A per-ANT counter tracks consecutive
   restarts. The counter is keyed by ANT identifier (directory name).

2. **Check restart limit.** If `max_restarts > 0` and the counter exceeds
   `max_restarts`, the supervisor MUST log an error and MUST NOT restart
   the ANT. The ANT remains stopped permanently until the supervisor is
   restarted or a manual reload is triggered.

3. **Compute backoff delay.** The delay before restart MUST be calculated
   using exponential backoff:

   ```
   delay = min(restart_delay_secs * 2^(attempt - 1), 300)
   ```

   where `attempt` is the current value of the restart counter (1-indexed)
   and the exponent is capped at 6 (i.e., `2^6 = 64`). The maximum delay
   is 300 seconds (5 minutes), regardless of the base delay or attempt
   count.

   Example with the default `restart_delay_secs = 5`:

   | Attempt | Delay (seconds) |
   |---------|-----------------|
   | 1       | 5               |
   | 2       | 10              |
   | 3       | 20              |
   | 4       | 40              |
   | 5       | 80              |
   | 6       | 160             |
   | 7       | 300 (capped)    |
   | 8-10    | 300 (capped)    |

4. **Re-read configuration.** The ANT's `ant.toml` MUST be re-read from
   disk before restarting. This ensures that configuration changes made
   while the ANT was stopped (e.g., disabling rumination, changing the
   system prompt) take effect on restart. If re-reading fails, the
   supervisor MUST fall back to the previously loaded configuration.

5. **Re-spawn.** A new task MUST be spawned via the same `spawn_bot_task`
   mechanism described in Section 4.1. The old `JoinHandle` MUST be
   replaced with the new one.

6. **Broadcast restart.** A `BotStatus` event with `status: "running"`
   MUST be broadcast on the global channel.

### 5.4 Restart Counter Reset

The restart counter is NOT automatically reset on successful operation.
A reload signal (Section 6) that causes an ANT to be removed from
`ant_tasks` and re-discovered effectively resets the counter because the
ANT is treated as a new entry.

---

## 6. Reload Protocol

### 6.1 Trigger

The web server holds a `reload_tx` sender (mpsc channel, capacity 1). When
an operator creates a new ANT, edits an ANT's configuration, or issues a
restart command through the dashboard, the web server sends a unit message
`()` on this channel.

### 6.2 Processing

On each monitor loop iteration, the supervisor checks `reload_rx` with a
non-blocking `try_recv()`. If a reload signal is received, the supervisor
MUST execute the following steps:

1. **Prune finished tasks.** Iterate `ant_tasks` and remove any entry
   whose `JoinHandle` reports `is_finished() == true`. This allows stopped
   ANTs to be re-discovered in the next step.

2. **Re-discover ANTs.** Run `discover_ants` on the `ants_dir` to obtain
   the current set of ANT identifiers on disk.

3. **Spawn new ANTs.** For each discovered ANT identifier that is not
   present in the current running task set, load its configuration via
   `Config::load` and spawn it using `spawn_bot_task`. Log the hot-add.

4. **Detect config changes.** For each discovered ANT identifier that IS
   present in the running task set, load its configuration and compare it
   to the stored configuration. If the configuration has changed:
   - Abort the running task by calling `handle.abort()`.
   - Remove the ANT from `ant_tasks`.
   - Re-spawn the ANT with the new configuration.
   This mechanism allows runtime reconfiguration without restarting the
   entire supervisor.

### 6.3 Restart-ANT Workflow

To restart a specific ANT via the web dashboard, the implementation uses
the following sequence:

1. Remove the ANT from the BotRegistry (marking it as stopped).
2. Send a reload signal.
3. On reload, the supervisor prunes the finished task, re-discovers the
   ANT (its directory still exists on disk), and re-spawns it with a fresh
   `Config::load` -- picking up any configuration changes.

---

## 7. Bot Registry

### 7.1 Structure

The `BotRegistry` is an `Arc`-wrapped structure shared between the
supervisor, all ANT tasks, and the web server. It contains:

| Field        | Type                                              | Description                                           |
|--------------|---------------------------------------------------|-------------------------------------------------------|
| `bots`       | `RwLock<HashMap<String, BotHandle>>`              | Map from ANT identifier to runtime handle.            |
| `global_tx`  | `broadcast::Sender<WsEvent>`                      | Global event broadcast channel (capacity 256).        |
| `ants_dir`   | `PathBuf`                                         | Path to the ANT configuration directory on disk.      |

### 7.2 BotHandle

Each running ANT is represented by a `BotHandle`:

| Field          | Type                                          | Description                                         |
|----------------|-----------------------------------------------|-----------------------------------------------------|
| `name`         | `String`                                      | Stable identifier (directory name).                 |
| `display_name` | `String`                                      | Human-readable name from `ant.toml` (or dir name).  |
| `working_dir`  | `PathBuf`                                     | Absolute path to the ANT's working directory.       |
| `request_tx`   | `mpsc::UnboundedSender<CliRequest>`           | Channel to send user messages to the AI worker.     |
| `stats`        | `Arc<Mutex<HashMap<String, Stats>>>`          | Per-backend runtime statistics.                     |
| `tasks`        | `Arc<Mutex<HashMap<u32, TaskInfo>>>`          | Currently running AI tasks, keyed by task ID.       |
| `follow_ups`   | `Arc<Mutex<HashMap<u32, Vec<FollowUp>>>>`     | Queued follow-up messages per task.                  |
| `event_tx`     | `broadcast::Sender<WsEvent>`                  | Per-ANT event channel (capacity 256).               |
| `status`       | `Arc<RwLock<BotStatusKind>>`                  | Current lifecycle status.                           |

### 7.3 BotStatusKind

An ANT's status MUST be one of the following:

| Variant      | Description                                                         |
|--------------|---------------------------------------------------------------------|
| `Running`    | The ANT is actively processing events.                              |
| `Stopped`    | The ANT has exited (cleanly or via crash) and is not running.       |
| `Configured` | The ANT exists on disk but has not been started in this session.    |
| `Error(msg)` | The ANT encountered a fatal error; `msg` describes the cause.      |

### 7.4 Registry Operations

The BotRegistry MUST support the following operations:

- **`list_bots`** -- Merge running ANTs (from memory) with configured ANTs
  (from disk). ANTs present on disk but not running MUST appear with status
  `Configured`. The result MUST be sorted by display name.
- **`send_message`** -- Route a user message to a specific ANT's AI worker
  via its `request_tx`. Lookup MUST be case-insensitive.
- **`ask_ant`** -- Send a colony query from one ANT to another. The target
  ANT's AI worker receives the query with `chat_id = -2` and a source
  string encoding the return address as `colony:<from_ant>:<chat_id>`.
- **`read_config`** -- Read an ANT's `ant.toml` from disk.
- **`write_config`** -- Write an ANT's `ant.toml` to disk, creating the
  directory if necessary.
- **`delete_config`** -- Remove an ANT's configuration directory.
- **`list_config_dirs`** -- List all ANT directories on disk that contain
  `ant.toml`, sorted lexicographically.

---

## 8. Event Broadcasting

### 8.1 Architecture

The colony uses a two-tier broadcast channel architecture:

1. **Global channel.** A single `broadcast::Sender<WsEvent>` with capacity
   256, owned by the `BotRegistry`. The web server subscribes to this
   channel to push real-time updates to connected dashboard clients via
   WebSocket. The history recorder also subscribes to persist messages.

2. **Per-ANT channel.** Each ANT creates its own `broadcast::Sender<WsEvent>`
   with capacity 256. In supervisor mode, per-ANT events are forwarded to
   the global channel. In standalone mode (single ANT, no supervisor), the
   per-ANT channel serves as the sole event source.

### 8.2 WsEvent Variants

All events MUST be serialised with a `type` discriminator tag. The
following variants are defined:

| Variant          | Tag (serde)        | Fields                                           | Description                                                |
|------------------|--------------------|--------------------------------------------------|------------------------------------------------------------|
| `Message`        | `message`          | `bot`, `chat_id`, `text`, `task_id`              | An ANT produced a chat response.                           |
| `UserMessage`    | `user_message`     | `bot`, `chat_id`, `text`, `source`               | A user sent a message (for history and cross-channel sync).|
| `TaskStarted`    | `task_started`     | `bot`, `task_id`, `preview`                      | A new AI task began processing.                            |
| `TaskProgress`   | `task_progress`    | `bot`, `task_id`, `kind`, `detail`               | Real-time progress from a running task.                    |
| `TaskCompleted`  | `task_completed`   | `bot`, `task_id`, `duration_secs`                | A task finished successfully.                              |
| `TaskError`      | `task_error`       | `bot`, `task_id`, `error`                        | A task failed or timed out.                                |
| `BotStatus`      | `bot_status`       | `bot`, `status`                                  | An ANT's lifecycle status changed.                         |
| `GraphUpdated`   | `graph_updated`    | `bot`, `graph`, `source`                         | A knowledge graph was modified.                            |
| `Typing`         | `typing`           | `bot`                                            | Typing indicator for the dashboard.                        |

### 8.3 TaskProgress Kinds

The `kind` field of `TaskProgress` MUST be one of:

- `"tool_use"` -- the AI is invoking a tool.
- `"agent_spawn"` -- the AI spawned a sub-agent.
- `"text"` -- the AI is producing text output.

The `detail` field provides a human-readable description, e.g.,
`"Running: ls -la"` or `"Reading: src/main.rs"`.

### 8.4 Lag Handling

Broadcast channels have bounded capacity. If a subscriber falls behind
(e.g., a slow WebSocket client), it MUST receive a `Lagged(n)` error
indicating the number of missed events. Implementations SHOULD log a
warning when lag occurs but MUST NOT terminate the subscriber.

### 8.5 History Recording

The supervisor MUST spawn a history recorder task that subscribes to the
global broadcast channel and persists `Message` and `UserMessage` events.
Messages are stored per-ANT with role (`"bot"` or `"user"`), text, task
ID, and timestamp. The history store is made available to the web server
for rendering conversation logs.

---

## 9. Maintenance Loop

### 9.1 Overview

Every ANT MUST have a dedicated maintenance loop spawned as an async task.
This loop performs three categories of background work:

1. **Consolidation** -- structural maintenance of knowledge graphs.
2. **Cross-linking** -- discovery of shared entities across topic graphs.
3. **Rumination** -- autonomous AI thinking when the ANT is idle.

The maintenance loop waits 60 seconds after spawn before its first check,
then polls every 300 seconds (5 minutes).

### 9.2 Consolidation

Consolidation runs on each topic graph at a configurable interval. The
default interval is 900 seconds (15 minutes) as passed to the maintenance
loop by the bot runner. Operations include:

- Deduplication of equivalent nodes.
- Merging of near-identical conjectures.
- Collapsing of redundant edges.
- Decay of untested conjectures (fading foundations).
- Contradiction detection and clustering.

All changes are logged. When a graph is modified, a `GraphUpdated` event
with `source: "consolidation"` SHOULD be broadcast.

### 9.3 Cross-Linking

Cross-linking runs at a configurable interval. The default interval is
21600 seconds (6 hours). It scans all topic graphs to find shared entities
(nodes with the same or similar labels across different topic graphs) and
creates cross-reference edges in the meta-graph.

When cross-links are created, a `GraphUpdated` event with
`source: "cross-linking"` SHOULD be broadcast.

### 9.4 Rumination

Rumination is the autonomous thinking engine described in ANTHILL-RUMINATE.
It is governed by the `[claude.rumination]` section of `ant.toml`:

| Field                      | Type       | Default | Description                                           |
|----------------------------|------------|---------|-------------------------------------------------------|
| `enabled`                  | `bool`     | `false` | Enable the rumination engine.                         |
| `interval_secs`            | `u64`      | `7200`  | Minimum interval between rumination cycles (seconds). |
| `min_idle_secs`            | `u64`      | `300`   | Minimum idle time before ruminating (seconds).        |
| `topics`                   | `Vec`      | `[]`    | Topic graphs to focus on. Empty means all topics.     |
| `refutation_enabled`       | `bool`     | `true`  | Challenge existing beliefs with counter-evidence.     |
| `synthesis_enabled`        | `bool`     | `true`  | Conjecture transitive relationships.                  |
| `contradiction_resolution` | `bool`     | `true`  | Pit conflicting beliefs against each other.           |
| `initiative_enabled`       | `bool`     | `false` | Open-ended autonomous self-improvement.               |

Rumination MUST only run when all of the following conditions are met:

1. `rumination.enabled` is `true`.
2. A `request_tx` channel to the AI worker is available.
3. The ANT is idle -- the task map is empty (no running AI tasks).
4. The ANT has been continuously idle for at least `min_idle_secs`.
5. At least `interval_secs` have elapsed since the last rumination cycle.

Idle tracking MUST be reset whenever the ANT has active tasks. When the
ANT transitions from busy to idle, the idle timer restarts from zero.

Rumination requests are sent to the AI worker with `chat_id = -1`
(the rumination system chat ID), distinguishing them from user messages
(positive chat IDs) and colony queries (`chat_id = -2`).

Each rumination cycle consists of the following phases, executed in order:

1. **Corroboration strength** -- recompute confidence scores across all
   graphs (no AI tokens required).
2. **Synthesis** -- detect and create transitive relationship edges (no AI
   tokens required).
3. **Undetermined connections** -- investigate edges marked with `?`
   relation type using AI reasoning.
4. **Competition** -- pit similar ideas against each other using AI
   evaluation.
5. **Active refutation** -- challenge high-confidence beliefs with
   counter-arguments (if `refutation_enabled`).
6. **Contradiction resolution** -- identify and resolve contradictory
   beliefs (if `contradiction_resolution`).
7. **Autonomous initiative** -- open-ended self-directed thinking (if
   `initiative_enabled`).

Each phase is an atomic "thought" -- changes are committed as a single
unit to the knowledge store. All rumination prompts MUST include a stop
directive instructing the AI to complete its work and stop without asking
follow-up questions.

---

## 10. Conformance

### 10.1 REQUIRED

A conforming colony supervisor implementation MUST:

1. Parse `supervisor.toml` with all fields defaulting per Section 2.1.
2. Discover ANTs per the algorithm in Section 3.1, including
   lexicographic sort.
3. Spawn each ANT on an isolated thread with a single-threaded tokio
   runtime and `LocalSet` as described in Section 4.1.
4. Execute the `run_bot` sequence (Section 4.2) including directory
   creation, MCP settings, registry insertion, AI worker spawn,
   maintenance loop spawn, EventBus initialisation, and the 50ms main
   loop.
5. Detect crashed ANTs via `JoinHandle::is_finished()` in a monitor loop
   running at least every 2 seconds.
6. Apply exponential backoff restart with the formula
   `min(base * 2^(attempt-1), 300)` and respect `max_restarts`.
7. Re-read ANT configuration from disk on every restart.
8. Process reload signals by pruning finished tasks, re-discovering ANTs,
   spawning new ANTs, and restarting ANTs with changed configuration.
9. Maintain a BotRegistry with at least the fields specified in
   Section 7.2 and support all operations listed in Section 7.4.
10. Broadcast all WsEvent variants listed in Section 8.2 with the
    specified tag names and fields.
11. Spawn a maintenance loop per ANT with consolidation, cross-linking,
    and rumination as described in Section 9.
12. Broadcast `BotStatus` events on ANT start, stop, and restart.

### 10.2 RECOMMENDED

A conforming implementation SHOULD:

1. Provide a history recorder that persists `Message` and `UserMessage`
   events for conversation replay.
2. Support case-insensitive ANT lookup in `send_message` and `ask_ant`.
3. Log lag warnings when broadcast subscribers fall behind.
4. Support encrypted backups when `encrypt_backups` is enabled.

### 10.3 OPTIONAL

A conforming implementation MAY:

1. Support relay configuration for distributed multi-engine deployments
   (see ANTHILL-FEDERATION).
2. Implement additional `WsEvent` variants beyond those specified.
3. Provide a global AI backend registry derived from the first ANT's
   configuration for use by the web dashboard.

---

## 11. Security Considerations

- The supervisor MUST NOT expose the `request_tx` channels or
  `BotRegistry` internals over an unauthenticated interface. Access
  control is governed by ANTHILL-TRUST.
- The `colony.query` mechanism (Section 4.2, step 6) uses `chat_id = -2`
  to distinguish colony-internal queries from user messages. A conforming
  implementation MUST NOT allow external clients to inject messages with
  negative chat IDs.
- Backup encryption (when enabled) MUST use a colony-wide key stored at
  `~/.config/anthill/colony.key`. The key MUST NOT be committed to the
  ANT's Git repository.

---

## 12. References

- RFC 2119. Bradner, S. "Key words for use in RFCs to Indicate Requirement
  Levels." IETF, 1997.
- ANTHILL-SENTANT -- ANT sentant model, conductor FSM, plugin contract.
- ANTHILL-TRUST -- Trust groups, capability tokens, colony security.
- ANTHILL-WORKER -- AI worker subprocess lifecycle and backend dispatch.
- ANTHILL-FEDERATION -- Cross-node colony mesh and relay protocol.
- ANTHILL-RUMINATE -- Autonomous thinking triggers and depth control.
