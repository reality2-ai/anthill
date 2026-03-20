# Configuration Reference

## supervisor.toml

Located at `~/.config/anthill/supervisor.toml`. Controls the supervisor process.

```toml
# Directory containing ANT configs (relative to this file)
ants_dir = "ants"

# Auto-restart crashed ANTS
restart_on_crash = true

# Base delay before restarting (multiplied by consecutive failure count)
# 1st crash: 5s, 2nd: 10s, 3rd: 15s, etc.
restart_delay_secs = 5

# Max consecutive restarts before giving up (0 = unlimited)
max_restarts = 10

# Web dashboard
http_port = 3000
http_bind = "0.0.0.0"
```

## ant.toml

Located at `~/.config/anthill/ants/<name>/ant.toml`. One per ANT.

### Minimal config

Everything has sensible defaults — an empty file works:

```toml
name = "My ANT"
```

- **Telegram** disabled (web dashboard only)
- **Working directory** defaults to `~/.config/anthill/ants/<id>/working`
- **Backend** defaults to Claude Code
- **Memory** — knowledge graph, episodic memory, per-user memory all auto-created
- **Backups** disabled

### Full config

```toml
# Display name shown in the web dashboard (default: directory name)
name = "My ANT"
```

### [telegram] (optional)

Omit this entire section for web-dashboard-only ANTS.

```toml
[telegram]
# Bot token from @BotFather
# If omitted, the ANT is only accessible via the web dashboard.
token = "123456:ABC-DEF..."

# Restrict to specific Telegram chat IDs
# Empty or omitted = allow everyone with the bot link
allow = [123456789, 987654321]
```

### [slack] (optional)

Omit this section if you don't use Slack. Uses Socket Mode (WebSocket) — no public URL needed.

```toml
[slack]
# Bot token from your Slack app (xoxb-...)
bot_token = "xoxb-..."

# App-level token for Socket Mode (xapp-...)
app_token = "xapp-..."
```

To set up:
1. Create a Slack app at [api.slack.com/apps](https://api.slack.com/apps)
2. Enable **Socket Mode** → generate an app-level token (`xapp-...`)
3. Add bot OAuth scopes: `chat:write`, `channels:history`, `groups:history`, `im:history`
4. Subscribe to bot events: `message.channels`, `message.groups`, `message.im`
5. Install the app to your workspace → copy the bot token (`xoxb-...`)
6. Invite the bot to a channel: `/invite @your-bot-name`

Telegram and Slack can both be active on the same ANT simultaneously.

### [claude] — AI and workspace

The `[claude]` section configures the AI backend and workspace (named for historical reasons — applies to all backends).

```toml
[claude]
# AI backends in priority order. Fallback on failure/rate limits.
# Supported: "claude", "codex", "ollama:<model>". Coming: "gemini".
# Ollama models use the "ollama:" prefix followed by the model name.
backends = ["claude"]
# Examples:
#   backends = ["ollama:llama3.2"]                      # Ollama only (local)
#   backends = ["claude", "ollama:llama3.2"]             # Claude with Ollama fallback
#   backends = ["ollama:llama3.2", "codex"]              # Ollama with Codex fallback

# Working directory — where the ANT operates
# Auto-created if missing. Auto-initialised as a git repo.
# Default: ~/.config/anthill/ants/<id>/working
working_dir = "/path/to/workspace"

# System prompt — defines the ANT's personality
system_prompt = """\
You are a helpful programming assistant."""

# Worker timeout — kill if no output for this many seconds (0 = no timeout)
worker_timeout_secs = 600

# Forward messages across channels: web ↔ Telegram ↔ Slack
sync_channels = false

# Auto-commit workspace to git (hours, 0 = disabled)
backup_interval_hours = 6

# Push backups to a git remote (empty = local only)
backup_remote = "origin"

# Encrypt memory/ and files/ in git backups (uses colony key)
encrypt_backups = false
```

The following are set automatically and don't normally need changing:

```toml
# Per-user memory files (always "memory" within working_dir)
memory_dir = "memory"

# Cloned git repos (always "repos" within working_dir, excluded from backup)
repos_dir = "repos"

# Always true — AI backends need command execution permission
skip_permissions = true
```
