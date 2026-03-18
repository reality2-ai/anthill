# Anthill

AI-powered Telegram bots backed by Claude Code, built on the Reality2 (R2) sentant engine.

## What is Anthill?

Anthill lets you interact with Claude Code from your phone via Telegram. Send a message, get intelligent responses with full Claude Code capabilities — file editing, shell commands, code generation, git operations — all from a chat interface.

It supports multiple bots running simultaneously, each with their own personality, working directory, per-user memory, and conversation context. A built-in supervisor manages all bots and restarts them if they crash.

## Features

- **Claude Code via Telegram** — full tool use (file editing, shell commands, git) from your phone
- **Per-user memory** — each user gets a persistent memory file that Claude reads and updates
- **Conversation continuity** — uses `claude -p --continue` to maintain context across messages
- **Multiple bots** — supervisor manages any number of bots, each with their own config
- **Auto-restart** — crashed bots are restarted with exponential backoff
- **Systemd integration** — starts on boot, logs to journalctl
- **Typing indicator** — shows the Telegram "typing..." bubble while Claude is working
- **Markdown rendering** — headings, code blocks, bold, italic, links all render natively in Telegram
- **Git backups** — working directory auto-committed on a schedule
- **Access control** — restrict bots to specific Telegram chat IDs
- **Custom personalities** — each bot gets its own system prompt

## How it uses Reality2

Anthill is built on the R2 sentant engine — the same event-driven architecture used for IoT sensor networks. Every component is either a **sentant** (a deterministic state machine) or a **plugin** (a hardware/service adapter):

**Sentants** — pure FSMs that receive events and emit events. No I/O, no side effects:

| Sentant | States | Role |
|---|---|---|
| ClaudeCliSentant | Idle → Running → Idle | Coordinates the Claude Code conversation flow |
| AiSentant | Idle → Translating → Executing → Summarising → Idle | Coordinates NL→command→summary pipeline |

**Plugins** — service adapters that bridge external I/O into the event bus:

| Plugin | Role |
|---|---|
| TelegramPlugin | Bridges Telegram Bot API ↔ R2 events; handles message formatting, output batching, session routing |
| PtyPlugin | Manages pseudo-terminal lifecycle, input/output, chunking |
| ClaudeCliPlugin | Polls for completed Claude CLI responses |
| AiPlugin | Polls for Claude API responses (ai mode) |

Sentants only see events — they don't know about Telegram, PTY, or Claude. They make decisions (which state to enter, which events to emit) and the plugins handle the actual I/O. This separation keeps sentant logic testable and deterministic.

All communication flows through **R2 events** (CBOR-encoded, FNV-hashed names) on the **EventBus**. Sentants never perform I/O directly — they push **Actions** (send event, call plugin, delayed send) and the engine executes them.

**Side-channels** bypass the engine's 256-byte PayloadBuf limit for large data (Claude responses, terminal output). These use `tokio::mpsc` channels shared between plugins and sentants.

**Delayed sends** with replacement semantics (R2-SENTANT §3.1.5) implement debounce timers — one timer per (sentant, event_hash) pair. Setting a new timer replaces the pending one.

## Operating Modes

| Mode | Description | Use case |
|---|---|---|
| `claude` | Runs `claude -p` per message | Full Claude Code capabilities via Telegram |
| `ai` | NL → shell command → summarised output | Lightweight shell assistant (needs API key) |
| `raw` | Persistent PTY with raw terminal output | Direct shell access from Telegram |

## Prerequisites

### 1. Install Rust

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env
```

Verify:

```bash
cargo --version
```

### 2. Install Node.js (required for Claude Code)

```bash
# Arch/Manjaro
sudo pacman -S nodejs npm

# Ubuntu/Debian
sudo apt install nodejs npm

