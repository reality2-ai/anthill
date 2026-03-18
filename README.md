# Anthill

A colony for **ANTS** — Autonomous iNTelligenceS.

Anthill lets you run AI agents (powered by Claude Code) on a server and interact with them from anywhere — your phone, laptop, tablet — via Telegram or a built-in web dashboard. Each ANT has its own personality, workspace, persistent memory, and conversation context.

## Features

- **Multiple ANTS** — run any number of AI agents, each with their own config, personality, and workspace
- **Telegram interface** — talk to your ANTS from your phone with typing indicators, markdown rendering, and code blocks
- **Web dashboard** — responsive UI accessible from any device on your network (PWA installable on mobile/desktop)
- **Concurrent tasks** — send messages while others are running; check progress with `/ants`, cancel with `/cancel`
- **Per-user memory** — each user gets a persistent memory file that the ANT reads and updates across conversations
- **Conversation continuity** — sessions survive restarts via `claude -p --continue`
- **Git-backed workspace** — working directory auto-committed on a schedule, with optional push to GitHub
- **Supervisor** — manages all ANTS, auto-restarts crashed ones with exponential backoff
- **Systemd integration** — starts on boot, logs to journalctl
- **Access control** — restrict each ANT to specific Telegram chat IDs
- **Tailscale support** — access the web dashboard from any device on your Tailscale network

## How It Works

Each ANT runs Claude Code in print mode (`claude -p`) per message. This gives full Claude Code capabilities — file editing, shell commands, code generation, git operations — with clean text output suitable for chat interfaces. No TUI, no ANSI escape codes.

The architecture uses the Reality2 (R2) sentant engine:
- **Sentants** (pure state machines) make decisions — which messages to dispatch, when to cancel, how to route responses
- **Plugins** handle I/O — Telegram API, Claude CLI processes, WebSocket connections
- Events carry decisions (small, < 256 bytes). Plugins carry data (unlimited). This separation keeps the logic testable and deterministic.

## Prerequisites

### 1. [Rust](https://www.rust-lang.org/tools/install)

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env
cargo --version
```

### 2. [Node.js](https://nodejs.org/) (required for Claude Code)

```bash
# Arch/Manjaro
sudo pacman -S nodejs npm

# Ubuntu/Debian
sudo apt install nodejs npm

# macOS
brew install node
```

### 3. [Claude Code](https://docs.anthropic.com/en/docs/claude-code)

You need an [Anthropic account](https://console.anthropic.com/) or a Claude Pro/Team subscription.

Install the CLI:

```bash
npm install -g @anthropic-ai/claude-code
```

**Authenticate Claude Code** — this must be done once interactively on the machine where Anthill will run:

```bash
cd /tmp
claude
```

Follow the prompts to:
1. Log in with your Anthropic account (or API key)
2. Accept the workspace trust prompt
3. Verify it works — type "hello" and wait for a response
4. Exit with `/exit`

**Verify print mode** — this is how Anthill runs Claude:

```bash
claude -p "Say hello"
```

You should see a plain text response. If this works, Anthill will work.

**Important:** Claude Code must be authenticated as the same user that will run Anthill. If you install as a systemd service running as `roycdavies`, Claude Code must be authenticated under that user account.

### 4. [Telegram](https://telegram.org/) bot token

1. Install [Telegram](https://telegram.org/apps) on your phone or desktop
2. Message [**@BotFather**](https://t.me/BotFather)
3. Send `/newbot`
4. Choose a display name (e.g. "My Dev ANT")
5. Choose a username ending in `bot` (e.g. `my_dev_ant_bot`)
6. BotFather replies with a **bot token** — save it for the config file
7. Each ANT needs its own bot token (create multiple via @BotFather)

**Find your chat ID** (recommended for access control):
1. Message [**@userinfobot**](https://t.me/userinfobot) on Telegram
2. It replies with your numeric chat ID (e.g. `123456789`)

### 5. [Tailscale](https://tailscale.com/) (recommended for web dashboard)

[Tailscale](https://tailscale.com/) creates a private encrypted network between your devices. Install it on both the server running Anthill and the devices you want to access it from.

```bash
# Arch/Manjaro
sudo pacman -S tailscale
sudo systemctl enable --now tailscaled
sudo tailscale up

# Ubuntu/Debian
curl -fsSL https://tailscale.com/install.sh | sh
sudo tailscale up

