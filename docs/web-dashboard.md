# Web Dashboard

A browser-based interface for Anthill, accessible from any device on your Tailscale network.

## Accessing the dashboard

The web server runs alongside the supervisor on port 3000 (configurable in `supervisor.toml`).

### HTTP (simple)

```
http://<tailscale-ip>:3000
```

### HTTPS via Tailscale (recommended)

Tailscale provisions a valid Let's Encrypt certificate automatically:

```bash
# Find your machine's Tailscale domain
tailscale status --self
# e.g. <machine-name>.tail12345.ts.net

# Set up HTTPS proxy (persists across reboots)
sudo tailscale serve --bg http://localhost:3000
```

Access at `https://<machine-name>.tail12345.ts.net`

Check what's being served:

```bash
tailscale serve status
```

HTTPS is recommended because:
- Required for PWA installation on some browsers
- Enables secure WebSocket (`wss://`)
- Encrypted even within Tailscale

## Features

- **Sidebar** — lists all ANTS with live status indicators (green = running, red = stopped)
- **Chat interface** — markdown rendering with headings, code blocks, tables, links
- **Code blocks** — syntax highlighted with copy buttons (appear on hover)
- **Task panel** — shows running workers with durations and cancel buttons
- **Chat history** — persists to disk, loads on connect from any device
- **Cross-device sync** — messages from Telegram appear in the web UI and vice versa
- **Auto-reconnecting WebSocket** — reconnects automatically if the connection drops
- **Responsive** — works on mobile phones, tablets, and desktops

## Install as a PWA

Install Anthill as a standalone app on your device (works best over HTTPS):

- **Android:** Chrome menu (⋮) → "Add to Home screen"
- **iOS:** Safari share (⎋) → "Add to Home Screen"
- **Linux/macOS:** Chrome menu → "Install Anthill"

The app runs without browser chrome — looks and feels like a native app.

## Configuration

In `supervisor.toml`:

```toml
http_port = 3000        # Web server port
http_bind = "0.0.0.0"   # Bind address (0.0.0.0 = all interfaces)
```

## Chat history

Messages from all sources (Telegram, web UI) are recorded centrally in JSONL files:

```
~/.config/anthill/history/
├── my-ant.jsonl
└── another-ant.jsonl
```

When you open the dashboard from a new device, full conversation history loads immediately. History is capped at 500 messages per ANT.
