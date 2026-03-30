# ANTHILL-CHAT: Conversation Model and Command System

| Field      | Value                                                        |
|------------|--------------------------------------------------------------|
| Version    | 0.1 Draft                                                    |
| Date       | 2026-03-30                                                   |
| Status     | Draft                                                        |
| Depends on | ANTHILL-SENTANT, ANTHILL-WORKER                              |
| Related    | ANTHILL-DASHBOARD, ANTHILL-CHANNELS, ANTHILL-WORKERS-UX      |

> The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT",
> "SHOULD", "SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this
> document are to be interpreted as described in RFC 2119.

---

## 1. Introduction

The conversation model defines how users communicate with ANTs across all
channels (web dashboard, Telegram, Slack). It covers message routing, the
slash command system, context continuity, cross-channel synchronisation, and
interaction patterns.

Users interact with ANTs through natural-language messages and structured
slash commands. Messages are classified at the input boundary, routed
through the conductor FSM, and dispatched to either a local handler or an
AI worker subprocess. Responses are streamed back through the originating
channel and, when cross-channel sync is enabled, broadcast to all connected
channels simultaneously.

### 1.1 Scope

This specification covers:

- Message flow from user input to AI response.
- The slash command system: classification, dispatch, help, and
  autocomplete.
- Context continuity: sessions, follow-ups, and interrupts.
- Cross-channel message synchronisation.
- Chat history persistence and cross-device access.
- Reply-to-message (quoting) behaviour.
- Message rendering and scroll behaviour in the web dashboard.

This specification does NOT cover:

- The internal operation of AI workers (see ANTHILL-WORKER).
- The conductor FSM state machine (see ANTHILL-SENTANT).
- Channel-specific adapter protocols (see ANTHILL-TELEGRAM, ANTHILL-SLACK).
- Knowledge graph operations triggered by commands (see ANTHILL-KNOWLEDGE,
  ANTHILL-THURISAZ).

### 1.2 Terminology

| Term               | Definition                                                                                         |
|--------------------|----------------------------------------------------------------------------------------------------|
| Message            | A text payload sent by a user or an ANT through any channel.                                       |
| Command            | A message beginning with `/` that maps to a known operation code.                                  |
| Follow-up          | A message queued to execute with session continuity after the current task completes.               |
| Interrupt          | A message prefixed with `!` that cancels the running task and restarts with combined context.       |
| Session            | A continuous conversation with an AI backend, maintained via the `-c` (continue) flag.              |
| Compaction         | The process of analysing recent conversation, extracting knowledge to the graph, and trimming chat history. |
| Cross-channel Sync | Forwarding of messages between web, Telegram, and Slack when `sync_channels` is enabled.            |
| Chat History       | The persistent JSONL record of messages per ANT, loaded on connect and capped at 500 entries.       |
| Quote/Reply        | Selecting a previous message and prepending it as a markdown blockquote to the new message.         |

---

## 2. Message Flow

### 2.1 Input Path

A message enters the system through one of three channel adapters:

1. **Web dashboard** -- WebSocket JSON message of type `Chat { bot, message,
   chat_id }`.
2. **Telegram** -- Incoming bot message via the Teloxide polling loop.
3. **Slack** -- Socket-mode event via the Slack WebSocket connection.

### 2.2 Classification

Each channel adapter MUST classify the message before emitting an R2 event.
Classification is performed by a `classify_command` function that maps the
message text to a command type byte:

| Command                        | Code | Arguments     | Channels           |
|--------------------------------|------|---------------|--------------------|
| `/help`, `/start`              | 0x01 | --            | All                |
| `/ants`, `/bots`               | 0x02 | --            | All                |
| `/usage`                       | 0x03 | --            | All                |
| `/cancel`                      | 0x04 | [task_id]     | All                |
| `/cancel all`                  | 0x05 | --            | All                |
| `/new`                         | 0x06 | --            | All                |
| `/status`                      | 0x07 | --            | All                |
| `/followup`                    | 0x08 | \<text\>      | All                |
| `/analyse`, `/analyze`         | 0x09 | \<file\>      | Web only (security)|
| `/reflect`                     | 0x0A | --            | All                |
| `/specify`                     | 0x0B | \<file\>      | Web only (security)|
| `/test-vectors`, `/testvectors`| 0x0C | \<file\>      | Web only (security)|
| `/ruminate`                    | 0x0D | --            | All                |
| `/citations`                   | 0x11 | --            | All                |
| (regular message)              | 0x00 | --            | All                |

