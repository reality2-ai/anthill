# ANTHILL-WEB: Web Dashboard and API

**Version:** 0.3.0
**Date:** 2026-03-20
**Status:** Draft
**Depends on:** ANTHILL-INTRO, ANTHILL-COLONY

---

## 1. Introduction

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT", "SHOULD", "SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this document are to be interpreted as described in RFC 2119.

The Anthill web dashboard is an embedded single-page application (SPA) served from the Anthill binary. It provides real-time chat with ANTS, worker monitoring, file management, device provisioning, and ANT configuration.

### 1.1 Terminology

| Term | Definition |
|------|-----------|
| **SPA** | Single-page application — the entire web UI is one HTML file |
| **Credential** | Hex-encoded Ed25519 private key seed, stored in `localStorage` |
| **Envelope** | HMAC-signed wrapper for WebSocket messages |

---

## 2. Architecture

### 2.1 Embedded SPA

The web application HTML is compiled into the binary via `include_str!`. No external file serving is required.

### 2.2 Progressive Web App

The dashboard is a PWA with:
- `manifest.json` for install-to-homescreen.
- A service worker (`sw.js`) for offline caching.
- SVG icons at 192px and 512px.
- `apple-mobile-web-app-capable` for iOS homescreen apps.

### 2.3 Responsive Design

Font sizes scale with viewport width:
- Mobile (<768px): 14px base
- Tablet/laptop (768px+): 16px base
- Desktop (1200px+): 17px base

---

## 3. Authentication Flow

### 3.1 Join Screen

If no credential is stored in `localStorage`, the join screen is displayed.

Two entry paths:
1. **QR scan**: URL contains `#join=<code>` — the code is auto-filled.
2. **Manual entry**: user types the `xxxx-xxxx-xxxx` code.

The join flow:
1. Client POSTs `{code, device_name}` to `/api/auth/join`.
2. Server validates and consumes the join code (ANTHILL-COLONY §4.2).
3. Server returns `{device_id, credential, name}`.
4. Client stores credential and device info in `localStorage`.

### 3.2 Session Resumption

On page load:
1. Read credential from `localStorage`.
2. POST to `/api/auth/verify` with the credential.
3. If valid, show the app. If invalid, clear storage and show join screen.

### 3.3 Protected Endpoints

All API endpoints under `/api/` (except `/api/auth/verify`, `/api/auth/join`, `/api/auth/status`) require the `X-Credential` header.

The auth middleware:
1. Reads `X-Credential` header.
2. Derives the Ed25519 public key from the seed.
3. Looks up the device in the trust group.
4. Returns 401 if not found.

---

## 4. WebSocket Protocol

### 4.1 Connection

Client connects to `/ws` with query parameters:
- `credential`: hex-encoded Ed25519 seed
- `device_id`: hex-encoded public key

### 4.2 Envelope Format

Over HTTPS (where `crypto.subtle` is available), messages are signed:

```json
{
  "device_id": "hex...",
  "timestamp": 1711000000,
  "signature": "hex...",
  "payload": "{\"type\":\"chat\",...}"
}
```

Signature: HMAC-SHA256(credential, `device_id:timestamp:payload`).

Over HTTP, messages are sent unsigned (server accepts both).

### 4.3 Server → Client Events

On connection, the server sends a `snapshot` message containing:
- All ANT listings (running + configured)
- Chat history for each ANT

Subsequently, the server broadcasts events as they occur (see ANTHILL-WORKER §7.2). The supervisor also broadcasts ANT crash and restart status events, so the web UI can reflect real-time ANT health.

If a client's WebSocket falls behind (slow network, browser tab backgrounded), the broadcast channel drops oldest events. The server detects this and sends a `lag_warning` event:
```json
{"type": "lag_warning", "dropped": 5, "message": "Connection fell behind — 5 events dropped. Refresh for current state."}
```
The web UI displays this as a system message in the chat.

### 4.4 Client → Server Commands

| Command | Format |
|---------|--------|
| `chat` | `{type: "chat", bot: "id", message: "text", chat_id: 0}` |
| `cancel` | `{type: "cancel", bot: "id", task_id: 123}` |
| `followup` | `{type: "followup", bot: "id", task_id: 123, message: "text"}` |

---

