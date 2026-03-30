# ANTHILL-WORKER: AI Worker Supervision and Backend Abstraction

| Field      | Value                                                        |
|------------|--------------------------------------------------------------|
| Version    | 0.1 Draft                                                    |
| Date       | 2026-03-30                                                   |
| Status     | Draft                                                        |
| Depends on | ANTHILL-SENTANT, R2-PLUGIN                                   |
| Related    | ANTHILL-KNOWLEDGE, ANTHILL-RUMINATION, ANTHILL-COLONY        |

> The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT",
> "SHOULD", "SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this
> document are to be interpreted as described in RFC 2119.

---

## 1. Introduction

AI workers execute prompts via external AI backend processes. Each worker
is a supervised subprocess with watchdog monitoring, stall detection, and
timeout killing. Multiple backends are supported through a pluggable
registry with automatic fallback when a backend fails.

An ANT's worker loop runs as a long-lived tokio task that receives
requests over an unbounded MPSC channel and spawns a concurrent tokio
task for each request. Multiple requests MAY be in flight simultaneously.

### 1.1 Terminology

**Worker** -- A tokio task that manages a single AI backend invocation
from request to response.

**Backend** -- An external AI process or API endpoint that executes
prompts. Backends implement the `AiBackend` trait.

**Task** -- A tracked unit of work identified by a `task_id` (`u32`).
A task maps one-to-one with a worker invocation and is registered in a
shared `TaskMap`.

**Follow-up** -- A message queued while a task is running and dispatched
with session continuity after the task completes.

**System Prompt** -- A structured prompt assembled from fixed preambles,
knowledge graph context, episodic memory, user memory, and a
self-evolved thinking process file, subject to a 16 KB budget.

**Stall Detection** -- A watchdog that monitors stdout activity and
kills unresponsive workers after a configurable timeout.

**Watchdog** -- A per-worker tokio task that polls the last-activity
timestamp every 15 seconds and emits warnings or kills the process
group when idle thresholds are exceeded.

**Process Group** -- On Unix, each CLI subprocess is spawned with
`process_group(0)` so that the entire process tree can be killed with a
single `SIGKILL` to the group.

**Backend Registry** -- A central index of all available backends, keyed
by ID and indexed by category. Constructed once at startup and shared
across all ANT workers.

**Engine Category** -- A user-facing classification that maps to an
ordered list of concrete backends. Categories are: `cost_effective`,
`intellectual`, `fast`, `local`, `balanced`, and `specialized:<domain>`.

---

## 2. Backend Abstraction

### 2.1 Supported Backends

A conforming implementation MUST support the following backend kinds:

| Kind       | ID            | Display Name          | Binary     | Type   |
|------------|---------------|-----------------------|------------|--------|
| Claude     | `claude-cli`  | Claude Code (CLI)     | `claude`   | CLI    |
| Codex      | `codex-cli`   | OpenAI Codex (CLI)    | `codex`    | CLI    |
| Gemini     | `gemini-cli`  | Google Gemini (CLI)   | `gemini`   | CLI    |
| Ollama     | `ollama`      | Ollama (local)        | `ollama`   | HTTP   |
| OpenCode   | `opencode`    | OpenCode              | `opencode` | CLI    |
| Grok       | `grok`        | Grok                  | `grok`     | CLI    |
| DeepSeek   | `deepseek`    | DeepSeek              | `deepseek` | CLI    |
| LM Studio  | `lmstudio`    | LM Studio             | `lms`*     | HTTP   |

\* LM Studio detection checks multiple binary names: `lms`, `llmster`,
`lmstudio`, and `~/.lmstudio/bin/lms`.

An implementation MAY support additional backends by registering them in
the `BackendRegistry` via the `[ai.backends_config]` TOML section.
Configurable backend types include: `cli`, `openai`, `anthropic`,
`ollama`, `openai-compatible`, and `groq`.

### 2.2 Backend Detection

Each CLI backend is detected by running `which <command>`. A backend is
considered available if and only if the binary exists on `PATH` and the
`which` invocation exits with status 0.