Unrecognised commands (any `/` prefix not in the table above) MUST be
treated as regular messages (code 0x00) and dispatched to the AI worker.

### 2.3 Event Emission

After classification, the channel adapter:

1. Stores the full message text in the shared message queue (data plane).
   For commands with arguments, the command prefix is stripped so the AI
   plugin receives only the argument content.

2. Emits a compact CBOR event on the R2 event bus:

   ```
   RELAY_COMMAND: { 0: uint(cmd_type), 1: uint(chat_id), 2: uint(cancel_task_id) }
   ```

   This event MUST NOT exceed the 256-byte R2 wire limit. The full message
   text travels on the plugin data plane, never on the event bus.

### 2.4 Conductor Dispatch

The conductor FSM (ANTHILL-SENTANT) receives `RELAY_COMMAND` events and
emits `Action::plugin_call()` to the AI plugin, mapping each command type
to the corresponding plugin command constant:

| Command Type | Plugin Command   | Behaviour                                     |
|--------------|------------------|-----------------------------------------------|
| 0x00         | CMD_DISPATCH     | Pop message from queue, dispatch to AI worker  |
| 0x01         | CMD_HELP         | Return help text                               |
| 0x02         | CMD_ANTS         | Return running task list                       |
| 0x03         | CMD_USAGE        | Return usage statistics                        |
| 0x04         | CMD_CANCEL       | Cancel task by ID (or most recent)             |
| 0x05         | CMD_CANCEL_ALL   | Cancel all running tasks                       |
| 0x06         | CMD_NEW_SESSION  | Start fresh conversation                       |
| 0x07         | CMD_STATUS       | Return live worker status                      |
| 0x08         | CMD_FOLLOWUP     | Queue follow-up for running task               |
| 0x09         | CMD_ANALYSE      | Thematic analysis of a file                    |
| 0x0A         | CMD_REFLECT      | Review and consolidate knowledge graph         |
| 0x0B         | CMD_SPECIFY      | Generate specification from code               |
| 0x0C         | CMD_TEST_VECTORS | Generate test vectors from code                |
| 0x0D         | CMD_RUMINATE     | Trigger rumination cycle                       |
| 0x11         | CMD_CITATIONS    | Run citation consolidation                     |

The conductor is a pure FSM: it performs no I/O, holds no shared state, and
makes no external calls. All data handling is delegated to the AI plugin.

### 2.5 Response Path

When the AI worker completes, it pushes a `CliResponse` to the shared
response queue and emits a `RELAY_AI_READY` event. The conductor receives
this event and issues `CMD_REPLY` to the AI plugin, which pops the response
and sends it to the originating channel.

For Telegram, responses exceeding 4000 characters are split on character
boundaries to avoid corrupting multibyte UTF-8 sequences.

### 2.6 Web Dashboard Command Handling

The web dashboard handles a subset of commands locally (without dispatching
to the AI worker) for immediate responsiveness. These locally handled
commands include: `/help`, `/start`, `/ants`, `/status`, `/usage`,
`/cancel`, `/cancel all`, `/new`, `/questions`, `/ask`, `/export`,
`/report`, `/compact-chat`, and `/doctor`.

If a web message is not handled locally, it is forwarded to the ANT's AI
worker via the `send_message` method on the bot registry.

---

## 3. Slash Command System

### 3.1 Command Classification

Messages starting with `/` are classified into command codes as defined in
Section 2.2. The classification function MUST:

- Trim leading and trailing whitespace before matching.
- Match both `/analyse` and `/analyze` (British and American spelling).
- Match both `/test-vectors` and `/testvectors` (hyphenated and
  concatenated).
- Return code 0x00 for any unrecognised `/` prefix, treating it as a
  regular message.

### 3.2 Sensitive Commands

Commands that read files from the ANT's working directory are classified as
sensitive:

- `/analyse <file>` (0x09)
- `/specify <file>` (0x0B)
- `/test-vectors <file>` (0x0C)

