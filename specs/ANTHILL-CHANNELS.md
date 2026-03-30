# ANTHILL-CHANNELS: Multi-Channel Experience

| Field      | Value                                                        |
|------------|--------------------------------------------------------------|
| Version    | 0.1 Draft                                                    |
| Date       | 2026-03-30                                                   |
| Status     | Draft                                                        |
| Depends on | ANTHILL-CHAT, ANTHILL-TRUST                                  |
| Related    | ANTHILL-DASHBOARD, ANTHILL-SENTANT                           |

> The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT",
> "SHOULD", "SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this
> document are to be interpreted as described in RFC 2119.

---

## 1. Introduction

Anthill exposes each ANT through three communication channels: the web
dashboard, Telegram, and Slack. Each channel is implemented as an R2 plugin
that translates platform-specific messages into R2 events and relays ANT
responses back to users. The channels share a common AI worker pipeline but
differ in capabilities, security properties, and message formatting.

This specification defines the architecture of the channel layer, the feature
differences between channels, the security model that governs which operations
each channel MAY perform, and the cross-channel synchronisation mechanism.

---

## 2. Channel Architecture

Each channel is an R2 plugin (R2-PLUGIN) that implements the `Plugin` trait.
Channels interact with the ANT sentant exclusively through R2 events on the
event bus and through the shared message queue on the plugin data plane.

### 2.1 Event Bus vs Data Plane

The R2 event bus carries small decision events (256-byte limit per R2-WIRE).
User message text, which may be arbitrarily long, travels on the plugin data
plane -- a shared `MessageQueue` (a `Mutex<VecDeque<(i64, String, String)>>`)
that stores `(chat_id, text, source)` tuples.

When a channel plugin receives an incoming message it MUST:

1. Classify the message into a command code (Section 5).
2. Push the full message text onto the shared message queue with its source
   label (`"telegram"`, `"slack"`, or `"web"`).
3. Emit a small CBOR-encoded `RELAY_COMMAND` event on the R2 event bus
   containing only: `{ 0: uint(cmd_type), 1: uint(chat_id), 2: uint(cancel_task_id) }`.

The AiPlugin consumes the `RELAY_COMMAND` event, pops the corresponding
message from the shared queue, and dispatches it to an AI worker.

### 2.2 Web Channel

The web dashboard is served by an Axum HTTP server embedded in the Anthill
binary. Real-time communication uses WebSocket with signed envelopes.

- **Transport:** WebSocket (`/ws`) over HTTP/HTTPS.
- **Authentication:** R2-TRUST device credential passed as a query parameter
  on WebSocket connect and as an `X-Credential` header on REST calls.
- **Message signing:** Every WebSocket frame is wrapped in an HMAC-SHA256
  signed envelope containing `device_id`, `timestamp`, `signature`, and
  `payload`. The server signs outgoing frames; clients SHOULD sign incoming
  frames.
- **Command handling:** The web channel handles some commands locally
  (e.g. `/help`, `/ants`, `/status`, `/usage`, `/reflect`) via
  `handle_web_command()` without routing through the event bus. Commands that
  require AI inference (regular messages, `/analyse`, `/specify`,
  `/test-vectors`) are dispatched to the AI worker via `registry.send_message()`
  with source `"web"`.
- **Real-time events:** The server broadcasts `WsEvent` variants (messages,
  task progress, task completion, graph updates, knowledge changes) to all
  connected WebSocket clients.

### 2.3 Telegram Channel

The Telegram channel uses the `teloxide` crate to run a long-polling bot.

- **Transport:** HTTPS to Telegram Bot API servers (long polling via
  `Dispatcher`).
- **Authentication:** Telegram bot token. An optional `allow` list restricts
  which chat IDs may interact.
- **Message formatting:** Markdown responses are converted to Telegram-
  compatible HTML via a two-pass converter that handles headings (four levels),
  bold, italic, inline code, fenced code blocks, bullet lists, numbered lists,
  links, strikethrough, and horizontal rules. If HTML parsing fails on send,
  the plugin falls back to plain text.
- **Message splitting:** Messages exceeding 4000 characters are split on
  character boundaries to avoid corrupting multi-byte UTF-8.
- **Typing indicator:** An empty text payload triggers a `ChatAction::Typing`
  indicator.
