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

Edit `anthill.toml`. The minimal config is:

```toml
name = "My ANT"
mode = "claude"

[claude]
skip_permissions = true

system_prompt = """\
You are a helpful programming assistant."""
```

That's it — no Telegram token required. The ANT will be accessible via the web dashboard only, and the working directory defaults to `~/.config/anthill/ants/standalone/working`.

To also enable Telegram, add:

```toml
[telegram]
token = "YOUR_BOT_TOKEN_HERE"    # from @BotFather
allow = [YOUR_CHAT_ID]           # optional but recommended
```

To set a specific working directory:

```toml
[claude]
working_dir = "/home/youruser/my-ant-workspace"
```

If omitted, it defaults to `~/.config/anthill/ants/<id>/working`.

## 3. Build and run

```bash
cargo run --release
```

## 4. Test it

**Via web dashboard:** Open `http://localhost:3000` in your browser (single-bot mode also starts the web server).

**Via Telegram** (if configured): Find your bot and send a message. You should see:

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

- [Production Setup](production-setup.md) — multiple ANTS, supervisor, auto-start
- [Web Dashboard](web-dashboard.md) — browser-based interface, Tailscale HTTPS
- [Memory & Workspaces](memory-and-workspaces.md) — persistent memory, git backups
- [Configuration](configuration.md) — full reference for all settings
