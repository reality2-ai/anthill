# ANTHILL-WORKER: AI Worker Lifecycle

**Version:** 0.2.0
**Date:** 2026-03-20
**Status:** Draft
**Depends on:** ANTHILL-INTRO, ANTHILL-COLONY, ANTHILL-MEMORY

---

## 1. Introduction

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT", "SHOULD", "SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this document are to be interpreted as described in RFC 2119.

This specification defines how ANTS dispatch work to AI backends, monitor worker processes, handle failures, and manage concurrent tasks.

### 1.1 Terminology

| Term | Definition |
|------|-----------|
| **Worker** | A spawned AI backend process (e.g. `claude -p`, `codex exec`) |
| **Task** | A single user request being processed by a worker |
| **Backend** | An AI CLI tool that can be invoked (Claude, Codex, Gemini, Ollama) |
| **Follow-up** | A message queued to run after a task completes, with session continuity |
| **Watchdog** | A per-task monitor that detects stalls and enforces timeouts |

---

## 2. Worker Lifecycle

### 2.1 Request → Task → Worker

1. User sends a message (via web, Telegram, or Slack).
2. The input plugin stores the message text in the shared message queue (data plane).
3. The input plugin emits a `RELAY_COMMAND` event with command type and chat ID.
4. The conductor sentant classifies the command and emits a plugin call to the AI plugin.
5. The AI plugin reads the message from the queue and sends a `CliRequest` to the worker loop.
6. The worker loop assigns a task ID and spawns a tokio task.
7. The tokio task spawns the AI backend process.

### 2.2 Process Configuration

The spawned process MUST be configured as follows:

| Setting | Value | Rationale |
|---------|-------|-----------|
| `stdin` | `Stdio::null()` | Prevent hanging on permission prompts |
| `stdout` | `Stdio::piped()` | Stream progress via JSON |
| `stderr` | `Stdio::piped()` | Capture error output |
| `kill_on_drop` | `true` | Clean up on task abort |
| `process_group` | `0` (Unix) | Kill entire process tree on cancel |
| `current_dir` | ANT working directory | Consistent file operations |

### 2.3 Concurrency

Multiple tasks MAY run concurrently for the same ANT. Each task is an independent tokio task with its own spawned process.

---

## 3. Multi-Backend Support

### 3.1 Backend Detection

Implementations SHOULD detect installed backends at startup by checking for their CLI executables: `claude`, `codex`, `gemini`, `ollama`. The `anthill --doctor` command reports which backends are detected.

### 3.2 Backend Priority and Fallback

The `backends` configuration field lists backends in priority order. For each request:

1. Try the first backend.
2. If it fails with a retriable error (rate limit, quota, billing, timeout, API error), try the next.
3. If all backends fail, return an error message to the user.
4. Broadcast a `TaskError` event on all-backend failure.

### 3.3 Retriable Errors

An error is retriable if the response text (stdout or stderr) contains any of: "rate limit", "quota", "insufficient", "billing", "credits", "exceeded", "overloaded", "capacity", "timeout", "api error" (case-insensitive).

### 3.4 Backend Command Building

Each backend has a specific CLI invocation:

| Backend | Command | Key Flags |
|---------|---------|-----------|
| `claude` | `claude -p --verbose --output-format stream-json` | `--dangerously-skip-permissions`, `-c` (continue), `--append-system-prompt` |
| `codex` | `codex exec --json` | — |
| `ollama:<model>` | `ollama run <model>` | `--nowordwrap`, model specified via `ollama:` prefix in backends config |
| `gemini` | Reserved | Not yet implemented |

### 3.5 Progress Parsing

Each backend's JSON output is parsed for progress events:

**Claude (`stream-json`):**
- `type: "assistant"` with `tool_use` blocks → tool progress (Bash, Read, Edit, Agent, etc.)
- `type: "assistant"` with `text` blocks → partial text result
- `type: "result"` → final result text
- `permission_denials` array → appended to result

**Codex (`json`):**
- `type: "item.started"` / `type: "item.completed"` with `command_execution` → tool progress
- `type: "item.completed"` with `agent_message` → result text

**Ollama (plain text):**
- Output is plain text (not JSON). Lines are accumulated as the result text.
- No structured progress events — progress is inferred from stdout activity (watchdog).

---

## 4. Worker Supervision

### 4.1 Watchdog

Each spawned worker MUST have a watchdog task that monitors activity:

1. **Activity tracking**: the stdout reader updates a shared `last_activity` timestamp on every line received.
2. **Stall warning**: if no activity for 120 seconds, broadcast a `TaskProgress` event with `kind: "warning"`.
3. **Hard timeout**: if no activity for `worker_timeout_secs` (default 600), kill the process group via `SIGKILL`.
4. **Stall recovery**: if activity resumes after a warning, the warning state resets.

### 4.2 Stderr Capture

Stderr MUST be read concurrently with stdout (via `tokio::join!`) to prevent pipe deadlock. Stderr content is capped at 4096 bytes and included in error messages.

### 4.3 Process Killing

On cancel or timeout:

1. The tokio task is aborted.
2. `kill_on_drop` sends SIGKILL to the child process.
3. `process_group(0)` ensures the signal reaches all child processes (sub-agents, shell commands).

---

## 5. Follow-Up Queue

### 5.1 Purpose

When a user sends a message while a task is running, they may want it to apply as context for the current work rather than starting a new concurrent task.

