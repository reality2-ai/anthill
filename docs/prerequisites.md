# Prerequisites

Everything you need before installing Anthill.

> **Platform:** Anthill runs on **Linux** (systemd), **macOS** (launchd), and **FreeBSD/OpenBSD** (rc.d). Windows is not supported. The install script auto-detects your platform.
>
> **Technical level:** Setup requires comfort with the command line and Rust toolchain. This is an early-stage project aimed at developers.

## 1. [Rust](https://www.rust-lang.org/tools/install)

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env
cargo --version
```

## 2. AI backend (at least one)

Anthill supports multiple AI backends. Install whichever you want to use:

### [Claude Code](https://docs.anthropic.com/en/docs/claude-code)

Requires [Node.js](https://nodejs.org/) and an [Anthropic account](https://console.anthropic.com/).

```bash
# Install Node.js
# Arch/Manjaro: sudo pacman -S nodejs npm
# Ubuntu/Debian: sudo apt install nodejs npm
# macOS: brew install node

# Install Claude Code
npm install -g @anthropic-ai/claude-code

# Authenticate (once, interactively)
cd /tmp && claude
# Follow prompts to log in, then exit with /exit

# Verify
claude -p "Say hello"
```

### [OpenAI Codex](https://developers.openai.com/codex/cli)

Requires Node.js and an OpenAI account.

```bash
npm install -g @openai/codex

# Authenticate
codex
# Follow prompts to sign in, then exit

# Verify
codex exec "Say hello"
```

### [Ollama](https://ollama.com/) (local, coming soon)

```bash
# Linux
curl -fsSL https://ollama.com/install.sh | sh

# macOS
brew install ollama

# Pull a model
ollama pull llama3
```

**Important:** Authenticate each backend as the same user that will run Anthill.

## 3. [Telegram](https://telegram.org/) bot token (optional)

1. Install [Telegram](https://telegram.org/apps) on your phone or desktop
2. Message [**@BotFather**](https://t.me/BotFather)
3. Send `/newbot`
4. Choose a display name and username
5. Save the bot token
6. Each ANT that uses Telegram needs its own token

**Find your chat ID** (recommended for access control):
1. Message [**@userinfobot**](https://t.me/userinfobot) on Telegram
2. It replies with your numeric chat ID

## 4. [Tailscale](https://tailscale.com/) (recommended)

Creates a private encrypted network between your devices for the web dashboard.

```bash
# Arch/Manjaro
sudo pacman -S tailscale
sudo systemctl enable --now tailscaled
sudo tailscale up

# Ubuntu/Debian
curl -fsSL https://tailscale.com/install.sh | sh
sudo tailscale up

# macOS
brew install tailscale
# Or download from https://tailscale.com/download/mac
```

Install [Tailscale](https://tailscale.com/download) on your phone/tablet too.

## 5. [GitHub CLI](https://cli.github.com/) (optional)

Only needed if your ANT should clone private repos, create PRs, etc.

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
