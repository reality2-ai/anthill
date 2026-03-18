# Anthill

A colony for **ANTS** — Autonomous iNTelligenceS.

Anthill uses [Reality2](https://github.com/reality2-ai) (R2) — an open-source engine where autonomous agents (sentants) make decisions via events, and plugins handle all I/O. In Anthill, the sentants coordinate AI conversations while plugins manage [Claude Code](https://docs.anthropic.com/en/docs/claude-code), [Telegram](https://telegram.org/), [Slack](https://slack.com/), and the web interface. R2's trust group model secures access — devices join the colony via one-time codes, and every request is authenticated.

Anthill runs AI agents on a Linux server and lets you interact with them from any device — phone, laptop, tablet — via a built-in web dashboard, Telegram, or Slack.

Each ANT has its own personality, workspace, persistent memory, and can run multiple tasks concurrently. Access is controlled by a trust group — devices join the colony via one-time join codes.

Built on the [Reality2](https://github.com/reality2-ai) (R2) sentant engine — the same event-driven architecture used for IoT sensor networks, now proven for AI agents.

> **Note:** Anthill currently runs on **Linux only** and requires some technical setup (Rust toolchain, Claude Code CLI, systemd). See [Prerequisites](docs/prerequisites.md) for what's needed. macOS and Windows support may come in future.

## Quick start

```bash
git clone https://github.com/reality2-ai/anthill.git
cd anthill
cp anthill-example.toml anthill.toml
# Edit anthill.toml — set mode = "claude"
cargo run --release
```

Generate a join code and open the dashboard:

```bash
anthill --join-code              # prints a code (expires in 5 min)
# Open http://localhost:3000 → enter the code → you're in
```

## What is Reality2?

[Reality2](https://github.com/reality2-ai) (R2) is an event-driven architecture for autonomous agents — originally designed for IoT sensor networks on ESP32 microcontrollers, now proven to work for AI agents too.

The core principles:

- **Sentants** are pure state machines. They receive events, make decisions, emit actions. No I/O, no side effects, no network access. Given the same events, they always produce the same actions. Deterministic and testable.

- **Plugins** are service adapters. They bridge external systems (Telegram, Claude Code, web servers, hardware) into the event bus. All I/O happens here.

- **Events carry decisions** (< 256 bytes) — IDs, state codes, routing signals. **Plugins carry data** — message text, AI responses, file contents. If it doesn't fit in 256 bytes, it belongs in the plugin data plane, not the event bus.

- **Trust groups** control access. Devices join a colony by presenting a join code (R2-TRUST provisioning). Every API call and WebSocket connection is authenticated. The colony server is the queen; browsers and phones are authenticated viewers.

Anthill is the first production application of R2 outside sensor networks. The same architecture that coordinates accelerometer readings on ESP32s now coordinates AI agent conversations from phones.

### How Anthill uses R2

![Anthill Architecture](https://mermaid.ink/img/Z3JhcGggTFIKICAgIHN1YmdyYXBoIFZpZXdlcnMKICAgICAgICBQaG9uZVtQaG9uZSBicm93c2VyXQogICAgICAgIExhcHRvcFtMYXB0b3AgYnJvd3Nlcl0KICAgICAgICBUR1tUZWxlZ3JhbSBhcHBdCiAgICBlbmQKCiAgICBzdWJncmFwaCBTZXJ2ZXJbU2VydmVyIC0gdGhlIFF1ZWVuXQogICAgICAgIHN1YmdyYXBoIFBsdWdpbnNbUGx1Z2lucyAtIGFsbCBJL09dCiAgICAgICAgICAgIFdQW1dlYlBsdWdpbl0KICAgICAgICAgICAgVFBbVGVsZWdyYW1QbHVnaW5dCiAgICAgICAgICAgIENQW0NsYXVkZUNsaVBsdWdpbl0KICAgICAgICBlbmQKICAgICAgICBzdWJncmFwaCBGU01zW1NlbnRhbnRzIC0gcHVyZSBGU01zXQogICAgICAgICAgICBDU1tDbGF1ZGVDbGlTZW50YW50XQogICAgICAgIGVuZAogICAgICAgIENXW0NsYXVkZSBDb2RlXQogICAgZW5kCgogICAgUGhvbmUgLS0+fHRydXN0IGdyb3VwIGF1dGh8IFdQCiAgICBMYXB0b3AgLS0+fHRydXN0IGdyb3VwIGF1dGh8IFdQCiAgICBURyAtLT4gVFAKICAgIFdQIC0tPnxldmVudHMgLSAxMiBieXRlc3wgQ1MKICAgIFRQIC0tPnxldmVudHMgLSAxMiBieXRlc3wgQ1MKICAgIENTIC0tPnxwbHVnaW5fY2FsbHwgQ1AKICAgIENQIC0tPnxkYXRhIHBsYW5lfCBUUAogICAgQ1AgLS0+fGRhdGEgcGxhbmV8IFdQCiAgICBDUCAtLT4gQ1c=)

**Sentants** make decisions. **Plugins** handle data. Events are tiny. Data flows plugin-to-plugin.

| Sentants (pure FSMs — zero I/O) | Plugins (all I/O) |
|---|---|
| ClaudeCliSentant — dispatches, routes, commands | ClaudeCliPlugin — Claude Code, tasks, stats, sends |
| AiSentant — NL→command→summary pipeline | AiMediationPlugin — API calls, buffering, history |
| ChunkerSentant — output batching decisions | ChunkerPlugin — ANSI stripping, chunking, sends |
| TerminalSentant — PTY lifecycle | TelegramPlugin — Bot API, classification, data plane |
| TelegramSentant — session routing | PtyPlugin — pseudo-terminal management |

## Features

- **Multiple ANTS** — each with its own personality, workspace, and optional Telegram bot
- **Web dashboard** — responsive, installable as PWA, accessible via [Tailscale](https://tailscale.com/)
- **Trust group security** — join codes, device provisioning, auth on every request
- **Concurrent tasks** — send messages while others are running; `/ants` to check progress
- **Real-time progress** — see what each worker is doing (tool use, agent spawns)
- **Per-user memory** — persistent across conversations and restarts
- **File browser** — browse workspace, upload/download files, preview images and code
- **Git-backed workspace** — auto-committed on schedule, optionally pushed to GitHub
- **ANT management UI** — create, configure, delete ANTS from the dashboard
- **Device management** — provision/revoke devices from the dashboard
- **Telegram + Slack** — optional, both can be active simultaneously on the same ANT
- **Markdown rendering** — code blocks, headings, tables, links in both Telegram and web
- **Auto-restart** — supervisor manages ANTS with exponential backoff
- **Systemd integration** — starts on boot, HTTPS via Tailscale

## Documentation

| Guide | What it covers |
|---|---|
| [Prerequisites](docs/prerequisites.md) | Linux, Rust, Node.js, Claude Code, Telegram, Slack, Tailscale |
| [Getting Started](docs/getting-started.md) | Single ANT setup, first message |
| [Production Setup](docs/production-setup.md) | Multiple ANTS, supervisor, systemd |
| [Web Dashboard](docs/web-dashboard.md) | Tailscale HTTPS, PWA, cross-device history |
| [Configuration](docs/configuration.md) | Full reference for supervisor.toml and ant.toml |
| [Commands](docs/commands.md) | Telegram and dashboard commands |
| [Memory & Workspaces](docs/memory-and-workspaces.md) | Per-user memory, git backups |
| [Architecture](docs/architecture.md) | R2 sentant engine, events, plugins |
| [Troubleshooting](docs/troubleshooting.md) | Common issues and fixes |
| [Security](docs/security.md) | Trust groups, device provisioning, access control |

## License

MIT OR Apache-2.0