- **Plugin commands:** `CMD_SEND_TEXT` (0x01) sends a text message;
  `CMD_SEND_MONO` (0x02) wraps text in a fenced code block before sending.
- **Outgoing data plane:** The AiPlugin holds a direct
  `mpsc::UnboundedSender<(i64, String)>` to the Telegram plugin for sending
  responses, bypassing the 256-byte event bus limit.

### 2.4 Slack Channel

The Slack channel uses Socket Mode (WebSocket) so no public URL is required.

- **Transport:** WebSocket via Slack Socket Mode API
  (`apps.connections.open`). Outgoing messages use the Slack Web API
  (`chat.postMessage`) with bearer authentication.
- **Authentication:** Requires both a `bot_token` (for posting messages) and
  an `app_token` (for Socket Mode WebSocket). No chat-ID-level allow list is
  currently implemented.
- **Reconnection:** On WebSocket disconnection, the plugin reconnects after a
  5-second delay.
- **Message handling:** The plugin listens for `events_api` envelopes of type
  `message`, ignores bot messages (those with `bot_id` or `subtype`) to
  prevent loops, and acknowledges every envelope by echoing the `envelope_id`.
- **Channel ID mapping:** Slack channel IDs (strings) are hashed to `u64`
  values using FNV-1a for compatibility with the numeric `chat_id` used
  internally.
- **Output format:** Messages are posted with `mrkdwn: true` (Slack's native
  markdown variant).
- **Plugin model:** The Slack plugin is currently input-only at the R2 plugin
  command level (`execute()` returns an error). Outgoing messages are routed
  through the AiPlugin's data plane, which sends them via the Telegram sender.
  Direct Slack outgoing support uses a separate `out_tx` channel to the Slack
  Web API poster.

---

## 3. Feature Parity Matrix

The following table summarises feature support across the three channels. "All"
means all slash commands listed in Section 5; "Subset" means the commands
listed in the channel's `classify_command` function.

| Feature                        | Web                     | Telegram                | Slack                   |
|--------------------------------|-------------------------|-------------------------|-------------------------|
| Free-text prompts              | Yes                     | Yes                     | Yes                     |
| Markdown rendering             | Full (HTML/CSS)         | Converted to HTML       | Slack mrkdwn            |
| Fenced code blocks             | Yes (syntax highlight)  | `<pre><code>` HTML      | Yes (native)            |
| File upload                    | Yes (REST API)          | No                      | No                      |
| File browser                   | Yes (REST API)          | No                      | No                      |
| Knowledge graph visualisation  | Yes (3D force graph)    | No                      | No                      |
| Worker progress (live)         | Yes (WebSocket events)  | No (final result only)  | No (final result only)  |
| Task cards (active workers)    | Yes (real-time UI)      | Via /status command     | Via /status command     |
| Follow-ups (/followup)         | Yes (WebSocket command) | Yes                     | Yes                     |
| Auto follow-up (implicit)      | Yes                     | Yes                     | Yes                     |
| Interrupt with ! prefix        | Yes                     | Yes                     | Yes                     |
| /help                          | Yes                     | Yes                     | Yes                     |
| /ants                          | Yes                     | Yes                     | Yes                     |
| /usage                         | Yes                     | Yes                     | Yes                     |
| /status                        | Yes                     | Yes                     | Yes                     |
| /cancel, /cancel all           | Yes                     | Yes                     | Yes                     |
| /new (fresh session)           | Yes                     | Yes                     | Yes                     |
| /reflect                       | Yes                     | Yes                     | Yes                     |
| /ruminate                      | Yes                     | Yes                     | No (not classified)     |
| /citations                     | Yes                     | Yes                     | No (not classified)     |
| /analyse (thematic analysis)   | Yes                     | Classified but blocked  | Classified but blocked  |
| /specify (spec generation)     | Yes                     | Classified but blocked  | Classified but blocked  |
| /test-vectors                  | Yes                     | Classified but blocked  | Classified but blocked  |
| /ask (inter-ANT query)         | Yes (via @mention)      | No                      | No                      |
| /report (background report)    | Yes                     | No                      | No                      |
| /export (knowledge graph HTML) | Yes                     | No                      | No                      |
| @mention (colony queries)      | Yes                     | No                      | No                      |
| Device provisioning            | Yes (QR + join code)    | N/A                     | N/A                     |
| Device management              | Yes (list + revoke)     | N/A                     | N/A                     |
| ANT creation / deletion        | Yes (REST API)          | No                      | No                      |
| ANT configuration editing      | Yes (REST API)          | No                      | No                      |
| HMAC message signing           | Yes                     | No                      | No                      |
| Trust group authentication     | Yes                     | No                      | No                      |
| Message history (persisted)    | Yes (loaded on connect) | No (Telegram-side only) | No (Slack-side only)    |

