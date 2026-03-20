# Production Setup

Run one or more ANTS as a system service that starts on boot.

## Directory layout

```
~/.config/anthill/
├── supervisor.toml               # Supervisor settings
├── history/                      # Chat history (auto-created)
│   └── my-ant.jsonl
└── ants/                         # One subdirectory per ANT
    ├── my-ant/
    │   └── ant.toml
    └── another-ant/
        └── ant.toml
```

## Step by step

### 1. Install

```bash
cd anthill
./install.sh
```

This builds a release binary, copies it to `/usr/local/bin/anthill`, creates `~/.config/anthill/ants/`, and sets up the appropriate auto-start service for your platform (systemd on Linux, launchd on macOS, rc.d on BSD).

### 2. Install Ollama (optional — for local AI and semantic search)

```bash
# Linux
curl -fsSL https://ollama.com/install.sh | sh

# macOS
brew install ollama

# Pull models
ollama pull llama3.2            # chat model
ollama pull nomic-embed-text    # embedding model (enables semantic graph search)
```

Run `anthill --doctor` to verify all prerequisites are installed and configured.

### 3. Create your first ANT

```bash
mkdir -p ~/.config/anthill/ants/my-ant
cp config-example/ants/dev-assistant/ant.toml ~/.config/anthill/ants/my-ant/ant.toml
```

### 4. Configure it

Edit `~/.config/anthill/ants/my-ant/ant.toml`:

Minimal config (web dashboard only, no Telegram):

```toml
name = "My Dev ANT"
mode = "claude"

[claude]
skip_permissions = true

system_prompt = """\
You are a helpful programming assistant."""
```

The working directory defaults to `~/.config/anthill/ants/my-ant/working`.

To also enable Telegram access, add:

```toml
[telegram]
token = "YOUR_BOT_TOKEN"         # from @BotFather
allow = [YOUR_CHAT_ID]           # recommended
```

To set a custom working directory:

```toml
[claude]
working_dir = "/home/youruser/Development/anthill-my-ant"
```

See [Configuration](configuration.md) for all options.

### 5. Add more ANTS (optional)

Each ANT needs its own directory and `ant.toml`. Telegram is optional per ANT:

```bash
mkdir -p ~/.config/anthill/ants/another-ant
cp config-example/ants/dev-assistant/ant.toml ~/.config/anthill/ants/another-ant/ant.toml
# Edit with a different token, working_dir, and personality
```

### 6. Start

```bash
sudo systemctl enable --now anthill
```

Or run manually:

```bash
anthill --supervise ~/.config/anthill
```

### 7. Set up HTTPS (recommended)

See [Web Dashboard](web-dashboard.md) for Tailscale HTTPS setup.

### 8. Auto-start on boot

Both Anthill and the HTTPS proxy survive reboots:

```bash
# Anthill — enabled via install.sh (done in step 5)
sudo systemctl enable anthill

# HTTPS proxy — persists with --bg flag
sudo tailscale serve --bg http://localhost:3000
```

Verify:

```bash
systemctl is-enabled anthill
sudo systemctl status anthill
tailscale serve status
```

### 9. Check logs

```bash
journalctl -u anthill -f                    # live logs
journalctl -u anthill --since "10 min ago"  # recent
```

### 10. Restart after config changes

```bash
sudo systemctl restart anthill
```

### 11. Update to a new version

```bash
cd ~/path/to/anthill
git pull
sudo systemctl stop anthill
./install.sh
sudo systemctl start anthill
```

Stop before installing — the binary can't be overwritten while running.
