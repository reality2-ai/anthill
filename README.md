<p align="center">
  <img src="docs/logo.svg" alt="Anthill" width="600">
</p>

<p align="center">
  <em>A colony for <strong>ANTS</strong> — Autonomous iNTelligenceS</em>
</p>

---

Anthill runs AI agents on a server and lets you interact with them from any device — phone, laptop, tablet — via a built-in web dashboard, [Telegram](https://telegram.org/), or [Slack](https://slack.com/). It uses [Reality2](https://reality2.ai) (R2), an event-driven architecture where sentants (pure state machines) make decisions and plugins handle all I/O — connecting to AI backends like [Claude](https://www.anthropic.com/), [ChatGPT](https://openai.com/), or local models via [Ollama](https://ollama.com/).

Each ANT has its own personality, workspace, persistent memory, and can run multiple tasks concurrently. Access is secured by R2's trust group model — devices join the colony via one-time codes, and every request is authenticated with HMAC-signed envelopes. The web dashboard is accessed securely over [Tailscale](https://tailscale.com/).

> Anthill runs on **Linux**, **macOS**, and **FreeBSD**. The install script auto-detects your platform. See [Prerequisites](docs/prerequisites.md) for setup details.

## Quick start

```bash
git clone https://github.com/reality2-ai/anthill.git
cd anthill
./install.sh                     # builds, installs binary, sets up service
anthill --join-code              # prints a code (expires in 5 min)
# Open http://localhost:3000 (or your Tailscale hostname) → enter the code
# Create your first ANT from the web dashboard (+  button)
```

## How it works

[Reality2](https://reality2.ai) (R2) is a software stack for wearables, IoT, and autonomous agents. Anthill is the first production R2 application outside hardware — proving the architecture works for AI agents, not just sensors.

**Sentants** are pure state machines — they receive events, make decisions, emit actions. No I/O, no side effects. Given the same events, they always produce the same output.

**Plugins** are service adapters — they bridge external systems (AI backends, Telegram, Slack, web servers) into the R2 event bus. All I/O happens here.

**Events carry decisions** (< 256 bytes). **Plugins carry data** (unlimited). This separation is enforced by design — the 256-byte limit ensures sentants never see raw content, making prompt injection via the event bus structurally impossible.

![Anthill Architecture](https://mermaid.ink/img/Z3JhcGggTFIKICAgIHN1YmdyYXBoIFZpZXdlcnMKICAgICAgICBQaG9uZVtQaG9uZSBicm93c2VyXQogICAgICAgIExhcHRvcFtMYXB0b3AgYnJvd3Nlcl0KICAgICAgICBUR1tUZWxlZ3JhbSBhcHBdCiAgICAgICAgU0xbU2xhY2sgYXBwXQogICAgZW5kCgogICAgc3ViZ3JhcGggU2VydmVyW1NlcnZlciAtIHRoZSBRdWVlbl0KICAgICAgICBzdWJncmFwaCBQbHVnaW5zW1BsdWdpbnMgLSBhbGwgSS9PXQogICAgICAgICAgICBXUFtXZWJQbHVnaW5dCiAgICAgICAgICAgIFRQW1RlbGVncmFtUGx1Z2luXQogICAgICAgICAgICBTUFtTbGFja1BsdWdpbl0KICAgICAgICAgICAgQVBbQUlQbHVnaW5dCiAgICAgICAgZW5kCiAgICAgICAgc3ViZ3JhcGggRlNNc1tTZW50YW50cyAtIHB1cmUgRlNNc10KICAgICAgICAgICAgQ1NbQ29uZHVjdG9yXQogICAgICAgIGVuZAogICAgICAgIE9MW09sbGFtYV06OjpmdXR1cmUKICAgIGVuZAoKICAgIHN1YmdyYXBoIEV4dGVybmFsW0V4dGVybmFsIEFJXQogICAgICAgIENsYXVkZVtDbGF1ZGVdCiAgICAgICAgT0FJW09wZW5BSV06OjpmdXR1cmUKICAgIGVuZAoKICAgIFBob25lIDwtLT58dHJ1c3QgZ3JvdXAgYXV0aHwgV1AKICAgIExhcHRvcCA8LS0+fHRydXN0IGdyb3VwIGF1dGh8IFdQCiAgICBURyA8LS0+IFRQCiAgICBTTCA8LS0+IFNQCiAgICBXUCA8LS0+fGV2ZW50c3wgQ1MKICAgIFRQIDwtLT58ZXZlbnRzfCBDUwogICAgU1AgPC0tPnxldmVudHN8IENTCiAgICBDUyA8LS0+fHBsdWdpbl9jYWxsfCBBUAogICAgQVAgPC0tPnxkYXRhIHBsYW5lfCBUUAogICAgQVAgPC0tPnxkYXRhIHBsYW5lfCBTUAogICAgQVAgPC0tPnxkYXRhIHBsYW5lfCBXUAogICAgQVAgPC0tPiBDbGF1ZGUKICAgIEFQIC0uLXxjb21pbmcgc29vbnwgT0FJCiAgICBBUCAtLi18Y29taW5nIHNvb258IE9MCgogICAgY2xhc3NEZWYgZnV0dXJlIHN0cm9rZS1kYXNoYXJyYXk6IDUgNSxvcGFjaXR5OjAuNQ==)

| Sentants (pure FSMs — zero I/O) | Plugins (all I/O) |
|---|---|
| Conductor — dispatches, routes, commands | AIPlugin — Claude (+ OpenAI, Ollama coming) |
| AiSentant — NL→command→summary pipeline | TelegramPlugin — Bot API, message classification |
| ChunkerSentant — output batching decisions | SlackPlugin — Socket Mode, message routing |
| TerminalSentant — PTY lifecycle | WebPlugin — dashboard, WebSocket, file browser |
| TelegramSentant — session routing | ChunkerPlugin, PtyPlugin — raw mode I/O |

## Features

- **Multiple ANTS** — each with its own personality, workspace, and optional Telegram/Slack
- **Web dashboard** — responsive, installable as PWA, accessible via Tailscale
- **Trust group security** — join codes, device provisioning, HMAC-signed messages
- **Concurrent tasks** — send messages while others are running; `/ants` to check progress
- **Real-time progress** — see what each worker is doing (tool use, agent spawns)
- **Per-user memory** — persistent across conversations and restarts
- **File browser** — browse workspace, upload/download files, preview images and code
- **Git-backed workspace** — auto-committed on schedule, optionally pushed to GitHub
- **ANT management** — create, configure, delete ANTS from the dashboard
- **Device management** — provision and revoke devices from the dashboard
- **Cross-channel sync** — messages forwarded between web, Telegram, and Slack (opt-in)
- **Markdown rendering** — code blocks, headings, tables, links in both Telegram and web
- **Auto-restart** — supervisor manages ANTS with exponential backoff
- **Cross-platform** — Linux (systemd), macOS (launchd), FreeBSD (rc.d)

## Documentation

| Guide | What it covers |
|---|---|
| [Prerequisites](docs/prerequisites.md) | Rust, AI backend, Telegram/Slack, Tailscale |
| [Getting Started](docs/getting-started.md) | Install, create your first ANT, send a message |
| [Production Setup](docs/production-setup.md) | Multiple ANTS, supervisor, auto-start |
| [Web Dashboard](docs/web-dashboard.md) | Tailscale HTTPS, PWA, cross-device history |
| [Configuration](docs/configuration.md) | Full reference for supervisor.toml and ant.toml |
| [Commands](docs/commands.md) | Telegram and dashboard commands |
| [Memory & Workspaces](docs/memory-and-workspaces.md) | Per-user memory, git backups |
| [Architecture](docs/architecture.md) | R2 sentant engine, events, plugins |
| [Troubleshooting](docs/troubleshooting.md) | Common issues and fixes |
| [Security](docs/security.md) | Trust groups, device provisioning, access control |

## Security

Anthill implements defence in depth:

| Layer | What it does | Applies to |
|---|---|---|
| [Tailscale](https://tailscale.com/) | Network encryption (WireGuard) — only your devices can reach the server | Web dashboard |
| HTTPS | Transport encryption via Tailscale's automatic certificates | Web dashboard |
| Trust group | Authentication — devices join with one-time codes, every request verified | Web dashboard |
| HMAC envelopes | Message integrity — every WebSocket message signed, timestamped against replay | Web dashboard |
| AES-256-GCM | Payload encryption (available, defence-in-depth) | Web dashboard |
| R2 architecture | Structural isolation — sentants see only 12-byte decisions, never raw content | All channels |

The **web dashboard** carries all six layers — use it for sensitive operations. **Telegram and Slack** are convenience channels with weaker security (third-party TLS, no message signing). See [Security](docs/security.md) for details.

## Why not just use OpenClaw?

[OpenClaw](https://www.zdnet.com/article/openclaw-moltbot-clawdbot-5-reasons-viral-ai-agent-security-nightmare/) went viral as an "AI that actually does things" — but security researchers [flagged serious concerns](https://www.zdnet.com/article/openclaw-moltbot-clawdbot-5-reasons-viral-ai-agent-security-nightmare/). Anthill addresses each one:

| OpenClaw problem | Anthill approach |
|---|---|
| **Exposed credentials** — leaked API keys, no auth | Trust group — colony.key stays on server, join codes expire in 5 min |
| **No authentication** — instances open on the internet | Auth middleware on every endpoint, HMAC-signed WebSocket |
| **Full system access** — broad permissions, no isolation | Per-ANT workspaces, sentant/plugin separation |
| **Prompt injection** — malicious content hijacks the agent | 256-byte event limit, untrusted content never reaches sentants |
| **Malicious plugins** — fake extensions, backdoored skills | No marketplace — plugins compiled from source |
| **Viral fakes** — scam repos, impostor extensions | Self-hosted single binary, no third-party downloads |

The security is structural (enforced by architecture), not aspirational (hoping users configure it right).

## License

MIT OR Apache-2.0
