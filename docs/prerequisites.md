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

## 2. AI backend

Anthill currently uses [Claude Code](https://docs.anthropic.com/en/docs/claude-code) as its AI backend. Support for [OpenAI](https://openai.com/), [Ollama](https://ollama.com/) (local/private), and other providers is coming soon.

### Claude Code setup

Requires [Node.js](https://nodejs.org/) and an [Anthropic account](https://console.anthropic.com/) (or Claude Pro/Team subscription).

```bash
# Install Node.js
# Arch/Manjaro
sudo pacman -S nodejs npm
# Ubuntu/Debian
sudo apt install nodejs npm
# macOS
brew install node
```

**Install:**

```bash
npm install -g @anthropic-ai/claude-code
```

**Authenticate** — must be done once interactively on the machine that will run Anthill:

```bash
cd /tmp
claude
```

1. Log in with your Anthropic account (or API key)
2. Accept the workspace trust prompt
3. Type "hello" and verify you get a response
4. Exit with `/exit`

**Verify print mode** — this is how Anthill runs Claude:

```bash
claude -p "Say hello"
```

You should see plain text output. If this works, Anthill will work.

**Important:** Authenticate as the same user that will run Anthill. If your systemd service runs as `youruser`, Claude Code must be authenticated under that account.

## 4. [Telegram](https://telegram.org/) bot token (optional)

Telegram is optional — ANTS can run with the web dashboard only. Skip this if you don't need Telegram access.

1. Install [Telegram](https://telegram.org/apps) on your phone or desktop
2. Message [**@BotFather**](https://t.me/BotFather)
3. Send `/newbot`
4. Choose a display name (e.g. "My Dev ANT")
5. Choose a username ending in `bot` (e.g. `my_dev_ant_bot`)
6. BotFather replies with a **bot token** — save it
7. Each ANT that uses Telegram needs its own token (create multiple via @BotFather)

**Find your chat ID** (recommended for access control):

1. Message [**@userinfobot**](https://t.me/userinfobot) on Telegram
2. It replies with your numeric chat ID (e.g. `123456789`)

## 5. [Tailscale](https://tailscale.com/) (recommended)

Tailscale creates a private encrypted network between your devices. Required for the web dashboard to be accessible from your phone/laptop.

```bash
# Arch/Manjaro
sudo pacman -S tailscale
sudo systemctl enable --now tailscaled
sudo tailscale up

# Ubuntu/Debian
curl -fsSL https://tailscale.com/install.sh | sh
sudo tailscale up
```

Install [Tailscale](https://tailscale.com/download) on your phone/tablet too.

After setup, your server gets a Tailscale IP (e.g. `100.91.6.128`) and a domain name (e.g. `myserver.tail12345.ts.net`).

## 6. [GitHub CLI](https://cli.github.com/) (optional)

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
