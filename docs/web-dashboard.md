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

- **Sidebar** — lists all ANTS with status (green = running, grey = configured, red = stopped)
- **Chat interface** — markdown rendering with headings, code blocks, tables, links
- **Reply-to-message** — hover any message → ↩ button → quote bar above input. Select text first to quote a specific part
- **Code blocks** — syntax highlighted with copy buttons (appear on hover)
- **Workers tab** — live progress per worker (tool use, stall warnings, questions), follow-up input, cancel
- **Files tab** — browse workspace, upload/download (auth-aware), preview, delete
- **Device management** — "Add Device (QR)" with 5-minute countdown timer, device list, revoke
- **ANT settings** — full config editor: backends, personality, sync, backups, timeout
- **Chat history** — persists to disk, loads on connect from any device
- **Cross-device sync** — messages from Telegram appear in the web UI and vice versa
- **Auto-reconnecting WebSocket** — reconnects with exponential backoff
- **Responsive** — scales fonts for mobile/tablet/desktop
- **Knowledge graph export** — export a single graph or all graphs as a self-contained HTML file with AI-written insights, interactive 3D graph, citations, and "View graph →" links. Optional guidance text shapes the AI report writer's focus and tone
- **Citation tracking** — citations are collected from graph edges and displayed as numbered references in exported reports. The `/citations` command consolidates sources across all topic graphs
- **Slash command autocomplete** — type `/` to see all commands with descriptions, arrow keys to navigate, Tab/Enter to select. Includes `/help`, `/status`, `/usage`, `/ants`, `/cancel`, `/analyse`, `/ruminate`, `/citations`, `/reflect`, and more
- **Web command routing** — `/help`, `/status`, `/usage`, `/ants`, `/cancel`, `/reflect`, `/ruminate`, `/citations`, `/reprocess-graphs` work directly from the web
- **Auto-followup** — when one task is running, new messages auto-queue as follow-ups instead of spawning concurrent tasks
- **Interrupt (`!`)** — prefix a message with `!` to cancel the running task and restart with combined context
- **ANT not-running feedback** — sending to a stopped or unconfigured ANT shows an error instead of silently dropping the message
- **Supervisor status events** — ANT crash and restart events are broadcast to the web UI in real time
- **Workers tab focus fix** — follow-up input no longer loses focus when the elapsed timer re-renders
- **Keyboard shortcuts** — Escape closes modals, Enter submits join/create dialogs

## Knowledge Export

Export an ANT's knowledge as a self-contained HTML file that can be opened in any browser — no server needed.

From the **Graph tab**, click:
- **Export** — export the currently selected graph
- **Export All** — export all graphs for this ANT

When exporting, you can optionally provide **guidance text** to shape the AI report writer's output — for example, "Focus on practical applications" or "Write for a beginner audience".

The exported file includes:
- **Insights tab** (default view) — AI-written narrative summary with numbered citations and "View graph →" links to jump to the relevant 3D visualisation
- **Graph tab** — interactive 3D force-directed graph with search, node details, and confidence-weighted edges
- **References section** — numbered citations linked to the claims they support

Exports are also automatically published as GitHub Gists when `gh` is authenticated.

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