# macOS
brew install node
```

### 3. Install Claude Code

```bash
npm install -g @anthropic-ai/claude-code
```

Authenticate Claude Code — you need to do this once interactively before the bot can use it:

```bash
cd /tmp    # or any directory
claude
```

Follow the prompts to:
1. Log in with your Anthropic account (or API key)
2. Accept the workspace trust prompt
3. Verify it works — type "hello" and wait for a response
4. Exit with `/exit`

Then verify print mode works (this is how the bot runs Claude):

```bash
claude -p "Say hello"
```

You should see a plain text response with no TUI formatting. If this works, the bot will work.

### 4. Create a Telegram Bot

1. Open Telegram on your phone or desktop
2. Search for **@BotFather** and start a chat
3. Send `/newbot`
4. Choose a display name (e.g. "My Dev Assistant")
5. Choose a username — must end in `bot` (e.g. `my_dev_assistant_bot`)
6. BotFather replies with a **bot token** like `123456:ABC-DEF1234ghIkl-zyx57W2v1u123ew11`
7. Save this token — you'll need it for the config file

**Find your chat ID** (recommended, for access control):

1. Search for **@userinfobot** on Telegram and start a chat
2. It replies with your numeric chat ID (e.g. `123456789`)
3. Use this in the `allow` config to restrict who can talk to the bot

### 5. Install GitHub CLI (optional)

Only needed if you want Claude to clone private repos, create PRs, etc.

```bash
# Arch/Manjaro
sudo pacman -S github-cli

# Ubuntu/Debian
sudo apt install gh

# macOS
brew install gh

# Authenticate
gh auth login
```

Follow the prompts to authenticate via browser or token.

### 6. Clone the R2 repository

```bash
mkdir -p ~/Development/R2
cd ~/Development/R2
git clone https://github.com/reality2-ai/r2-core.git
cd r2-core
```

## Quick Start (Single Bot)

For testing or running a single bot without the supervisor.

### 1. Build

```bash
cd r2-core/tools/anthill
cargo build -p anthill --release
```

### 2. Configure

```bash
cp anthill-example.toml anthill.toml
```

Edit `anthill.toml`:

```toml
mode = "claude"

[telegram]
token = "YOUR_BOT_TOKEN_HERE"
allow = [YOUR_CHAT_ID]     # optional but recommended

[claude]
skip_permissions = true     # required for non-interactive mode
system_prompt = """\
You are a helpful programming assistant."""
```

### 3. Run

```bash
cargo run -p anthill --release
```

### 4. Test

Open Telegram, find your bot, and send a message. You should see:

1. A typing indicator ("..." bubble)
2. A "Thinking..." message
3. Claude's response with proper formatting

Try: "What's 2+2?" or "Write a Python hello world".

## Production Setup (Multi-Bot with Supervisor)

For running one or more bots as a system service that starts on boot.

### Directory Structure

```
~/.config/anthill/
├── supervisor.toml               # Supervisor settings
└── bots/
    ├── dev-assistant/
    │   └── bot.toml              # Bot config
    ├── ops-bot/
    │   └── bot.toml
    └── ...
```

Each bot also gets an auto-created working directory:

```
<working_dir>/                    # Set in bot.toml [claude] section
├── .git/                         # Auto-initialised for backups
├── memory/                       # Per-user memory files
│   ├── 123456789.md              # One file per Telegram chat ID
│   └── 987654321.md
└── repos/                        # Cloned git repositories
```

### Step-by-step Setup

#### 1. Install

```bash
cd ~/Development/R2/r2-core/tools/anthill
./install.sh
```

This will:
- Build a release binary
- Copy it to `/usr/local/bin/anthill`
- Create `~/.config/anthill/bots/`
- Generate a systemd service file for your user
- Install the service (requires sudo)

#### 2. Create your first bot

```bash
mkdir -p ~/.config/anthill/bots/dev-assistant
cp config-example/bots/dev-assistant/bot.toml \
   ~/.config/anthill/bots/dev-assistant/bot.toml
```

#### 3. Configure the bot

Edit `~/.config/anthill/bots/dev-assistant/bot.toml`:

```toml
mode = "claude"

[telegram]
# Your bot token from @BotFather
token = "123456:ABC-DEF1234ghIkl-zyx57W2v1u123ew11"

# Restrict access to your chat ID (strongly recommended for production)
allow = [123456789]

[claude]
# Where Claude works — files created here, memory stored here
working_dir = "/home/youruser/.config/anthill/bots/dev-assistant/working"

# Per-user memory and git repos (relative to working_dir)
memory_dir = "memory"
repos_dir = "repos"

# REQUIRED for non-interactive mode — lets Claude run commands
skip_permissions = true

# Auto-commit working directory changes every 6 hours
backup_interval_hours = 6

