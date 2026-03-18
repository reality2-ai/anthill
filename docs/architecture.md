# Architecture

Anthill is built on the Reality2 (R2) sentant engine.

## Core concepts

**Sentants** — pure state machines. They receive events, make decisions, emit events. No I/O, no side effects. Given the same events, they produce the same actions. Always.

**Plugins** — service adapters. They bridge external systems (Telegram, Claude CLI, WebSocket) into the R2 event bus. They do I/O. They're the things you swap out when the platform changes.

**The rule:** Events carry decisions. Plugins carry data. If it doesn't fit in 256 bytes, it's data, not a decision — it belongs in the plugin data plane.

## Components

**Sentants:**

| Sentant | States | Role |
|---|---|---|
| ClaudeCliSentant | Idle → Running → Idle | Conductor — dispatches messages, handles commands |
| AiSentant | Idle → Translating → Executing → Summarising → Idle | AI mediation pipeline (ai mode) |

**Plugins:**

| Plugin | Role |
|---|---|
| TelegramPlugin | Bridges Telegram Bot API ↔ R2 events |
| ClaudeCliPlugin | Polls for completed Claude Code responses |
| PtyPlugin | Manages pseudo-terminal (raw mode) |
| AiPlugin | Polls for Claude API responses (ai mode) |

## Concurrent task execution

Each ANT uses a conductor/worker architecture:

- **Conductor** — always responsive. Accepts messages, dispatches to workers, handles `/ants` and `/cancel` locally.
- **Workers** — each message spawns an independent `claude -p` process. Multiple run in parallel.

Workers are tokio tasks, not separate processes. The conductor tracks them via a shared `TaskMap` for status reporting and cancellation.

## Supervisor

In production mode (`--supervise`), the supervisor:

1. Discovers ANT configs in the `ants/` directory
2. Spawns each ANT on a dedicated thread (the R2 EventBus is `!Send`)
3. Starts the web server alongside
4. Monitors ANT tasks, restarts crashed ones with backoff

ANTS register their handles (request channels, stats, task maps) with a shared `BotRegistry`, which the web server reads for the dashboard.

## Event flow

```
Telegram message
    → TelegramPlugin.poll() → RELAY_COMMAND event
    → Conductor sentant: dispatch to worker
    → Worker: spawn claude -p, stream typing indicators
    → claude -p completes
    → Worker: broadcast WsEvent::Message
    → ClaudeCliPlugin.poll() → RELAY_AI_READY event
    → Conductor sentant: send response
    → TelegramPlugin: deliver to Telegram
    → Web server: push to WebSocket clients
    → History recorder: persist to JSONL
```
