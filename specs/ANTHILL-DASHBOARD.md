# ANTHILL-DASHBOARD: Web Dashboard Interface

| Field      | Value                                                        |
|------------|--------------------------------------------------------------|
| Version    | 0.1 Draft                                                    |
| Date       | 2026-03-30                                                   |
| Status     | Draft                                                        |
| Depends on | ANTHILL-CHAT, ANTHILL-SENTANT, ANTHILL-TRUST                 |
| Related    | ANTHILL-GRAPH-UX, ANTHILL-WORKERS-UX, ANTHILL-ONBOARDING     |

> The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT",
> "SHOULD", "SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this
> document are to be interpreted as described in RFC 2119.

---

## 1. Introduction

The web dashboard is the primary interface for interacting with ANTs. It is a
single-page application served by the Anthill web server, providing real-time
visibility into ANT status, conversations, worker activity, file management,
and knowledge graph visualisation.

### 1.1 Scope

This specification covers:

- Page layout, navigation, and responsive breakpoints.
- Real-time update delivery via WebSocket.
- Theme system (light/dark).
- ANT creation and settings dialogs.
- REST API surface exposed to the dashboard.
- Progressive Web App (PWA) installation.

The following concerns are defined in separate specifications:

- **ANTHILL-GRAPH-UX** -- 3D knowledge graph interaction, node types, edge
  confidence colouring, graph queries, and export workflow.
- **ANTHILL-WORKERS-UX** -- worker task cards, progress rendering, follow-up
  input, and cancel behaviour.
- **ANTHILL-ONBOARDING** -- first-run provisioning, join-code flow, and
  device management.
- **ANTHILL-CHAT** -- conversation protocol, message formatting, context
  windowing, slash commands, and @mention routing.

---

## 2. Architecture

### 2.1 Single-File Application

The dashboard is a single HTML file (`web_app.html`) embedded in the server
binary at compile time via `include_str!`. The server MUST serve this file at
`GET /` as `text/html`.

### 2.2 Server Stack

The web server is built on Axum (Rust). It exposes three classes of endpoint:

1. **Public routes** -- no authentication required: `GET /` (dashboard),
   `GET /ws` (WebSocket), `GET /manifest.json`, `GET /sw.js`, icon routes,
   vendor JavaScript, and the auth verification endpoints (`POST /api/auth/verify`,
   `POST /api/auth/join`, `GET /api/auth/status`).

2. **Protected API routes** -- require a valid credential in the
   `X-Credential` HTTP header (see Section 10). These routes are guarded by
   an Axum middleware layer that calls `check_auth` and returns
   `401 Unauthorized` on failure.

3. **WebSocket** -- at `GET /ws`, authenticated via `credential` query
   parameter. Streams real-time events from the server to the client and
   accepts signed command envelopes from the client.

### 2.3 WebSocket Connection

The client MUST open a WebSocket connection to `/ws` with query parameters
`credential` and `device_id`. The protocol used MUST match the page protocol:
`wss:` for HTTPS pages, `ws:` for HTTP pages.

On connection, the server sends a `snapshot` message containing all ANTs,
their statuses, chat history, and any currently running tasks.

#### 2.3.1 Reconnection

On WebSocket close, the client MUST attempt reconnection after a fixed delay.
The current implementation reconnects after 3 seconds. During disconnection,
the client MUST display a visible "Disconnected -- reconnecting..." banner
using the `#connection` element with the `disconnected` CSS class.

On successful reconnection, the server sends a fresh `snapshot` message. The
client MUST replace its local task state with the server-authoritative
snapshot, ensuring stale entries are cleared.

### 2.4 Vendor Libraries

The following JavaScript libraries are embedded in the binary and served from
`/vendor/*`:

| Path                             | Library                          |
|----------------------------------|----------------------------------|
| `/vendor/three.min.js`           | Three.js (3D rendering)          |
| `/vendor/three-spritetext.min.js`| Three.js SpriteText              |
| `/vendor/3d-force-graph.min.js`  | 3D Force Graph                   |
| `/vendor/force-graph.min.js`     | 2D Force Graph                   |

The Marked library (`marked.min.js`) is loaded from a CDN.

---

## 3. Layout

The dashboard uses a two-column layout: a fixed-width sidebar on the left and
a flexible main area on the right. The overall layout MUST use `display: flex`
with `height: 100dvh` and `overflow: hidden` on the body.