Sensitive commands MUST only be accepted from the `"web"` or `"system"`
source. When a sensitive command arrives from Telegram or Slack, the
implementation MUST return a warning message to the user and MUST NOT
execute the file operation.

### 3.3 AI Backend Commands

The following commands are passed through to the active AI backend when
supported, rather than being handled by the Anthill conductor:

| Command   | Description                        |
|-----------|------------------------------------|
| `/compact`| Condense conversation context      |
| `/cost`   | Show token/cost usage              |
| `/model`  | Show or change the AI model        |
| `/memory` | Manage memory files                |
| `/clear`  | Clear conversation history         |

These commands are dispatched as regular messages (code 0x00) to the AI
worker, which forwards them to the CLI backend.

### 3.4 Command Autocomplete

The web dashboard MUST provide an autocomplete menu when the user types `/`
at the start of the input field. The menu:

- MUST appear when the input starts with `/` and contains no spaces.
- MUST filter the command list as the user continues typing.
- MUST display each command's name and a brief description.
- MUST support keyboard navigation (arrow keys to select, Enter/Tab to
  confirm, Escape to dismiss).
- MUST insert the selected command followed by a space into the input
  field.
- MUST be dismissed when the user clears the `/` prefix or presses Escape.

The canonical command list for autocomplete:

| Command             | Description                                        |
|---------------------|----------------------------------------------------|
| /help               | Show available commands                            |
| /status             | Live view of each worker                           |
| /ants               | Show running workers                               |
| /usage              | Session statistics                                 |
| /cancel             | Cancel a running task                              |
| /cancel all         | Cancel all tasks                                   |
| /followup           | Queue context for running task                     |
| /new                | Fresh conversation                                 |
| /analyse            | Thematic analysis of a file to graph               |
| /reflect            | Review and consolidate knowledge graph             |
| /ruminate           | Trigger a rumination cycle now                     |
| /citations          | Resolve unknown citations and link to topic graphs |
| /ask                | Ask another ANT about a topic                      |
| /export             | Download knowledge graph as shareable HTML         |
| /questions          | Show pending questions from rumination             |
| /specify            | Generate specification from code                   |
| /test-vectors       | Generate test vectors from code                    |
| /doctor             | Check system prerequisites and health              |
| /reprocess-graphs   | Consolidate all graphs and link orphan nodes       |

### 3.5 Mention Autocomplete

The web dashboard MUST also provide an `@mention` autocomplete menu. When
the user types `@` followed by one or more word characters, the dashboard
MUST display a list of available ANTs filtered by the typed prefix. Selecting
an ANT inserts `@Name` into the input. Mentions trigger inter-ANT
communication via the ANT bus (see ANTHILL-COLONY).

---

## 4. Context Continuity

### 4.1 Sessions

A session represents a continuous conversation thread with the AI backend.
Session continuity is maintained by the `-c` (continue) flag passed to the
CLI backend, which instructs it to resume the previous conversation rather
than starting fresh.

A new session is started in the following cases:

- **First message** -- no prior conversation exists.
- **`/new` command** -- the user explicitly requests a fresh conversation.
  The implementation MUST first ask the AI to summarise the conversation,
  then start a new session.
- **Colony messages** -- inter-ANT queries from other ANTs in the colony.
- **Rumination tasks** -- autonomous background thinking tasks dispatched
  by the maintenance daemon.

All other messages MUST continue the existing session.

### 4.2 Auto-Follow-up

When exactly one task is running for the active ANT and the user sends a
new message:

1. The message MUST be queued as a follow-up rather than starting a
   concurrent task.
2. The user MUST see a confirmation message indicating the message was
   queued (e.g., "Queued for after task #N").
3. When the running task completes, queued follow-ups MUST be dispatched
   with session continuity (using the `-c` flag).
4. The user's message MUST appear in the chat immediately (optimistic
   rendering) before the queue confirmation arrives.

When multiple tasks are running, new messages MUST start additional
concurrent tasks rather than queuing as follow-ups.

### 4.3 Interrupt (`!` prefix)

Messages prefixed with `!` trigger an interrupt:

1. The currently running task MUST be cancelled (its subprocess aborted).
2. The original task's message preview MUST be combined with the interrupt
   message in the format:

   ```
   <original message>

   ADDITIONAL CONTEXT (added while you were working):
   <interrupt message with leading ! stripped>
   ```

3. A new task MUST be dispatched with the combined message, maintaining
   session continuity.
4. The user MUST see a confirmation message (e.g., "Interrupted task #N --
   restarting with your addition").

Use case: redirecting an ANT that has gone down the wrong path without
losing the original context.

### 4.4 Compaction

The `/compact-chat` command (triggered via the web dashboard) performs chat
compaction:

1. The recent conversation is sent to the AI backend with instructions to
   analyse the exchange and extract structured knowledge to the knowledge
   graph, including episode/event entries for temporal reasoning.
2. The chat history is trimmed to the last 4 messages.
3. A system message is prepended: "{N} earlier messages compacted to
   knowledge graph."
4. The history file on disk is atomically rewritten with the compacted
   messages.

Purpose: keep the visible conversation context fresh while preserving
knowledge in persistent storage (the knowledge graph and episode log).

---

## 5. Cross-Channel Sync

### 5.1 Configuration

Cross-channel synchronisation is controlled by the `sync_channels` boolean
in the ANT's configuration (`[claude]` section of `ant.toml`). It defaults
to `false`.

### 5.2 Behaviour

When `sync_channels` is `true`:

- Messages from the web dashboard are forwarded to Telegram with a
  `[web]` label prefix.
- Messages from Slack are forwarded to Telegram with a `[slack]` label
  prefix.
- Messages from Telegram are NOT re-forwarded to Telegram (no echo).
- AI responses are broadcast to all connected channels via the event
  system.

### 5.3 Source Tracking

Each `CliRequest` carries a `source` field (`"telegram"`, `"slack"`,
`"web"`, or `"system"`) that identifies the originating channel. This field
is used for:

- Determining which channels to forward to when sync is enabled.
- Enforcing sensitive-command restrictions (Section 3.2).
- Routing responses back to the correct channel adapter.

---

## 6. Chat History

### 6.1 Persistence

Chat history MUST be persisted as JSONL (JSON Lines) files. Each ANT has
its own history file:

```
~/.config/anthill/history/<ant-id>.jsonl
```

Each line is a JSON object with the following fields:

| Field     | Type   | Description                                      |
|-----------|--------|--------------------------------------------------|
| role      | string | `"user"` or `"bot"`                              |
| text      | string | The full message text                            |
| task_id   | u32    | The worker task ID (0 for non-task messages)     |
| timestamp | u64    | Unix timestamp in seconds                        |

### 6.2 Capacity

History files MUST be capped at 500 messages per ANT. When the cap is
exceeded, the implementation MUST retain the last 500 messages and
atomically rewrite the file:

1. Write the retained messages to a temporary file (`<name>.jsonl.tmp`).
2. Flush the buffer.
3. Rename the temporary file to the canonical path.

This atomic rewrite prevents corruption if the process is interrupted
mid-write.

### 6.3 Loading

History MUST be loaded lazily on first access (when a WebSocket client
connects or a message is appended for a given ANT). The history store
caches loaded histories in memory, keyed by ANT name.

### 6.4 Cross-Device Access

- History is loaded from the server on any device that connects via the
  web dashboard.
- New messages MUST be broadcast to all connected WebSocket clients via the
  global event bus.
- A user joining from a phone MUST see the same conversation history as on
  desktop.

### 6.5 System Messages

System-generated messages (compaction banners, connection status,
rumination summaries) are stored in history with `role: "system"` and
`task_id: 0`. These MUST be rendered with distinct styling in the web
dashboard to differentiate them from user and ANT messages.

---

## 7. Reply-to-Message

The web dashboard MUST support reply-to-message (quoting):

### 7.1 Selection

- Long-press or click on a message to select it for quoting.
- If the user has a text selection within the message, only the selected
  text is quoted.
- If no text is selected, the full message text is used, truncated to 300
  characters with a `...` suffix if necessary.

### 7.2 Quote Bar

When a message is selected for quoting:

- A quote bar MUST appear above the input area, displaying the quoted text.
- The quote bar MUST include a dismiss button to clear the quote.
- The quote bar MUST have a left accent border to visually indicate a
  reply context.

### 7.3 Message Composition

When the user sends a message with a pending quote:

1. The quoted text MUST be prepended to the message as a markdown
   blockquote (each line prefixed with `> `).
2. A blank line MUST separate the blockquote from the user's new text.
3. The quote MUST be cleared after sending.

Example composed message:

```
> Previously you said the API uses REST.
> But the docs mention GraphQL.

Can you clarify which endpoints use which protocol?
```

---

## 8. Message Rendering

### 8.1 Markdown

All messages MUST be rendered as GitHub-Flavoured Markdown using the
marked.js library. Supported elements:

- Headings (H1--H6)
- Bold (`**text**`) and italic (`*text*`)
- Inline code (`` `code` ``) and fenced code blocks (` ``` `)
- Blockquotes (`> text`)
- Unordered lists (`-`, `*`) and ordered lists (`1.`, `2.`)
- Tables (pipe-delimited)
- Links (`[text](url)`)
- Strikethrough (`~~text~~`)
- Horizontal rules (`---`)

### 8.2 Code Blocks

Fenced code blocks MUST include:

- Syntax highlighting based on the language tag.
- A "Copy" button that copies the code content to the clipboard.
- The copy button MUST revert its label from "Copied!" back to "Copy"
  after 1500 milliseconds.

### 8.3 System Messages

System messages (compaction banners, connection status notices, lag
warnings) MUST be styled distinctly from user and ANT messages. They SHOULD
use a muted colour and smaller font size to visually separate them from
conversation content.

---

## 9. Scroll Behaviour

The web dashboard MUST implement smart auto-scrolling:

1. When a new message arrives, the chat container MUST auto-scroll to the
   bottom ONLY if the user is near the bottom of the scroll area (within
   150 pixels of the bottom edge).

2. If the user has scrolled up to read earlier messages, scroll position
   MUST be preserved. The user MUST NOT be forcibly scrolled away from
   what they are reading.

3. When switching ANTs or opening a chat, the dashboard MUST always scroll
   to the bottom to show the most recent messages.

4. Scroll-to-bottom MUST be performed asynchronously via
   `requestAnimationFrame` to ensure the DOM has updated before
   calculating scroll position.

---

## 10. Conformance

An implementation claiming conformance to ANTHILL-CHAT:

1. **MUST** classify messages using the command codes defined in Section
   2.2.

2. **MUST** restrict sensitive commands to the web and system sources as
   defined in Section 3.2.

3. **MUST** implement auto-follow-up when exactly one task is running, as
   defined in Section 4.2.

4. **MUST** implement the interrupt (`!`) mechanism as defined in Section
   4.3.

5. **MUST** persist chat history as JSONL files capped at 500 messages, as
   defined in Section 6.

6. **MUST** perform atomic file rewrites when truncating history, as
   defined in Section 6.2.

7. **MUST** broadcast new messages to all connected WebSocket clients for
   cross-device access, as defined in Section 6.4.

8. **MUST** implement smart auto-scrolling in the web dashboard as defined
   in Section 9.

9. **SHOULD** implement command autocomplete in the web dashboard as
   defined in Section 3.4.

10. **SHOULD** implement reply-to-message quoting as defined in Section 7.

11. **SHOULD** render messages as GitHub-Flavoured Markdown as defined in
    Section 8.

12. **MAY** implement cross-channel sync as defined in Section 5. Where
    implemented, it MUST conform to the behaviour described.

---

## 11. References

- ANTHILL-SENTANT -- Conductor FSM, state transitions, plugin contract.
- ANTHILL-WORKER -- AI worker subprocess lifecycle and backend dispatch.
- ANTHILL-DASHBOARD -- Web dashboard UI, real-time channels.
- ANTHILL-TELEGRAM -- Telegram bot adapter.
- ANTHILL-SLACK -- Slack app adapter.
- ANTHILL-KNOWLEDGE -- Knowledge graph schema and persistence.
- R2-WIRE -- Event encoding and 256-byte envelope constraint.
- RFC 2119 -- Bradner, S. "Key words for use in RFCs to Indicate
  Requirement Levels." IETF, 1997.