### 3.1 Notes on Parity Gaps

- **`/ruminate` and `/citations`**: The Slack `classify_command` function does
  not map these commands (they fall through to command code 0, treated as a
  regular message). The Telegram parser maps `/ruminate` to code 13 and
  `/citations` to code 17. Implementations SHOULD add these to the Slack
  parser for parity.

- **Sensitive commands**: `/analyse`, `/specify`, and `/test-vectors` are
  classified by both Telegram and Slack parsers but are blocked at the
  AiPlugin level via `is_sensitive_allowed()`, which returns `true` only for
  source `"web"` or `"system"`. These commands read workspace files and
  produce structured output best reviewed in the web UI.

- **Web-only features**: File management, knowledge graph visualisation,
  report generation, ANT lifecycle management, and @mention colony queries are
  available only through the web dashboard.

---

## 4. Security Comparison

Anthill applies defence in depth. The web channel implements all seven layers;
Telegram and Slack rely on third-party transport security only.

### 4.1 Defence-in-Depth Layers

| Layer | Defence                          | Web | Telegram | Slack |
|-------|----------------------------------|-----|----------|-------|
| 1     | Network isolation (Tailscale)    | Yes | No       | No    |
| 2     | Transport encryption (HTTPS/TLS) | Yes | Third-party TLS | Third-party TLS |
| 3     | Trust group membership           | Yes | No       | No    |
| 4     | Device credential (Ed25519)      | Yes | No       | No    |
| 5     | HMAC-SHA256 message signing      | Yes | No       | No    |
| 6     | Validated writes (Thurisaz)      | Yes | Yes      | Yes   |
| 7     | R2 architecture (256-byte limit) | Yes | Yes      | Yes   |

### 4.2 Web Channel Security

The web channel implements the full R2-TRUST model:

1. **Colony key** -- an Ed25519 signing key generated on first run
   (`colony.key`). The server is the key holder (the queen).
2. **Join codes** -- 48-bit single-use tokens (`xxxx-xxxx-xxxx`), valid for
   5 minutes. Generated via CLI or the web dashboard.
3. **Device credentials** -- Ed25519 private key seeds issued on join, stored
   in the browser's `localStorage` and in `devices.toml` on the server.
4. **Authentication** -- every REST call requires an `X-Credential` header.
   WebSocket connections require a credential query parameter. No credential
   means 401 Unauthorized.
5. **Message signing** -- every WebSocket frame is wrapped in a signed
   envelope: `{ device_id, timestamp, signature, payload }`.

### 4.3 Telegram Channel Security

Telegram security relies entirely on:

- TLS between the Telegram client and Telegram servers.
- TLS between Telegram servers and the Anthill bot (polling).
- The `allow` list, which restricts interaction to specific chat IDs.

The `allow` list is RECOMMENDED. Without it, anyone who discovers the bot
username can send commands. Implementations MUST document this risk.

### 4.4 Slack Channel Security

Slack security relies on:

- TLS for Socket Mode WebSocket connections.
- TLS for Slack Web API calls.
- Slack workspace membership (only workspace members can message the bot).

No additional Anthill-level authentication is applied.

### 4.5 Sensitive Command Restriction

The following commands are restricted to the web channel and MUST be blocked
when the source is `"telegram"` or `"slack"`:

| Command           | Reason                                                    |
|-------------------|-----------------------------------------------------------|
| `/analyse <file>` | Reads workspace files; output best reviewed in web UI     |
| `/specify <file>` | Reads workspace files; generates structured spec output   |
| `/test-vectors <file>` | Reads workspace files; generates structured test output |