# Bot personality
system_prompt = """\
You are a helpful programming assistant. Always carefully consider what is \
being instructed and push back when appropriate. Be a devil's advocate — \
everything is a conjecture and should be subject to refutation. \
If you think an approach is wrong, say so and explain why. \
If you agree, explain your reasoning. Never just comply blindly."""
```

**Important notes:**

- Replace `token` with your actual bot token from @BotFather
- Replace `allow` with your actual Telegram chat ID
- Replace `/home/youruser/` with your actual home directory
- `skip_permissions = true` is required — without it, Claude will ask for permission to run commands but there's no way to approve them non-interactively
- Each bot needs its **own** Telegram bot token (create another via @BotFather)

#### 4. Add more bots (optional)

Repeat steps 2–3 for each bot:

```bash
# Create a second bot
mkdir -p ~/.config/anthill/bots/ops-bot
cp config-example/bots/dev-assistant/bot.toml \
   ~/.config/anthill/bots/ops-bot/bot.toml

# Edit with a different token, working_dir, and system_prompt
$EDITOR ~/.config/anthill/bots/ops-bot/bot.toml
```

Each bot can have a completely different personality, working directory, and access control.

#### 5. Start the supervisor

```bash
# Start now and enable on boot
sudo systemctl enable --now anthill
```

Or run manually without systemd:

```bash
anthill --supervise ~/.config/anthill
```

#### 6. Check status and logs

```bash
# Service status
sudo systemctl status anthill

# Live logs
journalctl -u anthill -f

# Recent logs
journalctl -u anthill --since "10 minutes ago"
```

#### 7. Restart after config changes

```bash
sudo systemctl restart anthill
```

#### 8. Update to a new version

```bash
cd ~/Development/R2/r2-core
git pull
cd tools/anthill
sudo systemctl stop anthill
./install.sh
sudo systemctl start anthill
```

**Note:** You must stop the service before installing because the binary can't be overwritten while it's running.

## Configuration Reference

### supervisor.toml

Controls how the supervisor manages bot processes.

```toml
# Directory containing bot configs (relative to supervisor.toml)
bots_dir = "bots"

# Auto-restart crashed bots
restart_on_crash = true

# Base delay in seconds before restarting (multiplied by consecutive failure count)
# 1st crash: 5s, 2nd: 10s, 3rd: 15s, etc.
restart_delay_secs = 5

# Give up after this many consecutive restarts (0 = never give up)
max_restarts = 10

# Override the anthill binary path (default: the running binary)
# relay_binary = "/usr/local/bin/anthill"
```

### bot.toml

Full configuration for a single bot instance.

```toml
# Operating mode: "raw", "ai", or "claude"
mode = "claude"

[telegram]
# Bot token from @BotFather (REQUIRED)
token = "123456:ABC-DEF..."

# Restrict to these Telegram chat IDs. Empty list = allow everyone.
# Strongly recommended for production to prevent unauthorised access.
# allow = [123456789, 987654321]

[claude]
# Working directory for Claude Code (REQUIRED for claude mode)
# Claude operates in this directory — all file operations happen here.
# Auto-created if it doesn't exist. Auto-initialised as a git repo.
working_dir = "/path/to/working/dir"

# Per-user memory files, relative to working_dir (default: "memory")
memory_dir = "memory"

# Directory for cloned repos, relative to working_dir (default: "repos")
repos_dir = "repos"

# Allow Claude to run shell commands without interactive approval (default: false)
# REQUIRED for claude mode — without this, Claude can't execute anything.
skip_permissions = true

# Auto-commit working directory changes to git (default: 0 = disabled)
# Set to number of hours between commits.
backup_interval_hours = 6

# Push backups to a git remote (default: "" = local only)
# backup_remote = "origin"

# System prompt injected into every Claude invocation (default: none)
# This defines the bot's personality and behaviour.
# system_prompt = "You are a helpful assistant."

[raw]
# Shell to spawn in raw mode (default: /bin/bash)
shell = "/bin/bash"

[ai]
# Claude model for AI mediation mode (default: claude-sonnet-4-20250514)
model = "claude-sonnet-4-20250514"