### 3.1 Sidebar

The sidebar MUST be 260px wide (`min-width: 260px`) with a
`var(--surface)` background and a right border.

It contains, from top to bottom:

1. **Header** -- the Anthill logo (served from `GET /logo.svg`), a refresh
   button (triggers `GET /api/ants`), and a "+" button (opens the ANT
   creation dialog).

2. **ANT list** (`#bot-list`) -- scrollable list of all ANTs. Each entry
   MUST display:
   - A **status dot** (10px circle): green (`var(--green)`, class `running`)
     when the ANT is active; red (`var(--red)`, class `stopped`) when
     stopped or crashed; grey when configured but not started.
   - The ANT **display name** (bold) and metadata line (dimmed text).
   - A gear icon to open the settings dialog for that ANT.
   - The currently active ANT MUST have a left accent border
     (`3px solid var(--accent)`) and a `var(--surface2)` background.

3. **Manage Devices** button -- opens the device management modal.

4. **Colony Key** -- a collapsible `<details>` element showing export/import
   commands for the colony key.

5. **Footer** -- device name and the tagline "ANTS: Autonomous
   iNTelligenceS".

### 3.2 Main Area

The main area (`#main`) is a vertical flex column containing:

1. **Connection indicator** (`#connection`) -- hidden when connected, shown
   with red background and white text when disconnected.

2. **Header bar** (`#header`) -- contains:
   - A hamburger menu button (`#menu-btn`, visible only on mobile).
   - The active ANT's title (`#bot-title`).
   - An engine badge (`#engine-badge`) showing the current AI backend.
   - The **tab bar** (`#tab-bar`) with four tabs: Chat, Workers, Files,
     Graph. The tab bar MUST be hidden (`display: none`) until an ANT is
     selected.
   - A **task bar** (`#task-bar`) showing running task count and elapsed
     time.
   - A **theme toggle** button (sun/moon icon).

3. **Content panels** -- exactly one panel is visible at a time, controlled
   by the active tab:
   - `#chat` -- the chat message list.
   - `#workers-panel` -- live worker task cards.
   - `#files-panel` -- file browser with breadcrumb, toolbar, file list,
     and preview pane.
   - `#graph-panel` -- 3D knowledge graph container with toolbar, legend,
     query bar, and info panel.

4. **Tasks panel** (`#tasks-panel`) -- a compact task summary strip shown
   below the content area when tasks are running. Each task item shows a
   preview, elapsed time, and cancel button.

5. **Input area** (`#input-area`) -- visible only when an ANT is selected.
   Contains:
   - A **quote bar** (`#quote-bar`) for reply-to-message context, with an
     accent left border and a close button.
   - A **slash command menu** (`#slash-menu`) for autocomplete when the user
     types `/`.
   - A **mention menu** (`#mention-menu`) for `@`-mention autocomplete.
   - An **input row** with a `<textarea>` (auto-resizing, 1--5 rows), a
     Send button (`var(--accent)` background), and a Compact button
     (triggers conversation analysis and history trimming).

6. **No-ANT placeholder** (`#no-bot`) -- centred text shown when no ANT is
   selected.

---

## 4. Tabs

### 4.1 Chat

The Chat tab displays the conversation as a vertical list of message bubbles.
User messages align right (`align-self: flex-end`, `var(--surface2)`
background). Bot messages align left (`align-self: flex-start`,
`var(--surface)` background). Messages MUST have a maximum width of 85% of
the container (95% on mobile).

Message content is rendered from Markdown using the Marked library. The
renderer MUST support:

- Headings (h1--h4) with hierarchical sizing.
- Paragraphs, lists (ordered and unordered), and horizontal rules.
- Links (coloured with `var(--accent)`).
- Blockquotes (left border, dimmed text).
- Tables (collapsed borders, header row with `var(--surface2)` background).
- Code blocks (monospace font, `var(--code-bg)` background, horizontal
  scroll). Each code block MUST have a **copy button** that appears on hover
  and copies the block content to the clipboard.
- Inline code (monospace, padded, rounded).

Each message MUST have a **reply button** that appears on hover. Clicking the
reply button populates the quote bar with the referenced message text. If
text is selected within the message at the time, only the selected text is
quoted.

A **typing indicator** (italic, dimmed) MUST appear when a `typing` event is
received and MUST be cleared when a `message` event arrives.

See ANTHILL-CHAT for the full conversation protocol, slash commands, @mention
routing, and interrupt (`!`) behaviour.