# Android/iOS — install from your app store:
# https://tailscale.com/download
```

After setup, your server gets a Tailscale IP (e.g. `100.91.6.128`). The Anthill web dashboard is accessible at `http://<tailscale-ip>:3000` from any device on your Tailscale network — phone, laptop, tablet.

**Why Tailscale?** The web dashboard doesn't require a login — if you can reach the URL, you're trusted. Tailscale ensures only your devices can reach it. No port forwarding, no public exposure.

### 6. [GitHub CLI](https://cli.github.com/) (optional)

Only needed if you want your ANT to clone private repos, create PRs, etc.

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

## Quick Start (Single ANT)

For testing or running a single ANT without the supervisor.

```bash
# Clone
git clone https://github.com/reality2-ai/anthill.git
cd anthill

# Configure
cp anthill-example.toml anthill.toml
# Edit anthill.toml — set telegram.token, mode = "claude", claude.skip_permissions = true

# Build and run
cargo run --release
```

Send a message to your bot on Telegram. You should see a typing indicator, "Thinking...", then Claude's response.

## Production Setup

For running one or more ANTS as a system service that starts on boot.

### Directory layout

```
~/.config/anthill/
├── supervisor.toml               # Supervisor settings (port, restart policy)
├── history/                      # Chat history (auto-created)
│   ├── alfred.jsonl              # Per-ANT, loaded by web dashboard
│   └── hine.jsonl
└── ants/                         # One subdirectory per ANT
    ├── alfred/
    │   └── ant.toml              # ANT config
    └── hine/
        └── ant.toml
```

Each ANT also has a **working directory** (set in `ant.toml`):

```
<working_dir>/                    # e.g. ~/Development/anthill-alfred
├── .git/                         # Auto-initialised for backups
├── .gitignore                    # Auto-created: excludes repos/
├── memory/                       # Per-user memory files
│   ├── 123456789.md              # One file per Telegram chat ID
│   └── 0.md                     # Web dashboard user (chat_id 0)
└── repos/                        # Cloned git repositories (excluded from backup)
```

### Step-by-step setup

#### 1. Clone and install

```bash
git clone https://github.com/reality2-ai/anthill.git
cd anthill
./install.sh
```

This will:
- Build a release binary
- Copy it to `/usr/local/bin/anthill`
- Create `~/.config/anthill/ants/`
- Generate a systemd service file for your user
- Install the service

#### 2. Create your first ANT

```bash
mkdir -p ~/.config/anthill/ants/my-ant
cp config-example/ants/dev-assistant/ant.toml ~/.config/anthill/ants/my-ant/ant.toml
```

#### 3. Configure the ANT

Edit `~/.config/anthill/ants/my-ant/ant.toml`:

```toml
# Display name shown in the web dashboard.
name = "My Dev ANT"

mode = "claude"

[telegram]
# Bot token from @BotFather
token = "123456:ABC-DEF1234ghIkl-zyx57W2v1u123ew11"

# Restrict to your chat ID (strongly recommended)
allow = [123456789]

[claude]
# Where the ANT works — all files, memory, repos go here
working_dir = "/home/youruser/Development/anthill-my-ant"

# Per-user memory and repos (relative to working_dir)
memory_dir = "memory"
repos_dir = "repos"

# REQUIRED — lets Claude run commands without interactive approval
skip_permissions = true

# Auto-commit workspace changes every 6 hours
backup_interval_hours = 6

# ANT personality
system_prompt = """\
You are a helpful programming assistant. Always carefully consider what is \
being instructed and push back when appropriate. Be a devil's advocate — \
everything is a conjecture and should be subject to refutation."""
```

**Key points:**
- `skip_permissions = true` is required — without it, Claude can't run commands non-interactively
- Each ANT needs its own Telegram bot token
- The `name` field is what shows in the web dashboard sidebar (falls back to directory name)
- `working_dir` should be a dedicated directory for this ANT — it becomes a git repo

#### 4. Configure the supervisor (optional)

Edit `~/.config/anthill/supervisor.toml`:

```toml
ants_dir = "ants"
restart_on_crash = true
restart_delay_secs = 5
max_restarts = 10

# Web dashboard
http_port = 3000
http_bind = "0.0.0.0"
```

#### 5. Add more ANTS (optional)

```bash
mkdir -p ~/.config/anthill/ants/another-ant
cp config-example/ants/dev-assistant/ant.toml ~/.config/anthill/ants/another-ant/ant.toml
# Edit ant.toml — different token, working_dir, personality
```

#### 6. Start

```bash
# Via systemd (auto-starts on boot)
sudo systemctl enable --now anthill

# Or manually
anthill --supervise ~/.config/anthill
```

