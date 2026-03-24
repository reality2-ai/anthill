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

Anthill supports multiple AI backends. You can install one or more — Anthill will automatically select the best one based on your configured strategy.

### Cloud Backends

#### [Claude Code](https://docs.anthropic.com/en/docs/claude-code) (recommended)

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

#### [OpenAI Codex](https://developers.openai.com/codex/cli)

Requires Node.js and an OpenAI account.

```bash
npm install -g @openai/codex

# Authenticate
codex
# Follow prompts to sign in, then exit

# Verify
codex exec "Say hello"
```

#### [Google Gemini CLI](https://ai.google.dev/gemini-cli)

Requires Node.js and a Google account.

```bash
npm install -g @google/gemini-cli

# Authenticate
gemini
# Follow prompts to sign in

# Verify
gemini -p "Say hello"
```

#### [Grok CLI](https://github.com/superagent-ai/grok-cli) (xAI)

Requires Node.js and an [xAI API key](https://console.x.ai/).

```bash
npm install -g grok-cli

# Set API key (will be stored in macOS keychain on first run)
export XAI_API_KEY="your-api-key"

# Verify
grok -p "Say hello"
```

#### [DeepSeek CLI](https://github.com/holasoymalva/deepseek-cli)

Requires Node.js and a [DeepSeek API key](https://platform.deepseek.com/).

```bash
npm install -g run-deepseek-cli

# Set API key
export DEEPSEEK_API_KEY="your-api-key"

# Verify
deepseek -q "Say hello"
```

#### [OpenCode](https://opencode.ai/) (multi-provider)

OpenCode is an open-source AI coding agent that supports multiple providers.

```bash
npm install -g @opencode/cli

# Configure API keys in ~/.config/opencode/config.json
# or set environment variables

# Verify
opencode --version
```

### Local Backends

#### [Ollama](https://ollama.com/) (recommended for local)

Run AI models locally — no API key needed. Also provides embeddings for semantic knowledge graph search.

```bash
# Linux
curl -fsSL https://ollama.com/install.sh | sh

# macOS
brew install ollama

# Pull a chat model
ollama pull llama3.2

# Pull the embedding model (recommended — enables semantic search)
ollama pull nomic-embed-text
```

Without `nomic-embed-text`, knowledge graph retrieval falls back to keyword search.

#### [LM Studio](https://lmstudio.ai/)

Run AI models locally with a GUI or CLI.

```bash
# Download from https://lmstudio.ai/ or via Homebrew
brew install lm-studio

# Start LM Studio server (default: http://localhost:1234)
lm-studio

# Verify
curl http://localhost:1234/v1/models
```

### Backend Strategies

Anthill uses **strategies** to select which AI backend to use. Configure this in your ANT's settings or `ant.toml`:

| Strategy | Best For | Backend Priority |
|----------|----------|-----------------|
| **Cost Optimized** | Budget-conscious use | Ollama → DeepSeek → Gemini → Claude |
| **Capability Optimized** | Complex reasoning, research | Claude → Grok → Gemini → Codex |
| **Speed Optimized** | Quick responses | Ollama/LM Studio → DeepSeek → Gemini |
| **Balanced** | General use | Ollama → DeepSeek → Gemini → Claude → Grok |
| **Reliability Optimized** | Prioritize success rate | Based on recent success history |
| **Manual** | Full control | Your specified order |

For simple queries, Anthill uses faster/cheaper backends. For complex tasks (reasoning, analysis, coding), it switches to more capable models.

### Verify prerequisites

After installing, run the built-in diagnostic check:

```bash
anthill --doctor
```

This checks all installed AI backends, Rust, Git, Tailscale, config files, colony key, ANTs, devices, and service status. It is also available as a web API at `GET /api/doctor`.

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