### 4.2 Workers

The Workers tab shows live task cards for all running workers. Each card
displays the task preview, elapsed time, progress entries (tool use, agent
spawn, text output), and a cancel button.

When a `task_progress` event with `kind: "question"` arrives, the dashboard
MUST automatically switch to the Workers tab for the active ANT so the user
can respond to the question.

See ANTHILL-WORKERS-UX for detailed worker card rendering, follow-up input
behaviour, stall warnings, and progress history.

### 4.3 Files

The Files tab provides a file browser for the ANT's workspace directory.

- **Breadcrumb** (`#files-breadcrumb`) -- shows the current path.
- **Toolbar** (`#files-toolbar`) -- an Upload button (accepts multiple
  files; `.zip` files are auto-extracted server-side) and a note about zip
  extraction.
- **File list** (`#files-list`) -- each file item shows an icon, name,
  size, and a delete button (appears on hover). Clicking a directory
  navigates into it. Clicking a file opens the preview pane.
- **Preview pane** (`#file-preview`) -- displays file content or download
  link.

File operations use the following API endpoints:

| Operation | Method   | Endpoint                           |
|-----------|----------|------------------------------------|
| List      | `GET`    | `/api/ants/{id}/files`             |
| Download  | `GET`    | `/api/ants/{id}/files/{path}`      |
| Upload    | `POST`   | `/api/ants/{id}/upload/{path}`     |
| Delete    | `DELETE` | `/api/ants/{id}/files/{path}`      |

Uploads MUST NOT exceed 50 MiB (`MAX_UPLOAD_BYTES = 50 * 1024 * 1024`). The
server MUST return `413 Payload Too Large` for oversized uploads.

### 4.4 Graph

The Graph tab renders an interactive knowledge graph visualisation inside the
`#graph-panel`. It includes:

- A **graph selector** dropdown (`#graph-selector`) for choosing which topic
  graph to display (or the meta-graph).
- An **Export** button that opens the export dialog.
- A **theme toggle** button.
- A **legend** showing node type colours and edge confidence ranges.
- A **query bar** (`#graph-query-bar`) with a text input and Ask button for
  querying the current graph via the ANT's AI.
- A **node info panel** (`#graph-info`) that appears when a node is clicked,
  showing label, kind, summary, tags, and connected edges with confidence
  percentages.

Node types are colour-coded:

| Node Kind  | Colour    | CSS Variable    |
|------------|-----------|-----------------|
| person     | `#e94560` | `var(--accent)` |
| project    | `#4ade80` | `var(--green)`  |
| tool       | `#fbbf24` | `var(--yellow)` |
| concept    | `#60a5fa` | --              |
| decision   | `#c084fc` | --              |
| server     | `#f472b6` | --              |
| event      | `#fb923c` | --              |
| fact       | `#94a3b8` | --              |

Edge confidence is shown as a percentage with colour coding: green (>= 80%),
yellow (>= 50%), red (< 50%).

Right-clicking a graph node opens a **node update dialog** that allows the
user to describe a natural-language update to apply to that node.

See ANTHILL-GRAPH-UX for the full graph interaction model, 3D/2D rendering,
force layout parameters, and export workflow.

---

## 5. Real-time Updates

All real-time updates flow through the WebSocket as JSON messages. Each
message has a `type` field that determines how the client processes it. The
server defines the `WsEvent` enum (in `registry.rs`) with the following
variants, serialised as their `#[serde(rename)]` values:

### 5.1 `snapshot`

Sent on initial connection and reconnection. Contains:

- `bots` -- array of all ANTs with `id`, `status`, and metadata.
- `history` -- map of ANT ID to array of chat messages.
- `tasks` -- map of ANT ID to array of running tasks with `task_id`,
  `preview`, `elapsed_secs`, and `progress`.

The client MUST replace all local task state with the snapshot (server is
authoritative). Bot list, chat, and task panels MUST be re-rendered.

### 5.2 `message`

A bot sent a chat response. Fields: `bot`, `chat_id`, `text`, `task_id`.

The client MUST append the message to the chat history for the specified bot,
clear any typing indicator, and re-render the chat if the bot is active.

### 5.3 `user_message`

A user message from any channel (web, Telegram, Slack). Fields: `bot`,
`chat_id`, `text`, `source`.

