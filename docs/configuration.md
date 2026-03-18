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

The only required fields are `mode` and `skip_permissions`:

```toml
name = "My ANT"
mode = "claude"

[claude]
skip_permissions = true
```

Everything else has sensible defaults:
- **Telegram** — disabled (web dashboard only)
- **working_dir** — defaults to `~/.config/anthill/ants/<id>/working`
- **memory_dir** — `memory` (relative to working_dir)
- **repos_dir** — `repos` (relative to working_dir)
- **backup** — disabled

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

### [claude]

For `mode = "claude"` (the main mode).

```toml
[claude]
# Working directory — where the ANT operates
# Auto-created if missing. Auto-initialised as a git repo.
# Default: ~/.config/anthill/ants/<id>/working
working_dir = "/path/to/workspace"

# Per-user memory files (relative to working_dir)
memory_dir = "memory"

# Cloned git repos (relative to working_dir, excluded from backup)
repos_dir = "repos"

# REQUIRED — lets Claude run commands without interactive approval
skip_permissions = true

# Auto-commit workspace changes to git (hours, 0 = disabled)
backup_interval_hours = 6

# Push backups to a git remote (empty = local only)
backup_remote = "origin"

# System prompt — defines the ANT's personality
system_prompt = """\
You are a helpful programming assistant."""
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