# Anthropic API key for AI mode. Or set ANTHROPIC_API_KEY env var.
# Not needed for claude mode (uses Claude Code's own auth).
# anthropic_api_key = "sk-ant-..."
```

## Telegram Commands

### Bot commands (handled locally, always available)

| Command | Description |
|---|---|
| `/help` or `/start` | Show available commands |
| `/usage` | Show session statistics (messages, chars, uptime, running tasks) |
| `/status` | Show all running tasks with IDs, message previews, and duration |
| `/cancel` | Cancel the most recent running task |
| `/cancel <id>` | Cancel a specific task by ID (shown in `/status`) |
| `/cancel all` | Cancel all running tasks |
| `/new` | Start a fresh conversation (resets Claude session context) |

### Claude Code commands (passed through to Claude)

These are Claude Code's own slash commands. They're sent as input to `claude -p` and Claude handles them:

| Command | Description |
|---|---|
| `/compact` | Condense conversation context |
| `/cost` | Show token/cost usage for the session |
| `/model` | Show or change the AI model |
| `/memory` | Manage Claude's memory files |
| `/clear` | Clear conversation history |

### Raw mode special keys

When running in `raw` mode (persistent PTY), these commands send control characters for navigating TUI applications:

| Send | Sends to PTY | Use case |
|---|---|---|
| `/enter` | `\r` | Confirm prompts |
| `/esc` | `\x1b` | Cancel/back |
| `/up` `/down` | Arrow keys | Navigate menus |
| `/left` `/right` | Arrow keys | Cursor movement |
| `/tab` | `\t` | Tab completion |
| `/ctrl-c` | `\x03` | Interrupt running command |
| `/ctrl-d` | `\x04` | EOF / exit shell |
| `/ctrl-z` | `\x1a` | Suspend process |
| `/space` | ` ` | Toggle selections |

Everything else in raw mode is sent as text followed by Enter.

## Conversation Persistence

Claude Code sessions are preserved across bot restarts. The bot always uses `claude -p --continue` which resumes the most recent session in the working directory. This means:

- **Within a session:** Full context of all previous messages
- **After a restart:** The previous session is resumed — Claude remembers what you were working on
- **After `/new`:** Starts a fresh session (previous sessions can still be resumed manually via Claude Code)

### Workspace awareness

On every invocation, Claude is automatically told about:
- The working directory structure (`memory/`, `repos/`)
- Where to clone repositories (always into `repos/`)
- That `repos/` is excluded from git backups
- The path to the current user's memory file

This means you don't need to explain the setup — Claude already knows where things go from the first message.

## Concurrent Tasks

The bot uses a **conductor/worker** architecture:

- **Conductor sentant** — always responsive. Receives your Telegram messages, dispatches work to concurrent Claude Code invocations, relays responses back. Handles `/help`, `/status`, `/cancel` locally without blocking.
- **Worker tasks** — each message spawns an independent `claude -p` process. Multiple tasks run in parallel.

This means you can:

- **Send a new message while another is running** — both execute concurrently
- **Check progress** with `/status` — see all running tasks, their IDs, and how long they've been running
- **Cancel tasks** with `/cancel` — abort the latest, a specific ID, or all at once
- **Ask Claude to spawn sub-agents** — Claude Code's built-in Agent tool works normally within each task

Example workflow:

```
You:    "Clone the repo github.com/example/project into repos/"
Bot:    Thinking...
You:    /status
Bot:    Running tasks (1):
        #1 — Clone the repo github.com/example/project... (45s)