The client MUST append the message to chat history, de-duplicating against
the most recent local user message (to avoid echoing messages the current
client sent). This enables cross-channel sync: messages sent via Telegram
appear in the web UI and vice versa.

### 5.4 `task_started`

A new AI worker task has started. Fields: `bot`, `task_id`, `preview`.

The client MUST add a task entry to the tasks state for the specified bot,
re-render the task bar and bot list (to show active indicators).

### 5.5 `task_progress`

Real-time progress from a running task. Fields: `bot`, `task_id`, `kind`,
`detail`.

The `kind` field indicates the nature of the progress:

| Kind          | Meaning                                    |
|---------------|--------------------------------------------|
| `tool_use`    | The worker is executing a tool             |
| `agent_spawn` | The worker spawned a sub-agent             |
| `text`        | Streaming text output                      |
| `question`    | The worker is asking the user a question   |
| `warning`     | A stall or timeout warning                 |

The client MUST update the matching task card and keep the last 20 progress
entries. If `kind` is `question` and the bot is active, the client MUST
switch to the Workers tab automatically.

### 5.6 `task_completed`

A task has finished. Fields: `bot`, `task_id`, `duration_secs`.

The client MUST remove the task from the tasks state, update the task bar and
bot list, and re-render the Workers tab if active.

### 5.7 `task_error`

A task failed or timed out. Fields: `bot`, `task_id`, `error`.

The client MUST mark the task with an error state and display the error
message in the task card. The task remains visible until the next full
snapshot or manual dismissal.

### 5.8 `typing`

The bot is generating a response. Fields: `bot`.

The client MUST show a typing indicator in the chat area for the specified
bot.

### 5.9 `bot_status`

An ANT's status changed. Fields: `bot`, `status`.

The client MUST update the status field in the local bot state and re-render
the sidebar to reflect the new status dot colour.

### 5.10 `graph_updated`

The knowledge graph was modified. Fields: `bot`, `graph`.

The `graph` field contains the name of the changed graph (e.g. `"meta"`, a
topic name, or `"all"`). If the Graph tab is active and the current bot
matches, the client MUST reload the graph if the changed graph matches the
currently displayed graph or if `graph` is `"all"`.

---

## 6. Theme System

The dashboard supports light and dark themes, toggled via a button in the
header bar and the graph toolbar.

### 6.1 CSS Custom Properties

All colours are defined as CSS custom properties on `:root` (dark theme,
default) and `[data-theme="light"]`:

| Property          | Dark Value       | Light Value      |
|-------------------|------------------|------------------|
| `--bg`            | `#1a1a2e`        | `#f5f5f7`        |
| `--surface`       | `#16213e`        | `#ffffff`        |
| `--surface2`      | `#0f3460`        | `#e8eaed`        |
| `--accent`        | `#e94560`        | `#d63050`        |
| `--text`          | `#eee`           | `#1a1a2e`        |
| `--text-dim`      | `#888`           | `#666`           |
| `--green`         | `#4ade80`        | `#16a34a`        |
| `--yellow`        | `#fbbf24`        | `#ca8a04`        |
| `--red`           | `#f87171`        | `#dc2626`        |
| `--border`        | `#333`           | `#d1d5db`        |
| `--border-light`  | `#444`           | `#c0c4cc`        |
| `--code-bg`       | `#0a0a1a`        | `#eef0f4`        |
| `--overlay`       | `rgba(0,0,0,0.7)`| `rgba(255,255,255,0.85)` |
| `--overlay-strong`| `rgba(0,0,0,0.85)`| `rgba(255,255,255,0.95)` |

Font stacks:

| Property | Value |
|----------|-------|
| `--font` | `-apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif` |
| `--mono` | `'SF Mono', 'Fira Code', 'Consolas', monospace` |

### 6.2 Persistence

The selected theme MUST be persisted to `localStorage` under the key
`anthill_theme`. On page load, the client MUST read this value and apply the
theme before rendering. If no value is stored, the client MUST default to
dark theme unless the system prefers light (via `prefers-color-scheme`).

### 6.3 Theme-Colour Meta Tag

When the theme changes, the client MUST update the `<meta name="theme-color">`
tag: `#1a1a2e` for dark, `#f5f5f7` for light. This ensures the browser
chrome matches the app theme on mobile devices.

---

## 7. PWA Support

The dashboard is installable as a Progressive Web App on Android, iOS, and
desktop platforms.

### 7.1 Manifest