LM Studio is a special case: the implementation MUST check `lms`,
`llmster`, `lmstudio`, and `$HOME/.lmstudio/bin/lms` before declaring
the backend unavailable.

HTTP-based backends (Ollama, LM Studio, API backends) MUST implement an
`is_available()` check that verifies the service is reachable.

### 2.3 Backend Selection Strategy

#### 2.3.1 Registry Resolution

When a `BackendRegistry` is present and an `[ai]` config section exists,
backends are resolved through the registry. The resolution algorithm is:

1. If `[ai]` config provides a `default_category` or explicit `backends`
   list, resolve through the registry using `resolve(selector)`.
2. If `[ai]` config exists but resolves to an empty list, fall back to
   mapping legacy `[claude].backends` names through the registry.
3. If no `[ai]` config exists but an explicit `[claude].backends` list
   is present, map those names through the registry.
4. If neither config source yields backends, use all registered backends.

#### 2.3.2 Category-Based Sorting

The `resolve()` method accepts a selector that MAY be:

- A category name (`"intellectual"`, `"fast"`, `"local"`, etc.)
- A backend ID (`"claude-cli"`, `"openai-gpt4o"`)
- A comma-separated fallback list (`"claude-cli,openai-gpt4o"`)

Categories are tried first. If no backends match the category, the
selector is interpreted as a comma-separated ID list.

#### 2.3.3 Sort Order by Category

Each backend declares `EngineTags` with `cost_tier`, `speed_tier`, and
`quality_tier` (each 1--5). Sort order depends on the selected category:

| Category                     | Primary Sort           | Secondary Sort          | Local Backends |
|------------------------------|------------------------|-------------------------|----------------|
| `cost_effective`, `local`    | cost ascending         | quality descending      | In-place       |
| `fast`                       | speed descending       | quality descending      | Moved to end   |
| All others (incl. `balanced`)| quality descending     | cost ascending          | Moved to end   |

Local backends (`ollama`, `lmstudio`) are always moved to the end of the
fallback list unless the category is `local` or `cost_effective`.

#### 2.3.4 Engine Categories

The following categories MUST be recognised (case-insensitive, with
aliases):

| Category        | Aliases                                    |
|-----------------|--------------------------------------------|
| `cost_effective`| `cost-effective`, `cheap`                  |
| `intellectual`  | `smart`, `reasoning`, `best`               |
| `fast`          | `quick`, `speed`                           |
| `local`         | `private`, `on-premise`                    |
| `balanced`      | `default`                                  |
| `specialized:X` | (where X is a domain string, e.g. `coding`)|

### 2.4 Command Construction

#### 2.4.1 Claude CLI

```
claude -p --verbose --output-format stream-json \
  [--dangerously-skip-permissions]              \
  [--add-dir <working_dir>]                     \
  [-c]                                          \
  --append-system-prompt <system_prompt>        \
  <message>
```

- `--dangerously-skip-permissions` is included when `skip_permissions`
  is true in the ANT config.
- `--add-dir` is included when `working_dir` is non-empty.
- `-c` is included when `continue_session` is true (continuing an
  existing conversation).

#### 2.4.2 Codex CLI

```
codex exec --json <message>
```

Codex does not receive the system prompt via CLI flags. The system
prompt MUST be prepended to the message or handled by the backend
implementation.

#### 2.4.3 Gemini CLI

```
gemini -p <combined_prompt> --output-format stream-json --yolo
```

