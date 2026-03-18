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

Only `mode` is required. Everything else has sensible defaults:

```toml
mode = "claude"
```

- **Name** defaults to the directory name
- **Telegram** disabled (web dashboard only)
- **Working directory** defaults to `~/.config/anthill/ants/<id>/working`
- **Permissions** automatically set based on mode (claude/ai = skip, raw = don't skip)
- **Memory and repos directories** always `memory/` and `repos/` within the working directory
- **Backups** disabled

### Full config

```toml
# Display name shown in the web dashboard (default: directory name)
name = "My ANT"

# Operating mode
mode = "claude"     # "claude" | "ai" | "raw"
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

### [claude]

For `mode = "claude"` (the main mode).

```toml
[claude]
# Working directory — where the ANT operates
# Auto-created if missing. Auto-initialised as a git repo.
# Default: ~/.config/anthill/ants/<id>/working
working_dir = "/path/to/workspace"

# Auto-commit workspace changes to git (hours, 0 = disabled)
backup_interval_hours = 6

# Push backups to a git remote (empty = local only)
backup_remote = "origin"

# System prompt — defines the ANT's personality
system_prompt = """\
You are a helpful programming assistant."""
```

The following are set automatically and don't normally need changing:

```toml
# Per-user memory files (always "memory" within working_dir)
memory_dir = "memory"

# Cloned git repos (always "repos" within working_dir, excluded from backup)
repos_dir = "repos"

# Auto-set based on mode: true for claude/ai, false for raw
skip_permissions = true
```

### [raw]

For `mode = "raw"` (persistent PTY).

```toml
[raw]
shell = "/bin/bash"
```

### [ai]

For `mode = "ai"` (NL → shell command → summarised output).

```toml
[ai]
model = "claude-sonnet-4-20250514"

# Anthropic API key (or set ANTHROPIC_API_KEY env var)
# Not needed for claude mode.
anthropic_api_key = "sk-ant-..."
```