#### 7. Access the web dashboard

**Option A — HTTP (simple):**

Open `http://<your-tailscale-ip>:3000` in any browser on your Tailscale network.

**Option B — HTTPS via Tailscale (recommended):**

Tailscale can provision a valid HTTPS certificate and proxy to Anthill automatically:

```bash
# Find your machine's Tailscale domain
tailscale status --self
# e.g. alfred.tail12345.ts.net

# Set up HTTPS proxy to Anthill
tailscale serve https / http://localhost:3000
```

Now access the dashboard at `https://alfred.tail12345.ts.net` with a valid certificate. This is required for PWA installation on some browsers and enables secure WebSocket (`wss://`).

To make this permanent (survives reboots):

```bash
tailscale serve --bg https / http://localhost:3000
```

To check what's being served:

```bash
tailscale serve status
```

**Features:**
- Sidebar listing all ANTS with status indicators
- Chat interface with markdown rendering and code blocks
- Live task panel showing running workers
- Full conversation history (persists across devices and restarts)

**Install as a PWA** (works best over HTTPS):
- **Android:** Chrome menu → "Add to Home screen"
- **iOS:** Safari share → "Add to Home Screen"
- **Linux:** Chrome menu → "Install Anthill"

#### 8. Updating

```bash
cd ~/path/to/anthill
git pull
sudo systemctl stop anthill
./install.sh
sudo systemctl start anthill
```

Stop the service before installing — the binary can't be overwritten while running.

## Commands

### Anthill commands (handled locally)

| Command | Description |
|---|---|
| `/help` or `/start` | Show available commands |
| `/ants` | Show running workers and what they're working on |
| `/usage` | Show session statistics |
| `/cancel` | Cancel the most recent worker |
| `/cancel <id>` | Cancel a specific worker by ID |
| `/cancel all` | Cancel all running workers |
| `/new` | Start a fresh conversation |

### Claude Code commands (passed through)

| Command | Description |
|---|---|
| `/compact` | Condense conversation context |
| `/cost` | Show token/cost usage |
| `/model` | Show or change the AI model |
| `/memory` | Manage Claude's memory files |
| `/clear` | Clear conversation history |

### Raw mode special keys

For `mode = "raw"` (persistent PTY):

| Send | Does |
|---|---|
| `/enter` | Confirm |
| `/esc` | Cancel |
| `/up` `/down` `/left` `/right` | Arrow keys |
| `/tab` | Tab completion |
| `/ctrl-c` | Interrupt |
| `/ctrl-d` | EOF/exit |
| `/space` | Space/toggle |

## Per-User Memory

Each user (identified by Telegram chat ID, or `0` for web dashboard) gets a persistent memory file at `{working_dir}/memory/{chat_id}.md`.

Claude is instructed via the system prompt to:
- **Read** the file at the start of each conversation
- **Update** it when learning something worth remembering
- **Clean up** outdated entries

Memory persists across messages, restarts, and conversation resets (`/new`). If you tell the ANT "I prefer Python over JavaScript" today, it will remember next week.

The memory files are included in the git-backed workspace, so they're version-controlled and backed up.

## Workspace and Backups

Each ANT's working directory is automatically initialised as a git repository. This provides version history for everything the ANT creates or modifies.

### Automatic backups

Set `backup_interval_hours` in `ant.toml`:

```toml
[claude]
backup_interval_hours = 6      # commit every 6 hours (0 = disabled)
```

### Remote backups (GitHub)

To push backups to a private repo:

```bash
# Create the repo
gh repo create your-org/anthill-my-ant --private

# Add remote to the working directory
cd /path/to/working_dir
git remote add origin https://github.com/your-org/anthill-my-ant.git
git add -A && git commit -m "Initial commit"
git push -u origin master

# Enable in ant.toml
# backup_remote = "origin"
```

### What gets backed up

| Path | Backed up | Reason |
|---|---|---|
| `memory/` | Yes | Per-user persistent memory |
| Files Claude creates | Yes | Working artifacts, code, configs |
| `repos/` | **No** | Cloned repos have their own git history |

### Viewing history

```bash
cd /path/to/working_dir
git log --oneline                # all backup commits
git diff HEAD~1                  # last changes
git show HEAD:memory/12345.md    # specific file at last backup
```

## Conversation Persistence

Sessions survive restarts. The ANT always uses `claude -p --continue` to resume the most recent Claude Code session in its working directory.