### 5.2 Mechanism

1. User sends `/followup <text>` (Telegram/Slack) or uses the follow-up input in the web UI.
2. The message is queued in a `FollowUpQueue` keyed by task ID.
3. When the task completes, queued follow-ups are dispatched as new requests with `new_session: false` (session continuity via `-c` flag).
4. The AI sees the follow-up as a continuation of the same conversation.

### 5.3 Auto-Followup

When exactly one task is running for an ANT, any new message sent to that ANT is automatically queued as a follow-up rather than starting a concurrent task. This applies to all interfaces (web, Telegram, Slack).

- The user does not need to use `/followup` explicitly.
- If multiple tasks are running, the message starts a new concurrent task (existing behaviour).
- Auto-followup preserves session continuity: the queued message runs with `-c` when the current task completes.

### 5.4 Interrupt with `!`

A message prefixed with `!` cancels the running task and restarts with the combined context of the original prompt and the new message.

1. The user sends `! <new instruction>`.
2. The running task is cancelled (SIGKILL to the process group).
3. A new task is dispatched with both the original prompt and the interrupt message, preserving session continuity.
4. This is equivalent to cancel + re-send with additional context, but as a single atomic action.

### 5.5 Question Relay

When a worker's AI backend uses the `AskUserQuestion` tool:

1. The question is detected in the `stream-json` output.
2. A `TaskProgress` event with `kind: "question"` is broadcast.
3. On Telegram/Slack: the question is sent to the chat with instructions to use `/followup <answer>`.
4. On web: the worker card highlights in purple with "Answer the question above..." placeholder.
5. Since `stdin` is null, the AI handles EOF and continues. The user's answer arrives as a follow-up in the next session turn.

---

## 6. Commands

### 6.1 User Commands

| Command | Type ID | Description |
|---------|---------|-------------|
| (any text) | 0 | Dispatch as AI prompt |
| `/help` | 1 | Show command help |
| `/ants` | 2 | List running workers |
| `/usage` | 3 | Show session statistics |
| `/cancel [id]` | 4 | Cancel a task (most recent, or by ID) |
| `/cancel all` | 5 | Cancel all tasks |
| `/new` | 6 | Start fresh conversation |
| `/status` | 7 | Live worker status (backend, progress, follow-ups) |
| `/followup <text>` | 8 | Queue follow-up for running task |

### 6.2 Command Classification

Commands are classified by the input plugin (Telegram, Slack, Web) at receive time. The command type (0-8) is encoded in the event payload's first byte. The conductor sentant routes each type to the appropriate plugin command.

### 6.3 Web Command Routing

Slash commands (`/help`, `/status`, `/usage`, `/ants`, `/cancel`) now work from the web UI, not just Telegram and Slack. The web frontend detects slash-prefixed messages and routes them through the same command classification pipeline. Responses are delivered as system messages in the chat view.

### 6.4 UTF-8 Safety

All string slicing (message truncation, preview extraction, chunk boundaries) MUST use character or word boundaries, never byte offsets. This prevents panics on multi-byte characters such as Māori macrons (e.g. ā, ē, ī, ō, ū), emoji, or CJK characters.

---

## 7. Events

### 7.1 Bus Events

| Event | Hash | Payload |
|-------|------|---------|
| `relay.command` | FNV1a | `{0: cmd_type, 1: chat_id, 2: cancel_task_id}` |
| `relay.ai_ready` | FNV1a | `{0: kind, 1: chat_id}` |

### 7.2 WebSocket Events

| Event | Direction | Description |
|-------|-----------|-------------|
| `task_started` | server→client | New task spawned |
| `task_progress` | server→client | Tool use, stall warning, question |
| `task_completed` | server→client | Task finished |
| `task_error` | server→client | All backends failed |
| `message` | server→client | AI response text |
| `user_message` | server→client | User message (for sync) |
| `typing` | server→client | Typing indicator |

---

## 8. Sensitive Operation Restriction

### 8.1 Restricted Commands

The following commands MUST be blocked when received from Telegram or Slack:

| Command | Reason |
|---------|--------|
| `/analyse <file>` | Reads and analyses workspace files |
| `/specify <file>` | Reads source code, generates specifications |
| `/test-vectors <file>` | Reads source code or specs, generates test cases |

These commands operate on workspace files and produce structured output best reviewed in the web UI. Telegram and Slack lack the trust group authentication that the web dashboard provides.

### 8.2 Behaviour

When a restricted command is received from Telegram or Slack, the conductor MUST:

1. NOT dispatch the command to the AI backend.
2. Reply with a message explaining that this command is only available from the web dashboard.
3. Include the web dashboard URL in the reply if known.

---

## 9. System Prompt Architecture

The system prompt sent to the AI backend is composed of:

1. **Custom personality** (`system_prompt` from config) — if set.
2. **Workspace preamble** — working directory structure, repos convention.
3. **Memory preamble** — instructions for maintaining the knowledge graph (Popperian model), episodic memory, and user memory.
4. **[KNOWLEDGE GRAPH]** — rendered context from the Popperian knowledge graph (ANTHILL-MEMORY §4.4).
5. **[EPISODES]** — relevant conversation summaries (ANTHILL-MEMORY §5).
6. **[USER MEMORY]** — per-user freeform notes.

Total prompt size SHOULD be monitored. Each section is capped (knowledge graph: 4096 chars, episodes: 2048 chars, user memory: 4096 chars).
