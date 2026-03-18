# Anthill

A colony for **ANTS** — Autonomous iNTelligenceS.

Run AI agents powered by [Claude Code](https://docs.anthropic.com/en/docs/claude-code) on a server. Talk to them from anywhere — your phone, laptop, or tablet — via Telegram or a built-in web dashboard.

Each ANT has its own personality, workspace, persistent memory, and can run multiple tasks concurrently.

## Quick start

```bash
git clone https://github.com/reality2-ai/anthill.git
cd anthill
cp anthill-example.toml anthill.toml
# Edit anthill.toml — add your Telegram bot token
cargo run --release
```

Send a message to your bot on Telegram. Done.

For the full setup with multiple ANTS, web dashboard, and auto-start on boot, see the guides below.

## Documentation

| Guide | What it covers |
|---|---|
| [Prerequisites](docs/prerequisites.md) | Rust, Node.js, Claude Code, Telegram bot, Tailscale, GitHub CLI |
| [Getting Started](docs/getting-started.md) | Single ANT setup, first message, basic config |
| [Production Setup](docs/production-setup.md) | Multiple ANTS, supervisor, systemd, auto-start |
| [Web Dashboard](docs/web-dashboard.md) | Tailscale HTTPS, PWA install, cross-device history |
| [Configuration](docs/configuration.md) | Full reference for supervisor.toml and ant.toml |
| [Commands](docs/commands.md) | Telegram commands, Claude Code commands, raw mode keys |
| [Memory & Workspaces](docs/memory-and-workspaces.md) | Per-user memory, workspace structure, git backups |
| [Architecture](docs/architecture.md) | R2 sentant engine, events, plugins, concurrent tasks |
| [Troubleshooting](docs/troubleshooting.md) | Common issues and fixes |
| [Security](docs/security.md) | Access control, tokens, permissions |

## Features

- **Multiple ANTS** — each with its own personality, workspace, and Telegram bot
- **Web dashboard** — responsive UI, installable as PWA, accessible via Tailscale
- **Concurrent tasks** — send messages while others are running
- **Per-user memory** — persistent across conversations and restarts
- **Git-backed workspace** — auto-committed, optionally pushed to GitHub
- **Auto-restart** — supervisor manages ANTS with exponential backoff
- **Markdown rendering** — code blocks, headings, tables, links — in both Telegram and web UI

## License

MIT OR Apache-2.0
