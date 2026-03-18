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

| Sentant | States | Role |
|---|---|---|
| ClaudeCliSentant | Ready | Dispatches messages, routes responses, handles /help /ants /cancel |
| AiSentant | Idle → Translating → Executing → Summarising | NL→command→summary pipeline (ai mode) |
| ChunkerSentant | Idle → Buffering | Output batching with debounce (raw mode) |
| TerminalSentant | Idle → Active | PTY lifecycle, input/output routing |
| TelegramSentant | Ready | Session event routing (PTY exit messages) |

A sentant's `handle_event` method does three things:
1. Match the event hash
2. Decide what to do (state transition, which plugin command)
3. Push `Action::plugin_call()` or `Action::send()` into the action buffer

That's it. No `send_telegram()`, no `pop_response()`, no mutex locks. Pure logic.

## Plugins (I/O adapters)

Plugins handle everything the sentants can't: network calls, file access, process spawning, data buffering.

| Plugin | Manages |
|---|---|
| ClaudeCliPlugin | Claude Code worker channels, task map, stats, message queue, Telegram sends |
| AiMediationPlugin | Claude API calls, output buffering, conversation history (ai mode) |
| ChunkerPlugin | ANSI stripping, output chunking, Telegram sends (raw mode) |
| TelegramPlugin | Bot API polling, message classification, outgoing sender, data plane queue |
| PtyPlugin | Pseudo-terminal spawning, I/O, process lifecycle |

Plugins communicate with each other through the **data plane** — shared `Arc<Mutex<VecDeque>>` queues, `mpsc` channels. For example:

- TelegramPlugin stores the full message text in a `MessageQueue`
- ClaudeCliPlugin reads from that same queue when the sentant tells it to dispatch
- The event between them carries only `{cmd_type: 0, chat_id: 123}` — 12 bytes

## Trust group security

The colony implements R2-TRUST provisioning:

1. **Colony root secret** — generated on first run, stored in `colony.key`
2. **Join codes** — short-lived (5 min), derived from root, one-use
3. **Device credentials** — permanent, derived at join time, stored in `devices.toml`
4. **Auth middleware** — every API call verified via `X-Credential` header

The server is the **queen** — it exists the moment Anthill starts. Browsers and phones are **viewers** that join via join codes. This is the same provisioning ceremony that would bring an ESP32 into a sensor trust group.

## Event flow (claude mode)

```
Telegram message arrives
    → TelegramPlugin.poll()
    → Stores full text in MessageQueue (data plane)
    → Emits RELAY_COMMAND { cmd_type: 0, chat_id: 123 } (12 bytes)
    → ClaudeCliSentant receives event
    → Decides: dispatch to Claude
    → Emits Action::plugin_call(CMD_DISPATCH, { chat_id: 123 })
    → ClaudeCliPlugin.execute(CMD_DISPATCH)
    → Pops message from MessageQueue (data plane)
    → Sends "Thinking..." to Telegram (plugin-to-plugin)
    → Spawns claude -p task
    → ... Claude works ...
    → ClaudeCliPlugin.poll()
    → Emits RELAY_AI_READY { chat_id: 123 } (12 bytes)
    → ClaudeCliSentant receives event
    → Decides: send reply
    → Emits Action::plugin_call(CMD_REPLY, { chat_id: 123 })
    → ClaudeCliPlugin.execute(CMD_REPLY)
    → Pops response from queue (data plane)
    → Sends to Telegram (plugin-to-plugin)
    → Broadcasts to WebSocket clients
```

The sentant touches zero bytes of message text. It only routes IDs.

## Supervisor

In production mode (`--supervise`), the supervisor:

1. Discovers ANT configs in the `ants/` directory
2. Spawns each ANT on a dedicated thread (EventBus is `!Send`)
3. Starts the web server with auth middleware
4. Starts the history recorder (listens to broadcast events)
5. Monitors ANT tasks, restarts crashed ones with backoff

ANTS register their handles with a shared `BotRegistry`, which the web server reads for the dashboard.

## Why R2?

The same architecture that coordinates sensor readings from ESP32 accelerometers now coordinates AI conversations from phones. The sentant model works because:

- **Determinism** — sentants are testable in isolation, no mock I/O needed
- **Portability** — the same sentant code could run on an ESP32, a Linux server, or an Elixir node
- **Separation** — changing the Telegram plugin to a Matrix plugin doesn't touch any sentant code
- **Security** — trust group provisioning is the same ceremony for a phone and a microcontroller
- **Scale** — the event bus handles any number of sentants; plugins scale independently