## 5. REST API

### 5.1 ANT Management

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| GET | `/api/ants` | Yes | List all ANTS (running + configured) |
| POST | `/api/ants/create` | Yes | Create a new ANT |
| DELETE | `/api/ants/{id}` | Yes | Delete an ANT |
| POST | `/api/ants/reload` | Yes | Signal supervisor to scan for new ANTS |
| GET | `/api/ants/{id}/config` | Yes | Get ANT config (TOML) |
| PUT | `/api/ants/{id}/config` | Yes | Update ANT config |

### 5.2 Chat

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| POST | `/api/ants/{id}/chat` | Yes | Send a message to an ANT |
| POST | `/api/ants/{id}/cancel/{task_id}` | Yes | Cancel a running task |

### 5.3 Files

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| GET | `/api/ants/{id}/files` | Yes | List files in working directory |
| GET | `/api/ants/{id}/files/{path}` | Yes | Get a file |
| POST | `/api/ants/{id}/upload/{path}` | Yes | Upload a file |
| DELETE | `/api/ants/{id}/files/{path}` | Yes | Delete a file |

File paths are validated against the working directory via `canonicalize()` to prevent path traversal.

### 5.4 Authentication

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| POST | `/api/auth/verify` | No | Check if a credential is valid |
| POST | `/api/auth/join` | No | Join colony with a code |
| GET | `/api/auth/status` | No | Is the colony empty? |
| GET | `/api/auth/devices` | Yes | List provisioned devices |
| DELETE | `/api/auth/devices/{id}` | Yes | Revoke a device |
| GET | `/api/auth/qr-join` | Yes | Generate QR code for device provisioning |

### 5.5 Other

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| GET | `/api/backends` | Yes | List AI backends (installed/not) |
| GET | `/api/doctor` | Yes | Run diagnostic checks — returns status of all prerequisites (Rust, AI backends, Ollama models, Git, Tailscale, config, colony key, ANTs, devices, service). Same checks as `anthill --doctor` CLI |

---

## 6. UI Components

### 6.1 Sidebar

- ANT list with status indicators (green=running, red=stopped, grey=configured)
- Settings gear per ANT
- "Manage Devices" button

### 6.2 Chat View

- Message history with markdown rendering (via marked.js)
- Reply-to-message: hover → ↩ button → quote bar above input
- Auto-scroll on new messages
- Typing indicator
- **Slash command autocomplete**: typing `/` in the input opens a menu listing all available commands with descriptions. Arrow keys navigate, Tab or Enter selects. The menu filters as the user types.
- **Web command routing**: `/help`, `/status`, `/usage`, `/ants`, and `/cancel` are handled locally and return responses as system messages — no AI backend needed.
- **Auto-followup**: when one task is running, new messages auto-queue as follow-ups instead of starting concurrent tasks.
- **Interrupt (`!`)**: prefixing a message with `!` cancels the running task and restarts with combined context.
- **ANT not-running feedback**: sending a message to a stopped or unconfigured ANT displays an error message instead of silently dropping the message.

### 6.3 Workers Tab

- Per-worker cards: task ID, preview, elapsed time, progress, backend
- Confidence-coloured progress: green (normal), yellow (stall warning), purple (question), red (error)
- Follow-up input per worker card (focus is preserved during timer re-renders)
- Cancel button per worker

### 6.4 Files Tab

- Directory browser with breadcrumb navigation
- File preview (text, images)
- Upload and download (auth-aware)
- Delete with confirmation

### 6.5 Modals

- **Create ANT**: name, ID, Telegram token, working directory
- **ANT Settings**: full config editor (backends, personality, sync, backups)
- **Manage Devices**: device list + "Add Device (QR)" with countdown timer

All modals close on Escape. Create and Join submit on Enter.

---

## 7. Security Considerations

1. **No credential in URL paths.** Authentication is header-only (`X-Credential`). WebSocket auth uses query parameters (unavoidable for the upgrade handshake).
2. **Path traversal** is prevented by canonicalising file paths and verifying the prefix.
3. **CORS** is not configured (single-origin SPA).
4. **HTTPS** is recommended via `tailscale serve` or a reverse proxy.
5. **Downloads** use `authFetch` (not plain `<a href>`) to include the credential header.