The server MUST serve `GET /manifest.json` with `Content-Type:
application/json`. The manifest MUST contain:

```json
{
  "name": "Anthill",
  "short_name": "Anthill",
  "description": "AI-powered bots backed by Claude Code",
  "start_url": "/",
  "display": "standalone",
  "background_color": "#1a1a2e",
  "theme_color": "#1a1a2e",
  "icons": [
    { "src": "/icon.svg", "sizes": "any", "type": "image/svg+xml", "purpose": "any" },
    { "src": "/icon-192.svg", "sizes": "192x192", "type": "image/svg+xml", "purpose": "any maskable" },
    { "src": "/icon-512.svg", "sizes": "512x512", "type": "image/svg+xml", "purpose": "any maskable" }
  ]
}
```

### 7.2 Service Worker

The server MUST serve `GET /sw.js` with `Content-Type:
application/javascript`. The service worker is a minimal pass-through:

```javascript
self.addEventListener('fetch', e => e.respondWith(fetch(e.request)));
```

This satisfies browser PWA installation requirements without introducing
offline caching complexity.

### 7.3 Icons

The server MUST serve dynamically rendered SVG icons at:

- `GET /icon.svg` -- 512x512 app icon.
- `GET /icon-192.svg` (also `/icon-192.png`) -- 192x192 icon.
- `GET /icon-512.svg` (also `/icon-512.png`) -- 512x512 icon.

Icons depict the Anthill brand: a mound with a lightbulb motif on a
`#1a1a2e` background with `#e94560` mound and `#4ade80` lightbulb.

### 7.4 HTML Meta Tags

The HTML document MUST include the following meta tags for PWA support:

- `<meta name="mobile-web-app-capable" content="yes">`
- `<meta name="apple-mobile-web-app-status-bar-style" content="black-translucent">`
- `<meta name="apple-mobile-web-app-title" content="Anthill">`
- `<meta name="theme-color" content="#1a1a2e">`
- `<link rel="manifest" href="/manifest.json">`
- `<link rel="icon" type="image/svg+xml" href="/icon.svg">`
- `<link rel="apple-touch-icon" href="/icon-192.svg">`

---

## 8. ANT Creation Dialog

The ANT creation dialog is a modal (`#create-modal`) opened by clicking the
"+" button in the sidebar header.

### 8.1 Fields

| Field              | Element ID   | Required | Validation                        |
|--------------------|-------------|----------|-----------------------------------|
| ID                 | `#f-id`     | Yes      | Non-empty, no spaces              |
| Display Name       | `#f-name`   | No       | Defaults to ID                    |
| Telegram Bot Token | `#f-token`  | No       | Leave empty for web-only          |
| Working Directory  | `#f-workdir`| No       | Defaults to `~/.config/anthill/ants/<id>/working` |
| System Prompt      | `#f-prompt` | No       | Free text, defaults to generic    |

### 8.2 Submission

On submit, the client MUST validate that the ID field is non-empty and
contains no spaces, then send `POST /api/ants/create` with the form data as
JSON. On success, the client MUST close the modal, trigger a supervisor
reload, and refresh the bot list.

### 8.3 Keyboard Shortcuts

- **Enter** MUST submit the dialog.
- **Escape** MUST close the dialog.

---

## 9. ANT Settings Dialog

The ANT settings dialog is a modal (`#config-modal`) opened by clicking the
gear icon on an ANT in the sidebar.

### 9.1 Sections and Fields

**Display Name** -- editable text field.

**AI Engine** -- a dropdown (`#c-ai-category`) with the following categories:

| Value               | Label                                          |
|---------------------|------------------------------------------------|
| `balanced`          | Balanced -- good mix of speed, cost, quality   |
| `intellectual`      | Intellectual -- best reasoning / most capable  |
| `fast`              | Fast -- fastest response time                  |
| `cost_effective`    | Cost-effective -- prefer local and cheap models|
| `local`             | Local only -- no data leaves the machine       |
| `specialized:coding`| Specialized: Coding                            |
| `manual`            | Manual -- specify exact backend order below    |

When `manual` is selected, a backend list (`#c-backends`) MUST appear
allowing the user to specify and reorder backends.

An engine status panel (`#engine-status`) displays the current engine and
fallback order.

**Telegram** -- bot token and allowed chat IDs (comma-separated).

**Slack** -- bot token (`xoxb-...`) and app token (`xapp-...`).

**Workspace** -- working directory path.