The AiPlugin enforces this restriction via the `is_sensitive_allowed()`
function, which returns `true` only when `source` is `"web"` or `"system"`.
When a restricted command is received from Telegram or Slack, the plugin
MUST respond with a warning message directing the user to the web dashboard.

---

## 5. Message Classification

Each channel classifies incoming messages into numeric command codes. The
command code is carried in the `RELAY_COMMAND` event payload at CBOR map
key 0. The AiPlugin dispatches to the appropriate handler based on this code.

### 5.1 Command Code Table

| Code | Command              | Telegram | Slack | Web (local) | Web (worker) |
|------|----------------------|----------|-------|-------------|--------------|
| 0    | Regular message      | Yes      | Yes   | No          | Yes          |
| 1    | /help, /start        | Yes      | Yes   | Yes         | No           |
| 2    | /ants, /bots         | Yes      | Yes   | Yes         | No           |
| 3    | /usage               | Yes      | Yes   | Yes         | No           |
| 4    | /cancel <id>         | Yes      | Yes   | Yes         | No           |
| 5    | /cancel all          | Yes      | Yes   | Yes         | No           |
| 6    | /new                 | Yes      | Yes   | No          | Yes          |
| 7    | /status              | Yes      | Yes   | Yes         | No           |
| 8    | /followup <text>     | Yes      | Yes   | Yes         | No           |
| 9    | /analyse <file>      | Yes      | Yes   | No          | Yes          |
| 10   | /reflect             | Yes      | Yes   | Yes         | No           |
| 11   | /specify <file>      | Yes      | Yes   | No          | Yes          |
| 12   | /test-vectors <file> | Yes      | Yes   | No          | Yes          |
| 13   | /ruminate            | Yes      | No    | No          | Yes          |
| 17   | /citations           | Yes      | No    | No          | Yes          |

### 5.2 Telegram Parser

The Telegram `classify_command` function performs exact string matching on the
trimmed input. For commands with arguments (`/followup`, `/analyse`,
`/specify`, `/test-vectors`), it matches the prefix followed by a space. The
`/cancel` command has special handling: bare `/cancel` cancels the most recent
task, `/cancel <id>` cancels a specific task (parsed by `parse_cancel_id`),
and `/cancel all` cancels all tasks.

Command prefixes are stripped from the queue text before pushing to the message
queue so downstream handlers receive only the argument content.

### 5.3 Slack Parser

The Slack `classify_command` function mirrors the Telegram parser with two
exceptions: `/ruminate` (code 13) and `/citations` (code 17) are not mapped.
Messages matching these commands fall through to code 0 (regular message) and
are sent to the AI as free-text prompts.

Slack channel IDs (strings like `C01ABC23DEF`) are hashed to `u64` values
using FNV-1a (`hash_channel`) for use as numeric chat IDs in the shared
message queue.

### 5.4 Web Parser

The web channel does not use the numeric classification system. Instead,
`handle_web_command()` performs string matching on the trimmed message and
handles commands locally where possible (returning results directly via
WebSocket broadcast). Commands that require AI inference or file access are
dispatched to the AI worker via `registry.send_message()` with source `"web"`.

The web channel supports additional commands not available in Telegram or
Slack: `/ask`, `/report`, `/export`, and @mention syntax for inter-ANT
queries (handled by `extract_mentions()`).

---

## 6. Cross-Channel Synchronisation

When `sync_channels = true` in the ANT's configuration, user messages are
forwarded across channels so that participants on one platform can see activity
from another.

### 6.1 Forwarding Behaviour

When the AiPlugin receives a dispatched message and `sync_channels` is true:

1. If the source is not `"telegram"`, the message is forwarded to Telegram
   with a source label prefix: `[web]` or `[slack]`.
2. The label uses emoji indicators for visual distinction:
   - Web: `[🌐 web]`
   - Slack: `[💬 slack]`

Messages originating from Telegram are NOT forwarded back to Telegram (to
prevent echo loops).

### 6.2 Chat ID Tracking

Each message in the shared queue carries a `chat_id` and `source` label.
The AiPlugin uses the `chat_id` to route responses back to the originating
channel. For Telegram, the `chat_id` is the native Telegram chat ID (i64).
For Slack, it is the FNV-1a hash of the Slack channel ID string. For web, it
is either 0 (default) or a client-provided value.

### 6.3 Response Routing

