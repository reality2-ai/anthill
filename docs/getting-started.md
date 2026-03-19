# Getting Started

## 1. Clone and install

```bash
git clone https://github.com/reality2-ai/anthill.git
cd anthill
./install.sh
```

This builds the binary, installs it, creates `~/.config/anthill/`, and starts the service.

## 2. Generate a join code

```bash
anthill --join-code
```

This prints a code that expires in 5 minutes. You'll need it to connect from your browser.

## 3. Open the dashboard

Open `http://localhost:3000` (or your Tailscale hostname) in a browser. Enter the join code and a device name. You're in.

## 4. Create your first ANT

Click the **+** button in the sidebar. Enter an ID (e.g. `dev`) — everything else is optional and has sensible defaults. The ANT starts immediately.

## 5. Send a message

Select your ANT in the sidebar and type a message. You should see:

1. "Thinking..." appears
2. The Workers tab shows the task running
3. Claude's response appears with formatted code blocks

Try: "What's 2+2?" or "Write a Python hello world"

## 6. Useful commands

| Command | What it does |
|---|---|
| `/help` | List all commands |
| `/ants` | Show running workers |
| `/cancel` | Stop the current task |
| `/new` | Start a fresh conversation |

## 7. Optional: add Telegram or Slack

Click the ⚙ gear on your ANT → add a Telegram bot token or Slack tokens. Save and restart to activate.

## 8. Optional: set up HTTPS

```bash
sudo tailscale serve --bg http://localhost:3000
```

Access via `https://<machine-name>.<tailnet>.ts.net` with a valid certificate.

## Next steps

- [Production Setup](production-setup.md) — multiple ANTS, auto-start, backups
- [Web Dashboard](web-dashboard.md) — Tailscale HTTPS, PWA, cross-device history
- [Memory & Workspaces](memory-and-workspaces.md) — persistent memory, git backups
- [Configuration](configuration.md) — full reference for all settings