**Backups** -- interval in hours (0 = disabled) and git remote name.

**Cross-channel sync** -- checkbox to show messages from other channels.

**Code access** -- checkbox to allow modifying files outside workspace.

**Rumination** -- enable/disable toggle with sub-options (visible when
enabled):
- Interval in hours (1--24).
- Active refutation (challenge existing beliefs).
- Idea synthesis (conjecture transitive relationships).
- Contradiction resolution.
- Autonomous initiative (uses more tokens).

**Personality** -- system prompt textarea.

### 9.2 Submission

On save, the client MUST send `PUT /api/ants/{id}/config` with all fields.
The server merges the update onto the existing `ant.toml` configuration,
preserving fields not controlled by the UI (e.g. `[ai.categories]`,
`memory_dir`). On success the server returns the response
"Config saved. Restart Anthill to apply."

### 9.3 Deletion

The settings dialog includes a **Delete** button (red, bottom-left). On
click, the client MUST confirm the action, then send
`DELETE /api/ants/{id}`. On success, the ANT is removed from the bot list.

---

## 10. REST API Summary

All protected endpoints require the `X-Credential` header containing a valid
device credential (see ANTHILL-TRUST). The auth middleware returns
`401 Unauthorized` if the credential is missing, empty, or invalid.

### 10.1 ANT Management

| Method   | Endpoint                          | Description                           |
|----------|-----------------------------------|---------------------------------------|
| `GET`    | `/api/ants`                       | List all ANTs with status             |
| `POST`   | `/api/ants/create`                | Create a new ANT                      |
| `DELETE` | `/api/ants/{id}`                  | Delete an ANT                         |
| `POST`   | `/api/ants/{id}/restart`          | Restart an ANT                        |
| `POST`   | `/api/ants/reload`                | Trigger supervisor reload             |

### 10.2 Chat

| Method   | Endpoint                          | Description                           |
|----------|-----------------------------------|---------------------------------------|
| `POST`   | `/api/ants/{id}/chat`             | Send a message to an ANT              |
| `POST`   | `/api/ants/{id}/cancel/{task_id}` | Cancel a running task                 |
| `POST`   | `/api/ants/{id}/compact-history`  | Analyse and trim chat history         |

### 10.3 Configuration

| Method   | Endpoint                          | Description                           |
|----------|-----------------------------------|---------------------------------------|
| `GET`    | `/api/ants/{id}/config`           | Read ANT config as structured JSON    |
| `PUT`    | `/api/ants/{id}/config`           | Update ANT config from form fields    |

### 10.4 Files

| Method   | Endpoint                          | Description                           |
|----------|-----------------------------------|---------------------------------------|
| `GET`    | `/api/ants/{id}/files`            | List files in workspace               |
| `GET`    | `/api/ants/{id}/files/{path}`     | Download a file                       |
| `POST`   | `/api/ants/{id}/upload/{path}`    | Upload a file (max 50 MiB)            |
| `DELETE` | `/api/ants/{id}/files/{path}`     | Delete a file                         |

### 10.5 Knowledge Graph

| Method   | Endpoint                          | Description                           |
|----------|-----------------------------------|---------------------------------------|
| `GET`    | `/api/ants/{id}/graph`            | Get graph data for visualisation      |
| `GET`    | `/api/ants/{id}/export`           | Download graph export                 |
| `POST`   | `/api/ants/{id}/report`           | Start background report generation    |
| `GET`    | `/api/ants/{id}/reports/{filename}`| Download a generated report          |

### 10.6 Rumination and Engine

| Method   | Endpoint                          | Description                           |
|----------|-----------------------------------|---------------------------------------|
| `GET`    | `/api/ants/{id}/rumination`       | Get rumination log                    |
| `GET`    | `/api/ants/{id}/engine`           | Get engine info (current backend)     |

### 10.7 Authentication and Devices

| Method   | Endpoint                          | Description                           |
|----------|-----------------------------------|---------------------------------------|
| `POST`   | `/api/auth/verify`                | Verify a credential (public)          |
| `POST`   | `/api/auth/join`                  | Join colony with a code (public)      |
| `GET`    | `/api/auth/status`                | Check auth status (public)            |
| `GET`    | `/api/auth/devices`               | List connected devices                |
| `DELETE` | `/api/auth/devices/{id}`          | Revoke a device                       |
| `GET`    | `/api/auth/qr-join`              | Generate QR code for device provisioning |

