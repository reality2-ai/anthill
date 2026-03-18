# Anthill

A colony for **ANTS** — Autonomous iNTelligenceS.

Anthill uses [Reality2](https://reality2.ai) (R2) — an open-source engine where autonomous agents (sentants) make decisions via events, and plugins handle all I/O. In Anthill, the sentants coordinate AI conversations while plugins manage connections to AI backends (such as [Claude](https://www.anthropic.com/), [ChatGPT](https://openai.com/), or local models via [Ollama](https://ollama.com/)), messaging platforms ([Telegram](https://telegram.org/), [Slack](https://slack.com/)), and the web interface. R2's trust group model secures access — devices join the colony via one-time codes, and every request is authenticated.

Anthill runs AI agents on a Linux server and lets you interact with them from any device — phone, laptop, tablet — via a built-in web dashboard, [Telegram](https://telegram.org/), or [Slack](https://slack.com/). The web dashboard is accessed securely over [Tailscale](https://tailscale.com/) — a private network that connects your devices without exposing anything to the public internet. HTTPS is provided automatically via Tailscale's built-in certificate provisioning.

Each ANT has its own personality, workspace, persistent memory, and can run multiple tasks concurrently. Access is controlled by a trust group — devices join the colony via one-time join codes.

Built on the [Reality2](https://reality2.ai) (R2) sentant engine — a software stack for wearables and IoT, now proven for AI agents.

> **Note:** Anthill currently runs on **Linux only** and requires some technical setup (Rust toolchain, AI CLI tools, systemd). See [Prerequisites](docs/prerequisites.md) for what's needed. macOS and Windows support may come in future.

## Quick start

```bash
git clone https://reality2.ai/anthill.git
cd anthill
cp anthill-example.toml anthill.toml
# Edit anthill.toml — set mode = "claude"
cargo run --release
```

Generate a join code and open the dashboard:

```bash
anthill --join-code              # prints a code (expires in 5 min)
# Open http://localhost:3000 (or your Tailscale IP/hostname) → enter the code → you're in
```

## What is Reality2?

[Reality2](https://reality2.ai) (R2) is a new software stack for wearables and IoT devices — an event-driven architecture for autonomous agents. Now proven to work for AI agents too.

The core principles:

- **Sentants** are pure state machines. They receive events, make decisions, emit actions. No I/O, no side effects, no network access. Given the same events, they always produce the same actions. Deterministic and testable.

- **Plugins** are service adapters. They bridge external systems (AI backends, Telegram, Slack, web servers, hardware) into the event bus. All I/O happens here.

- **Events carry decisions** (< 256 bytes) — IDs, state codes, routing signals. **Plugins carry data** — message text, AI responses, file contents. If it doesn't fit in 256 bytes, it belongs in the plugin data plane, not the event bus.

- **Trust groups** control access. Devices join a colony by presenting a join code (R2-TRUST provisioning). Every API call and WebSocket connection is authenticated. The colony server is the queen; browsers and phones are authenticated viewers.

Anthill is the first production application of R2 outside sensor networks. The same architecture that coordinates accelerometer readings on ESP32s now coordinates AI agent conversations from phones.

### How Anthill uses R2

![Anthill Architecture](https://mermaid.ink/img/Z3JhcGggTFIKICAgIHN1YmdyYXBoIFZpZXdlcnMKICAgICAgICBQaG9uZVtQaG9uZSBicm93c2VyXQogICAgICAgIExhcHRvcFtMYXB0b3AgYnJvd3Nlcl0KICAgICAgICBUR1tUZWxlZ3JhbSBhcHBdCiAgICAgICAgU0xbU2xhY2sgYXBwXQogICAgZW5kCgogICAgc3ViZ3JhcGggU2VydmVyW1NlcnZlciAtIHRoZSBRdWVlbl0KICAgICAgICBzdWJncmFwaCBQbHVnaW5zW1BsdWdpbnMgLSBhbGwgSS9PXQogICAgICAgICAgICBXUFtXZWJQbHVnaW5dCiAgICAgICAgICAgIFRQW1RlbGVncmFtUGx1Z2luXQogICAgICAgICAgICBTUFtTbGFja1BsdWdpbl0KICAgICAgICAgICAgQVBbQUlQbHVnaW5dCiAgICAgICAgZW5kCiAgICAgICAgc3ViZ3JhcGggRlNNc1tTZW50YW50cyAtIHB1cmUgRlNNc10KICAgICAgICAgICAgQ1NbQ29uZHVjdG9yXQogICAgICAgIGVuZAogICAgICAgIE9MW09sbGFtYV06OjpmdXR1cmUKICAgIGVuZAoKICAgIHN1YmdyYXBoIEV4dGVybmFsW0V4dGVybmFsIEFJXQogICAgICAgIENsYXVkZVtDbGF1ZGVdCiAgICAgICAgT0FJW09wZW5BSV06OjpmdXR1cmUKICAgIGVuZAoKICAgIFBob25lIDwtLT58dHJ1c3QgZ3JvdXAgYXV0aHwgV1AKICAgIExhcHRvcCA8LS0+fHRydXN0IGdyb3VwIGF1dGh8IFdQCiAgICBURyA8LS0+IFRQCiAgICBTTCA8LS0+IFNQCiAgICBXUCA8LS0+fGV2ZW50c3wgQ1MKICAgIFRQIDwtLT58ZXZlbnRzfCBDUwogICAgU1AgPC0tPnxldmVudHN8IENTCiAgICBDUyA8LS0+fHBsdWdpbl9jYWxsfCBBUAogICAgQVAgPC0tPnxkYXRhIHBsYW5lfCBUUAogICAgQVAgPC0tPnxkYXRhIHBsYW5lfCBTUAogICAgQVAgPC0tPnxkYXRhIHBsYW5lfCBXUAogICAgQVAgPC0tPiBDbGF1ZGUKICAgIEFQIC0uLXxjb21pbmcgc29vbnwgT0FJCiAgICBBUCAtLi18Y29taW5nIHNvb258IE9MCgogICAgY2xhc3NEZWYgZnV0dXJlIHN0cm9rZS1kYXNoYXJyYXk6IDUgNSxvcGFjaXR5OjAuNQ==)

**Sentants** make decisions. **Plugins** handle data. Events are tiny. Data flows plugin-to-plugin.

| Sentants (pure FSMs — zero I/O) | Plugins (all I/O) |
|---|---|
| Conductor — dispatches, routes, commands | AIPlugin — Claude (+ OpenAI, Ollama coming) |
| AiSentant — NL→command→summary pipeline | TelegramPlugin — Bot API, message classification |
| ChunkerSentant — output batching decisions | SlackPlugin — Socket Mode, message routing |
| TerminalSentant — PTY lifecycle | WebPlugin — dashboard, WebSocket, file browser |
| TelegramSentant — session routing | ChunkerPlugin, PtyPlugin — raw mode I/O |

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
| [Prerequisites](docs/prerequisites.md) | Linux, Rust, AI backend (Claude, OpenAI, Ollama), Telegram/Slack, Tailscale |
| [Getting Started](docs/getting-started.md) | Single ANT setup, first message |
| [Production Setup](docs/production-setup.md) | Multiple ANTS, supervisor, systemd |
| [Web Dashboard](docs/web-dashboard.md) | Tailscale HTTPS, PWA, cross-device history |
| [Configuration](docs/configuration.md) | Full reference for supervisor.toml and ant.toml |
| [Commands](docs/commands.md) | Telegram and dashboard commands |
| [Memory & Workspaces](docs/memory-and-workspaces.md) | Per-user memory, git backups |
| [Architecture](docs/architecture.md) | R2 sentant engine, events, plugins |
| [Troubleshooting](docs/troubleshooting.md) | Common issues and fixes |
| [Security](docs/security.md) | Trust groups, device provisioning, access control |

## Why not just use OpenClaw?

[OpenClaw](https://www.zdnet.com/article/openclaw-moltbot-clawdbot-5-reasons-viral-ai-agent-security-nightmare/) (formerly Clawdbot/Moltbot) went viral as an "AI that actually does things" — but security researchers have [flagged serious concerns](https://www.zdnet.com/article/openclaw-moltbot-clawdbot-5-reasons-viral-ai-agent-security-nightmare/). Anthill addresses each one through its R2 architecture:

| OpenClaw problem | Anthill approach |
|---|---|
| **Exposed credentials** — API keys and tokens leaked via misconfigured instances with no auth | Trust group model — colony.key never leaves the server, every API call requires a provisioned credential, join codes expire in 5 minutes |
| **No authentication** — hundreds of instances found open on the internet | Auth middleware on every endpoint — no credential = 401. Web dashboard requires join code to access |
| **Full system access** — grants broad permissions with no isolation | Each ANT has its own workspace directory. Plugins handle I/O — sentants never touch raw data. Access restricted via Telegram allow-list and trust group |
| **Prompt injection** — malicious content in emails/web pages can hijack the agent | Sentants only see small event payloads (IDs, codes). Untrusted content flows through the plugin data plane, never through the decision layer. The 256-byte event limit makes injection via events structurally impossible |
| **Malicious plugins/skills** — fake extensions and backdoored skills circulating | No plugin marketplace. Plugins are compiled into the binary from source. You audit what you run |
| **Viral fakes** — scam repos, fake tokens, impostor extensions | Self-hosted single binary. No app store, no extensions, no third-party skill downloads |

The fundamental difference: OpenClaw gives an AI agent broad access and hopes nothing goes wrong. Anthill uses R2's architecture to enforce separation — sentants make decisions, plugins handle data, trust groups control access. The security is structural, not aspirational.

## Security layers

Anthill implements defence in depth — multiple independent layers, each with a different job:

| Layer | What it does | Applies to |
|---|---|---|
| [Tailscale](https://tailscale.com/) | Network encryption (WireGuard) — only your devices can reach the server | Web dashboard |
| HTTPS | Transport encryption via Tailscale's automatic certificates | Web dashboard |
| Trust group | Authentication — devices join with one-time codes, every request verified | Web dashboard |
| HMAC envelopes | Message integrity — every WebSocket message signed with HMAC-SHA256, timestamped to prevent replay | Web dashboard |
| AES-256-GCM | Payload encryption (available, defence-in-depth) | Web dashboard |
| R2 architecture | Structural isolation — sentants see only 12-byte decisions, never raw content | All channels |

### The web dashboard is the most secure channel

The web dashboard carries all five security layers. **Use it for sensitive operations** — managing ANTS, editing configs, browsing files, handling credentials.

### Telegram and Slack are convenience channels

Telegram and Slack are useful for quick interactions from your phone, but they are weaker:

| | Web dashboard | Telegram | Slack |
|---|---|---|---|
| Network encryption | Tailscale (you control) | Telegram's TLS (they control) | Slack's TLS (they control) |
| Authentication | Trust group + join codes | Bot token + `allow` list | Bot/app tokens |
| Message signing | HMAC envelopes | None | None |
| Payload encryption | AES-256-GCM (available) | None | None |
| Identity verification | Device credential | Trust Telegram's chat ID | Trust Slack's user ID |
| Who can read your messages | Only you | Telegram (Meta-adjacent) | Salesforce |

**Risks of Telegram/Slack:**
- If the bot token leaks, anyone can message your ANT (mitigated by `allow` list on Telegram)
- Messages pass through third-party servers you don't control
- No message signing — you trust the platform's claimed sender identity
- Prompt injection is possible via forwarded messages or channel content

**Mitigations:**
- Use the `allow` list on Telegram to lock to your chat ID
- Use private Slack channels with restricted membership
- The sentant/plugin separation treats all incoming text as untrusted data (never as decisions)
- Use Telegram/Slack for quick tasks; use the web dashboard for anything sensitive

The long-term vision: R2-WIRE native transport replaces third-party platforms entirely — your phone's hive talks directly to the server's hive over Tailscale with full trust group envelopes. No middlemen.

See [Security](docs/security.md) for full details on trust group provisioning and device management.

## License

MIT OR Apache-2.0