AI worker responses are routed primarily to Telegram via the
`telegram_tx` data plane sender. The web channel receives responses through
the `WsEvent::Message` broadcast on the global event channel, which all
connected WebSocket clients subscribe to. Slack outgoing messages are sent
via the Slack Web API poster task.

### 6.4 Limitations

- Cross-channel sync currently forwards user messages only in the Telegram
  direction. Web clients see all activity through the WebSocket broadcast
  regardless of `sync_channels`.
- Source labels are prepended as plain text, not structured metadata.
- File attachments and rich media are not forwarded across channels.

---

## 7. Telegram-Specific

### 7.1 Bot Token Configuration

The Telegram bot token is configured in the ANT's `ant.toml` file under
`[telegram]`. The token MUST be obtained from BotFather on Telegram.

```toml
[telegram]
token = "123456:ABC-DEF1234ghIkl-zyx57W2v1u123ew11"
allow = [123456789]  # restrict to specific chat IDs
```

### 7.2 Allow List

The `allow` field is a list of Telegram chat IDs permitted to interact with
the bot. When empty, all chats are allowed. Implementations SHOULD configure
this list in production to prevent unauthorised access.

### 7.3 Polling

The bot uses `teloxide::Dispatcher` with long polling (the default teloxide
strategy). No webhook configuration is required. The bot connects outbound
to Telegram servers, so no inbound port needs to be open.

### 7.4 Markdown to HTML Conversion

The plugin converts markdown responses to Telegram-compatible HTML using a
two-pass approach:

1. **First pass:** Extract fenced code blocks verbatim into `<pre><code>`
   elements.
2. **Second pass:** Process remaining lines for headings (`#` through `####`
   mapped to bold-uppercase, bold, bold-italic, and italic respectively),
   bullet lists (converted to Unicode bullet `\u2022`), horizontal rules,
   and inline formatting (bold, italic, inline code, strikethrough, links).

If the resulting HTML fails Telegram's parse validation on send, the plugin
falls back to sending the original plain text.

### 7.5 Message Size Limit

The Telegram API enforces a maximum message size. The AiPlugin splits
responses exceeding 4000 characters at valid UTF-8 character boundaries.

---

## 8. Slack-Specific

### 8.1 Socket Mode

The Slack plugin uses Socket Mode, which establishes a WebSocket connection
to Slack's infrastructure via the `apps.connections.open` API endpoint. This
means no public URL or inbound port is required -- the plugin works behind
NAT, firewalls, and Tailscale.

### 8.2 Token Configuration

Slack requires two tokens, configured in `ant.toml` under `[slack]`:

```toml
[slack]
bot_token = "xoxb-..."   # Bot User OAuth Token
app_token = "xapp-..."   # App-Level Token (Socket Mode)
```

The `bot_token` is used for posting messages via the Web API. The `app_token`
is used to open the Socket Mode WebSocket connection.

### 8.3 Message Handling

The plugin processes `events_api` envelopes and extracts `message` events.
It MUST:

1. Acknowledge every envelope by sending `{"envelope_id": "<id>"}` back on
   the WebSocket.
2. Ignore messages with a `bot_id` or `subtype` field to prevent bot loops.
3. Extract `text`, `channel`, and `user` from the event payload.

### 8.4 Outgoing Messages

Outgoing messages are posted to the Slack Web API (`chat.postMessage`) with
`mrkdwn: true` enabled. The `bot_token` is used as bearer authentication.

### 8.5 Reconnection

If the Socket Mode WebSocket connection drops, the plugin logs the error and
reconnects after a 5-second delay. This loop runs indefinitely.

---

## 9. Web-Specific

### 9.1 WebSocket Protocol

The web dashboard connects to `/ws` with the device credential as a query
parameter:

```
ws://host:port/ws?credential=<device_credential>&device_id=<device_id>
```

The server verifies the credential before upgrading the connection. Invalid
credentials result in 401 Unauthorized.

### 9.2 Signed Envelopes

Every WebSocket frame is wrapped in a JSON envelope:

```json
{
  "device_id": "server",
  "timestamp": 1711756800,
  "signature": "<HMAC-SHA256 hex>",
  "payload": "<JSON string>"
}
```

The server signs all outgoing frames using the device credential as the HMAC
key. Clients SHOULD verify the `signature` and `timestamp` to detect tampering
and replay attacks.