You:    "While that's running, what's in my memory file?"
Bot:    Thinking...
Bot:    [response to memory question]
Bot:    [response to clone task]
```

## Per-User Memory

Each Telegram user gets a persistent memory file at `{working_dir}/memory/{chat_id}.md`.

Claude is instructed via the system prompt to:
- **Read** the memory file at the start of each conversation
- **Update** it when learning something worth remembering (preferences, project context, key decisions)
- **Clean up** outdated entries when noticed

Memory persists across:
- Individual messages within a session
- Bot restarts
- Conversation resets (`/new`)

This means if you tell the bot "I prefer Python over JavaScript" in one conversation, it will remember that in future conversations.

## Typing Indicator

While Claude is processing, the bot shows the Telegram "typing..." bubble (the animated dots that appear when someone is composing a message). This is sent every 4 seconds until the response is ready, so the user knows the bot is working even on long-running tasks.

A "Thinking..." text message is also sent immediately when a request is received.

## Backups

The working directory is a git repository that is automatically committed on a schedule. The `repos/` subdirectory is excluded from backups (cloned repos have their own git history).

### How it works

- **On first run:** The working directory is `git init`'d automatically with a `.gitignore` that excludes `repos/`
- **On schedule:** Every `backup_interval_hours`, all changes in the working directory (memory files, any files Claude creates) are staged and committed with a timestamped message
- **Optionally:** Pushed to a remote git repository

### Local-only backups

Just set the interval in `bot.toml`:

```toml
[claude]
backup_interval_hours = 6
```

This gives you local version history — you can `git log` and `git checkout` to see or roll back changes.

### Remote backups (GitHub)

To push backups to a private GitHub repository:

#### 1. Create a private repo on GitHub

```bash
# Using GitHub CLI (must be authenticated: gh auth login)
gh repo create your-org/anthill-mybot --private
```

#### 2. Add the remote to the bot's working directory

```bash
cd /path/to/your/working_dir    # e.g. ~/.config/anthill/bots/dev-assistant/working
git remote add origin https://github.com/your-org/anthill-mybot.git
```

#### 3. Do an initial push

```bash
git add -A
git commit -m "Initial commit"
git push -u origin master
```

#### 4. Configure the bot to push automatically

In `bot.toml`:

```toml
[claude]
backup_interval_hours = 6
backup_remote = "origin"
```

The backup task will now auto-push every 6 hours.

### What gets backed up

| Directory | Backed up? | Reason |
|---|---|---|
| `memory/` | Yes | Per-user persistent memory |
| Any files Claude creates | Yes | Working artifacts |
| `repos/` | No | Cloned repos have their own git history |
| `.gitignore` | Yes | Tracks backup exclusions |

### Viewing backup history

```bash
cd /path/to/working_dir
git log --oneline                # list all backup commits
git diff HEAD~1                  # see what changed in the last backup
git show HEAD:memory/12345.md    # view a specific file at last backup
```

## Markdown Rendering

Claude's markdown output is automatically converted to Telegram-compatible HTML:

| Markdown | Telegram rendering |
|---|---|
| `# Heading` | **BOLD UPPERCASE** (with spacing) |
| `## Heading` | **Bold** (with spacing) |
| `### Heading` | ***Bold italic*** |
| `#### Heading` | *Italic* |
| `**bold**` | **Bold** |
| `*italic*` | *Italic* |
| `` `inline code` `` | `Monospace` |
| ```` ```code block``` ```` | Selectable/copyable pre-formatted block |
| `- bullet` or `* bullet` | • Bullet point |
| `[text](url)` | Clickable link |
| `~~strikethrough~~` | ~~Strikethrough~~ |
| `---` | —————————— (em dash line) |

If HTML rendering fails (malformed output), the bot falls back to plain text.

## Troubleshooting

### Bot doesn't respond

1. Check the service is running: `sudo systemctl status anthill`
2. Check logs: `journalctl -u anthill -f`
3. Verify the bot token is correct in `bot.toml`
4. Make sure your chat ID is in the `allow` list (or remove `allow` to allow everyone)

### "Failed to run claude" error

1. Verify Claude Code is installed: `which claude`
2. Verify it works: `claude -p "hello"`
3. Check the PATH in the systemd service includes Claude's location
4. Make sure Claude Code is authenticated (run `claude` interactively once)

### "Permission denied" or Claude asking for approval

Set `skip_permissions = true` in `[claude]` section of `bot.toml`. Without this, `claude -p` can't run commands non-interactively.

### Systemd service fails to start

Check the journal for details:

```bash
journalctl -u anthill -n 50 --no-pager
```

Common issues:
- Binary not found: re-run `./install.sh`
- Config directory missing: `mkdir -p ~/.config/anthill/bots`
- No bots configured: create at least one bot directory with a `bot.toml`

### Binary can't be overwritten during install

Stop the service first:

```bash
sudo systemctl stop anthill
./install.sh
sudo systemctl start anthill
```

### Long responses get cut off

Telegram has a 4096-character message limit. The bot automatically splits long responses into multiple messages. If a message is still missing content, check the logs for errors.

## Security Considerations

- **Bot tokens are secrets** — store them in `bot.toml` which is not committed to git (`.gitignore` excludes config files with tokens)
- **`skip_permissions = true` gives Claude full shell access** — restrict who can talk to the bot using the `allow` list
- **The bot runs commands as your user** — it has the same permissions as your login. Don't run it as root.
- **Use `allow` in production** — without it, anyone who discovers your bot username can execute commands on your machine
- **Memory files may contain sensitive context** — they're stored in the working directory, not encrypted