### 10.8 System

| Method   | Endpoint                          | Description                           |
|----------|-----------------------------------|---------------------------------------|
| `GET`    | `/api/backends`                   | List available AI backends            |
| `GET`    | `/api/doctor`                     | Run diagnostic checks                 |

---

## 11. Responsive Design

The dashboard MUST adapt to three breakpoint ranges using CSS media queries.

### 11.1 Mobile (max-width: 768px)

- The sidebar MUST be hidden by default (`left: -260px`, `position: fixed`,
  `z-index: 100`), revealed via a hamburger toggle button (`#menu-btn`)
  which slides it in (`left: 0`) with a 0.2s transition.
- A backdrop overlay (`#sidebar-backdrop`, `rgba(0,0,0,0.4)`) MUST appear
  behind the open sidebar. Clicking the backdrop MUST close the sidebar.
- Messages MUST have a maximum width of 95%.
- Tab labels MUST use reduced padding (`6px 10px`) and font size (`12px`).
- Config modal inner padding MUST be reduced to 16px.
- The graph legend MUST be hidden (`display: none`).
- All interactive elements (cancel buttons, delete buttons, copy buttons)
  MUST have minimum touch targets of 44x44px.

### 11.2 Tablet (min-width: 769px)

- The sidebar MUST be visible at all times.
- The hamburger button MUST be hidden.
- The sidebar backdrop MUST be hidden.
- Font sizes scale up: body 16px, messages 15px, code 14px, input 15px.

### 11.3 Desktop (min-width: 1200px)

- Body font size increases to 17px.
- Messages increase to 16px, code to 15px, input to 16px.
- Full sidebar width with the main content area filling remaining space.

---

## 12. Conformance

### 12.1 REQUIRED

An implementation claiming conformance to ANTHILL-DASHBOARD:

1. MUST serve the dashboard as a single HTML page at `GET /`.
2. MUST provide a WebSocket endpoint at `GET /ws` that streams `WsEvent`
   messages as defined in Section 5.
3. MUST authenticate all protected API routes via the `X-Credential` header
   and return `401 Unauthorized` on failure.
4. MUST authenticate WebSocket connections via the `credential` query
   parameter.
5. MUST send a `snapshot` message on WebSocket connection containing all
   ANTs, their statuses, chat history, and running tasks.
6. MUST render Markdown in chat messages with copy buttons on code blocks.
7. MUST display ANT status dots in the sidebar reflecting current status.
8. MUST support light and dark themes with CSS custom properties, persisted
   to `localStorage`.
9. MUST serve a valid PWA manifest at `GET /manifest.json` and a service
   worker at `GET /sw.js`.
10. MUST enforce the 50 MiB upload limit on file uploads.
11. MUST display a disconnection banner when the WebSocket is closed and
    attempt reconnection.
12. MUST support the four tabs: Chat, Workers, Files, Graph.

### 12.2 RECOMMENDED

1. SHOULD support keyboard shortcuts: Escape to close modals, Enter to
   submit dialogs.
2. SHOULD provide slash command autocomplete when the user types `/` in the
   input area.
3. SHOULD provide @mention autocomplete when the user types `@` in the
   input area.
4. SHOULD auto-switch to the Workers tab when a `question`-kind progress
   event arrives.
5. SHOULD support reply-to-message via hover button with text selection
   quoting.

### 12.3 OPTIONAL

1. MAY support the Compact button for conversation analysis and history
   trimming.
2. MAY support graph querying via the Ask bar in the Graph tab.
3. MAY support right-click node update on graph nodes.
4. MAY support auto-extraction of uploaded `.zip` files.

---

## 13. References

- RFC 2119. Bradner, S. "Key words for use in RFCs to Indicate Requirement
  Levels." IETF, 1997.
- ANTHILL-CHAT -- Chat Interface specification.
- ANTHILL-SENTANT -- ANT Sentant specification.
- ANTHILL-TRUST -- Trust & Security specification.
- ANTHILL-GRAPH-UX -- Graph Interaction UX specification.
- ANTHILL-WORKERS-UX -- Worker Visibility UX specification.
- ANTHILL-ONBOARDING -- Onboarding specification.
- Axum -- Rust web framework. https://github.com/tokio-rs/axum
- Marked -- Markdown parser. https://marked.js.org
- 3D Force Graph -- https://github.com/vasturiano/3d-force-graph