On every invocation, Claude is automatically told about:
- The workspace structure (`memory/`, `repos/`)
- Where to clone repositories (always into `repos/`)
- That `repos/` is excluded from git backups
- The current user's memory file path

You don't need to explain the setup — the ANT already knows from the first message.

## Concurrent Tasks

Each ANT uses a **conductor/worker** architecture:

- **Conductor** — always responsive, dispatches messages, handles `/ants` and `/cancel`
- **Workers** — each message spawns an independent Claude Code process running in parallel

Send multiple messages — they all execute concurrently. Use `/ants` to see what's running, `/cancel` to stop tasks.

## Web Dashboard

The built-in web dashboard runs alongside the supervisor on port 3000 (configurable in `supervisor.toml`).

**Access:** `http://<server-ip>:3000` — best over Tailscale for security.

**Features:**
- Sidebar listing all ANTS with live status
- Chat interface with full markdown rendering (headings, code blocks with copy buttons, tables, lists, links)
- Live task panel with cancel buttons and timers
- Chat history loaded from disk (persists across sessions and devices)
- Auto-reconnecting WebSocket
- Responsive layout (mobile and desktop)
- PWA installable as a home screen app

**Chat history** is stored as JSONL files at `~/.config/anthill/history/{ant-id}.jsonl`. Messages from all sources (Telegram, web UI) are recorded centrally, so opening the dashboard from a new device shows the full conversation history.

## Configuration Reference

### supervisor.toml

```toml
ants_dir = "ants"              # Where ANT configs live
restart_on_crash = true         # Auto-restart crashed ANTS
restart_delay_secs = 5          # Base delay (× restart count for backoff)
max_restarts = 10               # Max consecutive restarts (0 = unlimited)
http_port = 3000                # Web dashboard port
http_bind = "0.0.0.0"          # Bind address (0.0.0.0 = all interfaces)
```

### ant.toml

```toml
name = "My ANT"                 # Display name (default: directory name)
mode = "claude"                 # "raw" | "ai" | "claude"

[telegram]
token = "BOT_TOKEN"             # From @BotFather (REQUIRED)
# allow = [123456789]           # Restrict to specific chat IDs

[claude]
working_dir = "/path/to/work"   # ANT's workspace (REQUIRED)
memory_dir = "memory"           # Per-user memory (relative)
repos_dir = "repos"             # Git repos (relative, excluded from backup)
skip_permissions = true          # REQUIRED for claude mode
# backup_interval_hours = 6     # Auto-commit interval (0 = disabled)
# backup_remote = "origin"      # Push backups to remote
# system_prompt = "..."         # ANT personality

[raw]
shell = "/bin/bash"

[ai]
model = "claude-sonnet-4-20250514"
# anthropic_api_key = "sk-ant-..."
```

## Troubleshooting

### ANT doesn't respond to Telegram messages

1. `sudo systemctl status anthill` — is it running?
2. `journalctl -u anthill -f` — check logs
3. Verify the bot token in `ant.toml`
4. Check your chat ID is in the `allow` list (or remove `allow`)

### "Failed to run claude" error

1. `which claude` — is Claude Code installed?
2. `claude -p "hello"` — does it work directly?
3. Check PATH in the systemd service includes `~/.local/bin` (where `claude` typically lives)
4. Run `claude` interactively once to authenticate

### Claude asks for permission (can't execute)

Set `skip_permissions = true` in `[claude]` section of `ant.toml`.

### Web dashboard not loading

1. Check the supervisor is running and the web server started (look for "Web server listening on" in logs)
2. Verify you can reach the server IP (try `ping <ip>`)
3. If using Tailscale, verify both devices are connected: `tailscale status`
4. Check the port isn't blocked: `curl http://<ip>:3000`

### Chat history missing after rename

History is keyed by directory name (the ANT's stable ID), not the display name. If you renamed the directory under `ants/`, the history file won't match. Rename `~/.config/anthill/history/<old-name>.jsonl` to match.

### Binary can't be overwritten during install

```bash
sudo systemctl stop anthill
./install.sh
sudo systemctl start anthill
```

## Security

- **Bot tokens are secrets** — `ant.toml` files contain tokens and should not be committed to git
- **`skip_permissions = true`** gives Claude full shell access as your user — restrict access via the `allow` list
- **The web dashboard has no authentication** — rely on Tailscale (or a firewall) to restrict who can reach port 3000
- **Don't run as root** — Anthill runs commands as whatever user the service runs under
- **Memory files** may accumulate sensitive context — they're stored in the working directory, backed up to git, but not encrypted