### 9.3 Initial Snapshot

On connection, the server sends a `snapshot` event containing:

- `bots` -- list of all ANTs with status.
- `history` -- persisted chat history for all ANTs.
- `tasks` -- currently active tasks per ANT (with preview, elapsed time,
  progress, and backend).

### 9.4 Real-Time Events

The server broadcasts the following event types to all connected clients:

| Event Type       | Description                                         |
|------------------|-----------------------------------------------------|
| `message`        | ANT response text (with bot, chat_id, task_id)      |
| `user_message`   | User input echoed back (with source label)           |
| `task_started`   | A new AI worker task has been dispatched              |
| `task_progress`  | Worker progress update (live status text)             |
| `task_completed` | Worker finished (with duration)                       |
| `task_error`     | Worker failed or timed out                            |
| `graph_update`   | Knowledge graph was modified (with graph name, source)|

### 9.5 WebSocket Commands

Clients send JSON commands through the WebSocket:

| Command      | Fields                        | Description                     |
|--------------|-------------------------------|---------------------------------|
| `chat`       | bot, message, chat_id?        | Send a message to an ANT        |
| `cancel`     | bot, task_id                  | Cancel a running task            |
| `follow_up`  | bot, task_id, message         | Queue follow-up for a task       |

### 9.6 REST API

The web channel exposes a comprehensive REST API behind credential
authentication. Key endpoints:

| Method   | Path                              | Description                    |
|----------|-----------------------------------|--------------------------------|
| GET      | /api/ants                         | List all ANTs with status      |
| POST     | /api/ants/{id}/chat               | Send message to ANT            |
| POST     | /api/ants/{id}/cancel/{task_id}   | Cancel a task                  |
| GET/PUT  | /api/ants/{id}/config             | Read/update ANT configuration  |
| POST     | /api/ants/create                  | Create a new ANT               |
| DELETE   | /api/ants/{id}                    | Delete an ANT                  |
| GET      | /api/ants/{id}/graph              | Knowledge graph data           |
| GET      | /api/ants/{id}/export             | Export knowledge graph as HTML |
| POST     | /api/ants/{id}/report             | Start background report        |
| GET      | /api/ants/{id}/rumination         | Rumination log                 |
| GET      | /api/ants/{id}/files              | List workspace files           |
| POST     | /api/ants/{id}/upload/{path}      | Upload file to workspace       |
| GET      | /api/backends                     | List available AI backends     |
| GET      | /api/doctor                       | System health check            |

Public (unauthenticated) endpoints are limited to: `/`, `/api/auth/verify`,
`/api/auth/join`, `/api/auth/status`, and static assets.

---

## 10. Conformance

An implementation claiming conformance to this specification:

1. MUST implement at least one channel (web is REQUIRED).
2. MUST enforce the sensitive command restriction (Section 4.5) for all
   non-web channels.
3. MUST classify messages into command codes as defined in Section 5.1, or
   document deviations.
4. MUST implement the shared message queue architecture (Section 2.1) to
   separate the event bus from content delivery.
5. MUST sign all WebSocket frames with HMAC-SHA256 envelopes when the web
   channel is active.
6. SHOULD implement cross-channel synchronisation (Section 6) when more than
   one channel is configured.
7. SHOULD implement the Telegram allow list when the Telegram channel is
   active.
8. MAY omit the Telegram or Slack channel entirely; where implemented, the
   channel MUST conform to Sections 7 or 8 respectively.

---

## 11. References

- R2-WIRE -- Reality2 wire protocol (256-byte event limit).
- R2-TRUST -- Reality2 trust group and device provisioning model.
- R2-PLUGIN -- Reality2 plugin trait and lifecycle.
- ANTHILL-CHAT -- Conversation protocol and message formatting.
- ANTHILL-TRUST -- Anthill trust and security specification.
- ANTHILL-DASHBOARD -- Web dashboard specification.
- ANTHILL-SENTANT -- ANT conductor FSM and plugin contract.
- Telegram Bot API -- https://core.telegram.org/bots/api
- Slack Socket Mode -- https://api.slack.com/apis/connections/socket
- RFC 2119 -- Key words for use in RFCs to Indicate Requirement Levels.
