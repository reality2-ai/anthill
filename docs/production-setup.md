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

This builds a release binary, copies it to `/usr/local/bin/anthill`, creates `~/.config/anthill/ants/`, generates a systemd service, and installs it.

### 2. Create your first ANT

```bash
mkdir -p ~/.config/anthill/ants/my-ant
cp config-example/ants/dev-assistant/ant.toml ~/.config/anthill/ants/my-ant/ant.toml
```

### 3. Configure it

Edit `~/.config/anthill/ants/my-ant/ant.toml`:

```toml
name = "My Dev ANT"
mode = "claude"

[telegram]
token = "YOUR_BOT_TOKEN"
allow = [YOUR_CHAT_ID]

[claude]
working_dir = "/home/youruser/Development/anthill-my-ant"
memory_dir = "memory"
repos_dir = "repos"
skip_permissions = true
backup_interval_hours = 6

system_prompt = """\
You are a helpful programming assistant."""
```

See [Configuration](configuration.md) for all options.

### 4. Add more ANTS (optional)

Each ANT needs its own directory, `ant.toml`, and Telegram bot token:

```bash
mkdir -p ~/.config/anthill/ants/another-ant
cp config-example/ants/dev-assistant/ant.toml ~/.config/anthill/ants/another-ant/ant.toml
# Edit with a different token, working_dir, and personality
```

### 5. Start

```bash
sudo systemctl enable --now anthill
```

Or run manually:

```bash
anthill --supervise ~/.config/anthill
```

### 6. Set up HTTPS (recommended)

See [Web Dashboard](web-dashboard.md) for Tailscale HTTPS setup.

### 7. Auto-start on boot

Both Anthill and the HTTPS proxy survive reboots:

```bash
# Anthill — enabled via systemd (done in step 5)
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

### 8. Check logs

```bash
journalctl -u anthill -f                    # live logs
journalctl -u anthill --since "10 min ago"  # recent
```

### 9. Restart after config changes

```bash
sudo systemctl restart anthill
```

### 10. Update to a new version

```bash
cd ~/path/to/anthill
git pull
sudo systemctl stop anthill
./install.sh
sudo systemctl start anthill
```

Stop before installing — the binary can't be overwritten while running.