The system prompt and user message are concatenated into a single
`combined_prompt`. The `--yolo` flag auto-approves tool calls (analogous
to Claude's `--dangerously-skip-permissions`).

#### 2.4.4 Ollama

Ollama is invoked via its HTTP API, not as a CLI subprocess. The
implementation MUST use the Ollama chat completions endpoint with the
system prompt and user message as separate message roles.

#### 2.4.5 API Backends

Backends configured with type `openai`, `anthropic`, `openai-compatible`,
or `groq` in `[ai.backends_config]` are invoked via HTTP API. Each
`BackendConfig` specifies:

- `model` -- Model name/ID sent to the API.
- `api_base` -- Base URL for the API endpoint.
- `api_key_env` -- Environment variable holding the API key (preferred).
- `api_key` -- Direct API key (use `api_key_env` for security).
- `max_tokens` -- Maximum response tokens (OPTIONAL).
- `temperature` -- Sampling temperature 0.0--2.0 (OPTIONAL).

### 2.5 Engine Metadata

Each backend declares an `EngineTags` structure:

```
EngineTags {
    categories: Vec<EngineCategory>,    // which categories this backend serves
    capabilities: Vec<String>,          // "code", "vision", "function-calling", "file-access"
    cost_tier: u8,                      // 1 (cheapest) -- 5 (most expensive)
    speed_tier: u8,                     // 1 (fastest) -- 5 (slowest)
    quality_tier: u8,                   // 1 (basic) -- 5 (best)
}
```

Default tiers for built-in CLI backends:

| Backend      | Categories                              | Cost | Speed | Quality |
|--------------|-----------------------------------------|------|-------|---------|
| `claude-cli` | intellectual, balanced, specialized:coding | 4  | 3     | 5       |
| `codex-cli`  | balanced, specialized:coding            | 3    | 3     | 4       |
| `gemini-cli` | balanced, fast                          | 3    | 4     | 4       |

---

## 3. Worker Lifecycle

### 3.1 Task Creation

1. A request arrives on the worker's MPSC channel as a `CliRequest`
   containing: `chat_id` (i64), `message` (String), `new_session`
   (bool), `task_id` (u32), and `source` (String).
2. The `task_id` is assigned by the caller before enqueueing.
3. The task is registered in the shared `TaskMap` as a `RunningTask`
   with state `Running`.
4. A `TaskStarted` WebSocket event is broadcast.
5. The worker is spawned as a concurrent tokio task.

Sources include: `"telegram"`, `"slack"`, `"web"`, `"rumination"`, and
`"colony:<ant_name>:<chat_id>"`.

### 3.2 Process Execution

Each CLI backend subprocess MUST be spawned with the following
properties:

- **Working directory**: Set to the ANT's `working_dir`.
- **stdin**: `Stdio::null()` -- the subprocess MUST NOT block waiting
  for input.
- **stdout**: `Stdio::piped()` -- streamed line-by-line and parsed as
  stream-JSON for progress events.
- **stderr**: `Stdio::piped()` -- captured concurrently in a separate
  tokio task, limited to 4096 bytes.
- **kill_on_drop**: `true` -- if the tokio task is dropped, the child
  process is killed.
- **process_group(0)**: On Unix, the subprocess is placed in its own
  process group so that `SIGKILL` can be sent to the entire tree on
  cancel.

Stdout and stderr MUST be read concurrently (separate tokio tasks) to
avoid deadlock.

### 3.3 Stall Detection (Watchdog)

A per-worker watchdog task MUST be spawned alongside every CLI backend
invocation. The watchdog:

1. Polls the last-activity timestamp every 15 seconds.
2. If no stdout output has been received for 120 seconds (2 minutes),
   emits a `TaskProgress` event with kind `"warning"` and detail
   indicating the idle duration.
3. If the warning clears (activity resumes), the warning state resets.
4. If no stdout output has been received for `worker_timeout_secs`
   (default: 600 seconds / 10 minutes), the entire process group is
   killed via `SIGKILL`:
   ```
   killpg(child_pid as i32, SIGKILL)
   ```
5. The `worker_timeout_secs` value is configurable per ANT via the
   `[claude]` section in `ant.toml`. A value of 0 means no timeout.

The watchdog MUST be aborted when the subprocess completes (success or
failure).

### 3.4 Task State Machine

A task progresses through the following states:

```
Running --> Completed    (success)
Running --> Failed       (backend error, timeout, all backends exhausted)
Running --> Cancelled    (user /cancel or !interrupt)
```

The `TaskState` enum is:

- `Running` -- actively producing output.
- `Completed` -- finished successfully.
- `Failed(String)` -- failed with an error message.
- `Cancelled` -- killed by user request.

### 3.5 Task Completion

On completion (any terminal state):

1. The task is removed from the `TaskMap`.
2. A `TaskCompleted` WebSocket event is broadcast with `duration_secs`.
3. The response text is broadcast:
   - To the WebSocket event bus as a `Message` event.
   - To the Telegram channel (if applicable and sync is enabled).
   - To the R2 event bus via the response queue.
4. Per-user statistics are updated (message count, input/output chars).
5. The follow-up queue for this `task_id` is drained (see Section 4).

On failure when all backends are exhausted, the response text MUST be:
```
All <N> backend(s) failed:

* <backend_id>: <error_message>
* ...

Try /model to see which backends are available.
```

### 3.6 Task Cancellation

When a task is cancelled (via `/cancel` or `!interrupt`):

1. The process group is killed via `SIGKILL` on Unix.
2. The task state is set to `Cancelled`.
3. The task is removed from the `TaskMap`.

---

## 4. Follow-up Queue

### 4.1 Queueing

Messages sent while a task is running for the same session MUST be
queued as follow-ups rather than spawning a new worker. Follow-ups are
stored in a per-task queue keyed by `task_id`:

```
FollowUpQueue = Arc<Mutex<HashMap<u32, Vec<FollowUp>>>>
```

Each `FollowUp` contains: `chat_id`, `message`, and `source`.

### 4.2 Dispatch

When a task completes, the implementation MUST:

1. Remove the follow-up list for the completed `task_id`.
2. For each queued follow-up, re-enqueue it as a new `CliRequest` with
   `new_session: false` (to maintain session continuity via the `-c`
   flag).
3. Follow-ups are dispatched in FIFO order.

### 4.3 Rumination Follow-ups

When a rumination task completes (source `"rumination"` and
`new_session` was true), a single automatic follow-up is generated
based on the response content:

- If the response contains contradictions or inconsistencies, the
  follow-up investigates downstream effects.
- If the response contains refutation results, the follow-up explores
  new connections from strengthened beliefs.
- If the response contains undetermined connections, the follow-up
  searches for missing links.
- Otherwise, a generic continuation follow-up is generated.

Follow-ups are only spawned from top-level rumination tasks (where
`new_session` is true), not from follow-ups of follow-ups, to prevent
infinite chains.

---

## 5. Multi-Backend Fallback

### 5.1 Fallback Algorithm

Backends are tried in the order determined by the registry resolution
(Section 2.3). For each backend:

1. Set the live backend indicator.
2. Create an unbounded progress channel.
3. Spawn a progress relay task that forwards `AiProgress` events to
   the WebSocket broadcast channel.
4. Call `backend.execute(request, progress_tx)`.
5. Abort the progress relay.
6. On success: capture `response_text` and `used_backend`, break.
7. On failure:
   a. Capture the error in `all_errors`.
   b. If `err.retriable` is true AND more backends remain, broadcast
      a `TaskProgress` event with kind `"fallback"` and try the next.
   c. If `err.retriable` is false OR no backends remain, compose the
      "All backends failed" error message.

### 5.2 Error Classification

Errors from backends are classified to determine retriability:

| Classification            | Retriable | Examples                                   |
|---------------------------|-----------|--------------------------------------------|
| Context length exceeded   | No        | "context length exceeded", "too many tokens"|
| Invalid request           | No        | "invalid request", "invalid api"           |
| Authentication failure    | No        | "authentication", "unauthorized"           |
| Rate limit / overload     | Yes       | "rate limit", "overloaded", "503"          |
| Timeout                   | Yes       | "timeout", "timed out"                     |
| Billing / quota           | Yes       | "insufficient", "billing", "quota"         |
| Network error             | Yes       | Connection refused, DNS failure            |
| Unknown                   | No        | Default for unrecognised errors            |

---

## 6. Session Continuity

### 6.1 Session Rules

| Condition                          | Session Behaviour          |
|------------------------------------|----------------------------|
| First message in a conversation    | New session (no `-c` flag) |
| Subsequent messages                | Continue session (`-c`)    |
| `/new` command                     | Force new session          |
| Colony messages                    | Always new session         |
| Rumination                         | Always new session         |
| Follow-up after task completes     | Continue session (`-c`)    |

### 6.2 Backend Session Tracking

A `BackendSessions` structure tracks per-chat state:

- `last_backend`: which backend ID was used for each `chat_id`.
- `summaries`: a truncated summary (max 500 chars) of the last response
  per `chat_id`, stored for context injection when switching backends.

When a response is received, the implementation MUST record the backend
used and store a response summary.

---

## 7. System Prompt Construction

### 7.1 Budget

The total system prompt MUST NOT exceed `MAX_SYSTEM_PROMPT` = 16,384
bytes. If the assembled prompt exceeds this limit, dynamic sections are
truncated. A warning MUST be logged when the budget is exceeded.

### 7.2 Assembly Order

The system prompt is assembled in the following order. Sections marked
"fixed" are always included; sections marked "dynamic" are subject to
budget truncation.

| Priority | Section                     | Type    | Budget Allocation |
|----------|-----------------------------|---------|-------------------|
| 1        | File access restriction      | Fixed   | --                |
| 2        | Graph access restriction     | Fixed   | --                |
| 3        | Custom personality prompt    | Fixed   | --                |
| 4        | Workspace preamble           | Fixed   | --                |
| 5        | Memory preamble              | Fixed   | ~4 KB             |
| 6        | Methodology preamble         | Conditional | ~1 KB (analytical commands only) |
| 7        | Thinking process             | Dynamic | max 2,048 bytes   |
| 8        | Knowledge graph context      | Dynamic | 70% of remaining  |
| 9        | Episodic memory              | Dynamic | 15% of remaining  |
| 10       | User memory                  | Dynamic | 15% of remaining  |
| 11       | Colony directory             | Fixed   | --                |

### 7.3 Fixed Sections

**File access restriction** (when `allow_base_code_changes` is false):
The AI MUST NOT create, edit, or delete files outside the working
directory. It MAY read files anywhere.

**Graph access restriction**: The AI MUST NOT directly read or modify
knowledge graph files (`memory/knowledge.json`, `memory/graphs/*.json`,
`*.cbor`). All graph operations MUST go through MCP tools:
`graph_add_node`, `graph_add_edge`, `graph_add_citation`,
`graph_update_evidence`, `graph_strengthen`, `graph_weaken`,
`graph_contradict`, `graph_query_about`, `graph_query_uncertain`,
`graph_list_nodes`.

**Memory preamble** (~4 KB): Covers knowledge graph format, Thurisaz
Bayesian updating rules, evidence types with Bayes factors, decay
categories, episodic memory format, graph organisation rules,
self-modification rules, colony communication protocol, citation rules,
and anti-confirmation-bias directives.

**Methodology preamble** (~1 KB): Included only for analytical commands
(`/analyse`, `/reflect`, `/compact-chat`, `/specify`, `/test-vectors`).
Covers thematic analysis methodology (Braun & Clarke, 2022) with
Thurisaz-compliant integration.

**Workspace preamble**: Describes the working directory structure
(`memory/`, `repos/`) and git-as-a-reasoning-tool instructions.

**Colony directory**: A pre-populated listing of peer ANTs and their
topic graphs, generated by scanning the ants directory.

### 7.4 Dynamic Sections

**Knowledge graph context**: Rendered via semantic search (Ollama
embeddings) when Ollama is available, falling back to TF-IDF keyword
matching. Budget: 70% of remaining space after fixed sections.

**Episodic memory**: Recent conversation summaries loaded from
`episodes.json` and filtered by relevance to the current message (top 5
matches). Budget: 15% of remaining space.

**User memory**: Per-user freeform notes from `<chat_id>.md` (or
`rumination.md` / `colony.md` for those sources). Budget: 15% of
remaining space.

**Thinking process**: Self-evolved methodology from
`memory/thinking_process.md`. Hard cap: 2,048 bytes.

### 7.5 Confidence Thresholds

Edges below `MIN_PROMPT_CONFIDENCE` (0.15) MUST be excluded from the
knowledge graph context rendered into the system prompt. These edges are
retained in the graph but hidden from the AI.

High-confidence edges (>= 0.80) are in the "Established" trust tier and
SHOULD be presented without uncertainty qualifiers.

### 7.6 Graduated Trust Tiers

| Tier         | Confidence | Treatment in Prompt                      |
|--------------|------------|------------------------------------------|
| Established  | >= 0.80    | Reliable. Build on confidently.          |
| Likely       | >= 0.60    | Probably true. Note uncertainty.         |
| Possible     | >= 0.40    | Could go either way. Flag when used.     |
| Uncertain    | >= 0.20    | Weak. Requires caveats.                  |
| Doubtful     | < 0.20     | Likely wrong. Consider contradicting.    |

---

## 8. Progress Events

### 8.1 WebSocket Event Types

The following events MUST be broadcast over the WebSocket event bus
during worker execution:

#### TaskStarted
```json
{
    "type": "task_started",
    "bot": "<ant_name>",
    "task_id": 42,
    "preview": "What is the capital..."
}
```
Emitted when a task is spawned. `preview` is the message truncated to
50 characters.

#### TaskProgress
```json
{
    "type": "task_progress",
    "bot": "<ant_name>",
    "task_id": 42,
    "kind": "tool_use",
    "detail": "Reading: src/main.rs"
}
```
Emitted during execution. The `kind` field MUST be one of:

| Kind           | Meaning                                           |
|----------------|---------------------------------------------------|
| `thinking`     | Backend is in a reasoning/thought phase           |
| `tool_use`     | Backend invoked a tool (Read, Edit, Bash, etc.)   |
| `tool_result`  | A tool call returned results                      |
| `question`     | Backend is asking for user input                  |
| `warning`      | Stall warning or other non-fatal issue            |
| `fallback`     | Primary backend failed, trying next               |

The `detail` field provides a human-readable description. For tool_use
events, the detail SHOULD follow these patterns:
- `"Reading: <path>"` for file reads
- `"Editing: <path>"` for file edits
- `"Writing: <path>"` for file writes
- `"Running: <command>"` for shell commands (truncated to 60 chars)
- `"Searching: <pattern>"` for glob operations
- `"Grep: <pattern>"` for content searches
- `"Tool: <name>"` for other tools
- `"Using: <name>"` for Gemini tool calls

#### TaskCompleted
```json
{
    "type": "task_completed",
    "bot": "<ant_name>",
    "task_id": 42,
    "duration_secs": 15
}
```
Emitted when a task finishes (success or failure after all retries).

#### TaskError
```json
{
    "type": "task_error",
    "bot": "<ant_name>",
    "task_id": 42,
    "error": "Rate limited by backend"
}
```
Emitted on task failure.

#### Message
```json
{
    "type": "message",
    "bot": "<ant_name>",
    "chat_id": 12345,
    "text": "Here is the response...",
    "task_id": 42
}
```
Emitted when the AI produces a response. For rumination tasks with
negative `chat_id`, the broadcast `chat_id` MUST be set to 0 so the
message appears as a system message.

#### UserMessage
```json
{
    "type": "user_message",
    "bot": "<ant_name>",
    "chat_id": 12345,
    "text": "Hello, what do you think?",
    "source": "web"
}
```
Emitted when a user sends a message (for history and cross-channel
sync). MUST NOT be emitted for rumination messages.

#### GraphUpdated
```json
{
    "type": "graph_updated",
    "bot": "<ant_name>",
    "graph": "all",
    "source": "rumination"
}
```
Emitted after rumination tasks complete to trigger live graph refresh
in the dashboard.

### 8.2 Typing Indicator

While a task is running, a typing indicator MUST be sent to Telegram
every 4 seconds (an empty string message to the Telegram sender).

---

## 9. Cross-Channel Synchronisation

When `sync_channels` is true in the ANT config:

- User messages from non-Telegram sources MUST be forwarded to the
  Telegram channel with a source label prefix (e.g. `[web]`, `[slack]`).
- AI responses from non-Telegram sources MUST be forwarded to the
  Telegram channel.
- Source chat IDs are tracked per source (`"telegram"`, `"slack"`,
  `"web"`) so that forwarding targets the correct Telegram chat.

---

## 10. Periodic Maintenance

The worker loop MUST perform maintenance tasks at the following
intervals:

| Interval          | Action                                            |
|-------------------|---------------------------------------------------|
| Every 50 requests | Consolidate all graphs (dedup, link orphans, backfill Thurisaz fields) |
| Every 100 requests| Archive stale low-confidence edges                |
| Every 24 hours    | Apply confidence decay to all graphs              |

After maintenance, the knowledge graph cache MUST be invalidated so
subsequent prompts reflect the updated state.

---

## 11. Per-User Statistics

The implementation MUST track per-user (`chat_id`) statistics:

- `messages`: Total message count.
- `input_chars`: Total characters received.
- `output_chars`: Total characters sent.
- `started`: Timestamp of the first message in the session.

Statistics MUST NOT be updated for rumination or colony query messages.
The statistics map is shared with the sentant for the `/usage` command.

---

## 12. Stream-JSON Parsing

### 12.1 Claude Stream Format

Claude emits stream-JSON lines with a `type` field:

- `"assistant"` -- Contains the final response in
  `message.content[].text`.
- `"result"` -- Contains the final result in `result` (string) and
  optionally `cost_usd`.
- `"tool_use"` / `"tool_result"` -- Tool invocation events with tool
  name and input parameters.
- `"user_input_request"` -- The backend is waiting for user input;
  `message` contains the question text.

### 12.2 Codex Stream Format

Codex emits `"item.completed"` events with an `item.type` field:

- `"agent_message"` -- Contains the response in `item.text`.
- `"command_execution"` -- A command was run; `item.command` contains
  the shell command.

### 12.3 Gemini Stream Format

Gemini emits events with a `type` field:

- `"result"` -- Final response in `response`.
- `"message"` -- Streaming message chunk in `content`.
- `"tool_use"` -- Tool invocation; `name` identifies the tool.
- `"tool_result"` -- Tool call returned.
- `"thought"` -- Reasoning/thinking phase; `content` contains the
  thought text (truncated to 100 chars for progress events).
- `"error"` -- Error event; `message` contains the error text.
- `"init"` -- Initialisation event.

---

## 13. Conformance

A conforming ANTHILL-WORKER implementation:

1. MUST implement the `AiBackend` trait for all supported backend types
   (Section 2.1).
2. MUST detect backend availability via `which` for CLI backends and
   reachability checks for HTTP backends (Section 2.2).
3. MUST resolve backend selection through the registry when available,
   with correct category-based sorting (Section 2.3).
4. MUST spawn CLI subprocesses with `stdin: null`, `stdout: piped`,
   `stderr: piped`, `kill_on_drop: true`, and `process_group(0)` on
   Unix (Section 3.2).
5. MUST implement the stall detection watchdog with a 120-second warning
   threshold and a configurable hard timeout defaulting to 600 seconds
   (Section 3.3).
6. MUST implement the follow-up queue with FIFO dispatch and session
   continuity (Section 4).
7. MUST attempt all resolved backends in order before reporting failure,
   with correct error classification for retriability (Section 5).
8. MUST assemble the system prompt within the 16 KB budget with the
   specified priority ordering (Section 7).
9. MUST exclude edges below 0.15 confidence from the system prompt
   (Section 7.5).
10. MUST broadcast all specified WebSocket event types during task
    execution (Section 8).
11. MUST perform periodic maintenance (consolidation, archiving, decay)
    at the specified intervals (Section 10).
12. MUST track per-user statistics excluding rumination and colony
    queries (Section 11).
