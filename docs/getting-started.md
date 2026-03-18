# Getting Started

Run a single ANT for testing or personal use.

## 1. Clone Anthill

```bash
git clone https://github.com/reality2-ai/anthill.git
cd anthill
```

## 2. Create your config

```bash
cp anthill-example.toml anthill.toml
```

Edit `anthill.toml`:

```toml
mode = "claude"

[telegram]
token = "YOUR_BOT_TOKEN_HERE"    # from @BotFather
allow = [YOUR_CHAT_ID]           # optional but recommended

[claude]
skip_permissions = true           # required for non-interactive mode

system_prompt = """\
You are a helpful programming assistant."""
```

## 3. Build and run

```bash
cargo run --release
```

## 4. Test it

Open Telegram, find your bot, and send a message. You should see:

1. A typing indicator ("..." bubble)
2. A "Thinking..." message
3. Claude's response with formatted code blocks

Try: "What's 2+2?" or "Write a Python hello world"

## 5. Useful commands

| Command | What it does |
|---|---|
| `/help` | List all commands |
| `/ants` | Show running workers |
| `/cancel` | Stop the current task |
| `/new` | Start a fresh conversation |

## Next steps

- [Production Setup](production-setup.md) — multiple ANTS, auto-start, systemd
- [Web Dashboard](web-dashboard.md) — browser-based interface
- [Memory & Workspaces](memory-and-workspaces.md) — persistent memory, git backups
