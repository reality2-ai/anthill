# ANTHILL-ONBOARDING: Device Provisioning and First-Run Experience

| Field      | Value                                                        |
|------------|--------------------------------------------------------------|
| Version    | 0.1 Draft                                                    |
| Date       | 2026-03-30                                                   |
| Status     | Draft                                                        |
| Depends on | ANTHILL-TRUST, ANTHILL-DASHBOARD                             |
| Related    | ANTHILL-COLONY, R2-PROVISION                                 |

> The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT",
> "SHOULD", "SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this
> document are to be interpreted as described in RFC 2119.

---

## 1. Introduction

This specification defines the first-run experience for Anthill: from
installing the binary to sending the first message to an ANT. It covers
the `install.sh` script, the `anthill --doctor` diagnostics command,
join code generation and consumption, device provisioning via the web
dashboard, the empty colony state, first ANT creation, first
interaction, and device management.

The goal is a path where a new operator can go from zero to a working
ANT conversation in under five minutes, with clear feedback at every
step and no silent failures.

### 1.1 Scope

This specification covers:

- Binary installation and service registration across platforms.
- The `--doctor` diagnostic command and its web API equivalent.
- Join code generation (CLI and web API) and QR code provisioning.
- The browser-side join flow: enter code, receive credential, store in
  `localStorage`.
- The empty colony state displayed when no devices or ANTs exist.
- The create ANT modal: fields, validation, and defaults.
- First interaction: sending a message, observing the thinking
  indicator, and receiving a response.
- Device management: listing, revoking, and QR countdown behaviour.

### 1.2 Out of Scope

- Trust group internals and cryptographic primitives (see ANTHILL-TRUST).
- Dashboard layout, tabs, and graph visualisation (see ANTHILL-DASHBOARD).
- ANT configuration beyond the create modal (see ANTHILL-COLONY).
- Telegram and Slack adapter onboarding (see ANTHILL-TELEGRAM,
  ANTHILL-SLACK).

---

## 2. Installation

### 2.1 install.sh

The canonical installation method is the `install.sh` script located at
the repository root. Implementations MUST provide equivalent
functionality.

```
./install.sh            # production (info-level logging)
./install.sh dev        # development (debug-level logging)
```

The script performs the following steps in order:

1. **Build.** `cargo build -p anthill --release`. The script MUST fail
   (`set -e`) if the build fails.

2. **Platform detection.** `uname -s` determines the platform. Three
   platforms are supported: Darwin (macOS), Linux (systemd), and BSD
   (rc.d).

3. **Binary placement.** The release binary is copied to
   `/usr/local/bin/anthill` with mode `755`. On Linux, the script MUST
   stop an existing `anthill` systemd service before overwriting the
   binary, then restart it after installation.

4. **Config directory.** `~/.config/anthill/ants/` is created if it does
   not exist. If `~/.config/anthill/supervisor.toml` does not exist, the
   example configuration is copied from `config-example/supervisor.toml`.

5. **Service registration.**

   | Platform | Mechanism | Service Name |
   |----------|-----------|--------------|
   | macOS    | launchd plist | `ai.reality2.anthill` |
   | Linux    | systemd unit | `anthill.service` |
   | BSD      | rc.d script | `anthill` |

   On Linux, the script MUST call `systemctl daemon-reload` after
   writing the unit file and then `systemctl enable --now anthill`.

6. **Post-install output.** The script prints:
   - Config directory path.
   - Binary path.
   - ANT count (from `~/.config/anthill/ants/*/ant.toml`).
   - Instructions for generating a join code (`anthill --qr-join` or
     `anthill --join-code`).
   - Web dashboard URL (`http://localhost:3000`).
   - In dev mode, the script tails the log file (macOS) or journal
     (Linux) before exiting.

### 2.2 First Run

When the Anthill supervisor starts for the first time:

1. If `~/.config/anthill/colony.key` does not exist, a new Ed25519
   signing key is generated, hex-encoded, and written to that path.
   This key is the root of the colony trust group.

2. The web server binds to the address specified in `supervisor.toml`
   (default `:3000`).

3. The colony is in the **empty colony state** (no provisioned devices,
   no ANTs). See Section 6.

---

## 3. Diagnostics (`--doctor`)

### 3.1 CLI Invocation

```
anthill --doctor
```

The `--doctor` command checks prerequisites, configuration, and service
status, then prints a colour-coded report to the terminal.

### 3.2 Check Categories

Each check has a severity level: `required`, `recommended`, `optional`,
or `info`. The following checks MUST be performed:

#### 3.2.1 Required

| Check | Command | Help |
|-------|---------|------|
| Rust toolchain | `cargo --version` | https://rustup.rs/ |
| Git | `git --version` | https://git-scm.com/downloads |
| Config directory | Exists: `~/.config/anthill/` | `mkdir -p ~/.config/anthill/ants` |

#### 3.2.2 Recommended

| Check | Command | Help |
|-------|---------|------|
| Claude Code | `claude --version` | https://docs.anthropic.com/en/docs/claude-code |
| Ollama | `ollama --version` | https://ollama.com/download |
| Ollama `nomic-embed-text` model | `ollama list` contains model | `ollama pull nomic-embed-text` |
| Tailscale | `tailscale version` | https://tailscale.com/download |

#### 3.2.3 Optional

| Check | Command | Help |
|-------|---------|------|
| OpenAI Codex | `codex --version` | https://github.com/openai/codex |
| Google Gemini CLI | `gemini --version` | https://ai.google.dev/gemini-cli |
| OpenCode | `opencode --version` | `npm i -g @opencode/cli` |
| Grok CLI | `grok --version` | `npm i -g grok-cli` |
| DeepSeek CLI | `deepseek --version` | `npm i -g run-deepseek-cli` |
| LM Studio | `lm-studio --version` | https://lmstudio.ai |
| Ollama `llama3.2` model | `ollama list` contains model | `ollama pull llama3.2` |
| pdftotext | `pdftotext -v` | `sudo apt install poppler-utils` |
| pandoc | `pandoc --version` | https://pandoc.org/installing.html |

#### 3.2.4 Info

| Check | What | Detail |
|-------|------|--------|
| Colony key | `~/.config/anthill/colony.key` exists | Auto-generated on first `anthill --supervise` |
| ANTs configured | Count of `ant.toml` files in `~/.config/anthill/ants/` | — |
| Devices provisioned | Count of entries in `~/.config/anthill/devices.toml` | — |
| systemd service (Linux) | `systemctl is-active anthill` | `sudo systemctl enable --now anthill` |

### 3.3 Output Format

Each check is printed as one line:

```
  <icon> <name> -- <detail>
    -> <help>           (only if status != ok)
```

Icons:
- Green check mark: status `ok`.
- Red cross: status `missing` with severity `required`.
- Yellow cross: status `missing` with severity `recommended` or
  `optional`.
- Open circle: status `none` or `inactive`.

After all checks, a summary line reports the count of required items
missing.

### 3.4 Web API

`GET /api/doctor` (protected, requires `X-Credential` header) returns
the same checks as a JSON array. Each element is an object with fields:
`name` (string), `status` (string: `ok`, `missing`, `none`, `inactive`,
`error`), `detail` (string), `severity` (string), `help` (string).

---

## 4. Join Code Generation

A join code is a short, human-readable, single-use token that authorises
a new device to join the colony trust group. Codes are valid for five
minutes (300 seconds, `DEFAULT_JOIN_CODE_TTL_SECS`).

### 4.1 Code Format

A join code is derived from 6 random bytes (48 bits of entropy) and
displayed in the format `xxxx-xxxx-xxxx` where each group is four
hexadecimal characters. Internally, the 6 bytes are stored in the first
6 positions of a 16-byte array; the remaining 10 bytes are zero-padded.

Example: `a1b2-c3d4-e5f6`

Parsing MUST accept both the short format (`xxxx-xxxx-xxxx`, 6 bytes
extracted) and the full 16-byte format (`xxxx-xxxx-xxxx-xxxx`, backwards
compatibility). Non-hex characters (including dashes) are stripped
before decoding.

### 4.2 CLI Generation

#### `--join-code`

```
anthill --join-code [CONFIG_DIR]
```

Default config directory: `~/.config/anthill`. Loads the colony trust
group, generates a code, persists it to `join-codes.toml`, and prints:

```
  Join code:  a1b2-c3d4-e5f6
  Expires in: 5 minutes

  Enter this in the Anthill web dashboard to join the colony.
```

#### `--qr-join`

```
anthill --qr-join [CONFIG_DIR] [--hostname <host>]
```

Generates a join code and renders a QR code in the terminal using
Unicode half-block characters. The QR code encodes a URL of the form:

```
http://<hostname>:<port>/#join=<code>
```

The hostname defaults to the output of the `hostname` command. The port
is read from `supervisor.toml` (`http_port`, default `3000`).

Output:

```
  Scan this QR code to join the colony:

  [QR code rendered in terminal]

  URL: http://myhost:3000/#join=a1b2-c3d4-e5f6
  Code: a1b2-c3d4-e5f6  (expires in 5 minutes)
```

### 4.3 Web API Generation

`GET /api/auth/qr-join` (protected) generates a join code and returns
JSON:

```json
{
  "code": "a1b2-c3d4-e5f6",
  "url": "http://host:3000/#join=a1b2-c3d4-e5f6",
  "svg": "<svg ...>...</svg>"
}
```

The `svg` field contains a rendered QR code (minimum 200x200 dimensions,
dark `#000000` on light `#ffffff`). The URL is derived from the
request's `Host` header.

### 4.4 Join Code Persistence

Join codes are persisted to `~/.config/anthill/join-codes.toml` as
lines of the form `<formatted-code> <expiry-unix-timestamp>`. This
enables CLI-generated codes to be consumed by the running web server
and vice versa. On load, expired codes (expiry <= current time) are
skipped. On consumption, the code is removed from disk.

---

## 5. Device Provisioning Flow

### 5.1 Initial Load

When a browser navigates to the Anthill dashboard:

1. The app checks `localStorage` for `anthill_credential`.
2. If no credential is stored, the **join screen** is displayed.
3. If a credential exists, `POST /api/auth/verify` is called with
   `{ "credential": "<value>" }`.
   - If `{ "authenticated": true, "device_name": "<name>" }` is
     returned, the main app is shown.
   - If `{ "authenticated": false }` is returned, the credential is
     removed from `localStorage` and the join screen is displayed.
   - If the server is unreachable, the app is shown optimistically
     (it will reconnect via WebSocket).

### 5.2 Join Screen

The join screen is a full-viewport overlay (`z-index: 300`) containing:

- The Anthill logo (280px wide).
- Instructional text: "Enter a join code to connect to this colony."
- A join code input field with placeholder `xxxx-xxxx-xxxx`, centred,
  monospace font, 18px, letter-spacing 2px.
- A device name input field with placeholder "Device name (e.g. My
  Phone)".
- An error display area (hidden by default, red text).
- A "Join Colony" button (accent colour, full width, 16px bold).
- Help text: "Generate a code on the server: `anthill --join-code
  ~/.config/anthill`".

### 5.3 Join Submission

When the user clicks "Join Colony" or presses Enter:

1. The join code and device name are read from the form. If the device
   name is empty, it defaults to `"unnamed device"`.
2. If the code field is empty, the error "Enter a join code." is
   displayed and submission is aborted.
3. `POST /api/auth/join` is called with `{ "code": "<code>",
   "device_name": "<name>" }`. This endpoint is public (no auth
   required).
4. The server validates and consumes the join code via
   `ColonyTrust::join_with_code()`. This:
   - Reloads join codes from disk (in case CLI wrote them).
   - Parses the code and generates a new Ed25519 device keypair.
   - Calls `TrustGroup::process_join_request()` to validate the code,
     issue a device certificate, and register the device as a member.
   - Persists the updated device registry to `devices.toml`.
   - Removes the consumed code from `join-codes.toml`.
5. On success, the server returns:
   ```json
   {
     "device_id": "<hex-ed25519-public-key>",
     "credential": "<hex-ed25519-private-key-seed>",
     "name": "<device-name>"
   }
   ```
6. The client stores `credential`, `device_name`, and `device_id` in
   `localStorage` keys `anthill_credential`, `anthill_device_name`, and
   `anthill_device_id` respectively.
7. The main app is displayed and a WebSocket connection is established.

On failure (invalid or expired code), the server returns
`422 Unprocessable Entity` with body `"Invalid or expired join code"`.
The client displays this error in the join screen error area.

### 5.4 Hash-Based Join

If the URL contains a fragment of the form `#join=<code>` (as generated
by QR scan), the join screen is displayed and the code input is
pre-filled with the code value after a 100ms delay.

### 5.5 Authentication for Protected Routes

All protected API routes are guarded by `auth_middleware`. The
credential MUST be sent in the `X-Credential` HTTP header. The
middleware calls `ColonyTrust::authenticate()`, which derives the
Ed25519 public key from the credential seed and looks up the
corresponding member in the trust group. If authentication fails,
`401 Unauthorized` is returned.

### 5.6 WebSocket Authentication

WebSocket connections pass the credential and device ID as query
parameters: `ws://<host>/ws?credential=<cred>&device_id=<id>`. On
HTTPS connections, messages are HMAC-SHA256 signed using the credential
as key, with the format `HMAC(credential, device_id + ":" + timestamp
+ ":" + payload)`. Signed messages older than 60 seconds
(`MAX_MESSAGE_AGE_SECS`) are rejected.

---

## 6. Empty Colony State

### 6.1 Public Status Endpoint

`GET /api/auth/status` is a public endpoint (no auth required) that
returns:

```json
{
  "empty_colony": true
}
```

The `empty_colony` field is `true` when no devices have been provisioned
(`TrustGroup::is_empty()`).

### 6.2 Dashboard Display

When a device is provisioned but no ANTs exist:

- The sidebar ANT list (`#bot-list`) is empty.
- The main panel header displays "Select an ANT".
- The main content area displays the text: "Select an ANT from the
  sidebar to start chatting".
- The "+" button in the sidebar header is visible, inviting the user
  to create their first ANT.

When the knowledge graph tab is active for an ANT that has no graph
data:

- The graph panel displays: "No knowledge graph yet. Chat with the ANT
  to build one."

---

## 7. First ANT Creation

### 7.1 Create Modal

Clicking the "+" button in the sidebar opens the create modal
(`#create-modal`), a centred overlay with max-width 480px.

The modal title is "New ANT".

### 7.2 Fields

| Field | Input ID | Required | Type | Placeholder | Default | Validation |
|-------|----------|----------|------|-------------|---------|------------|
| ID | `f-id` | Yes | text | `my-ant` | (none) | Non-empty; alphanumeric, hyphens, underscores only |
| Display Name | `f-name` | No | text | `My Dev ANT` | Defaults to the ID value |  |
| Bot Token (Telegram) | `f-token` | No | text | `123456:ABC-DEF...` | Empty (web-only mode) |  |
| Working Directory | `f-workdir` | No | text | (empty) | `~/.config/anthill/ants/<id>/working` |  |
| System Prompt | `f-prompt` | No | textarea (3 rows, resizable) | `You are a helpful assistant.` | (empty) |  |

Fields are grouped under section headers:
- (no header) -- ID and Display Name.
- "Telegram (optional)" -- Bot Token.
- "Workspace (optional)" -- Working Directory.
- "Personality (optional)" -- System Prompt.

### 7.3 Validation

Client-side:
- If the ID field is empty, the error "ID is required." is displayed
  in the modal error area and submission is aborted.

Server-side (`POST /api/ants/create`):
- The ID MUST be non-empty.
- The ID MUST contain only alphanumeric characters, hyphens, or
  underscores.
- If validation fails, `400 Bad Request` is returned with body
  "Invalid ANT id. Use alphanumeric, hyphens, underscores."
- If an ANT with the same ID already exists, `409 Conflict` is returned
  with body "ANT already exists".

### 7.4 Creation Process

1. The client sends `POST /api/ants/create` with JSON body:
   ```json
   {
     "id": "dev",
     "name": "Dev Assistant",
     "token": "",
     "working_dir": "",
     "system_prompt": ""
   }
   ```
   Empty optional fields are sent as empty strings; the server treats
   them as absent.

2. The server builds a `Config` struct with the provided values. Empty
   strings for `name`, `token`, `working_dir`, and `system_prompt` are
   converted to `None`. If `name` is `None`, it defaults to the ID.

3. The config is serialised to TOML and written to
   `~/.config/anthill/ants/<id>/ant.toml`.

4. `POST /api/ants/reload` is called to tell the supervisor to scan
   for new ANT configurations and spawn them.

5. The client refreshes the bot list. The new ANT appears in the
   sidebar with a green status dot.

### 7.5 Keyboard Shortcuts

- **Enter** (when the create modal is open and focus is not in a
  textarea): submits the form.
- **Escape**: closes the modal.

---

## 8. First Interaction

### 8.1 Sending a Message

1. The user selects the newly created ANT in the sidebar. The main
   panel header updates to show the ANT's display name. The chat area
   becomes visible with the input bar at the bottom.

2. The user types a message in the input field and presses Enter (or
   the send button).

3. The message is sent via `POST /api/ants/<id>/chat` with JSON body
   `{ "message": "<text>", "chat_id": 0 }`.

4. The message appears in the chat area as a user message (right-
   aligned, `--surface2` background).

### 8.2 Thinking Indicator

While the AI worker is processing:

- The Workers tab shows the active task.
- If no task items are rendered and the ANT is in a typing state, the
  workers panel displays "Thinking..." in dim italic text.
- The task bar in the header MAY show task status.

### 8.3 Receiving a Response

When the AI worker completes:

1. The response arrives via WebSocket as a message event.
2. It is rendered as a bot message (left-aligned, `--surface`
   background) with Markdown formatting (via the `marked` library).
3. Code blocks are rendered with syntax highlighting and a monospace
   font.

---

## 9. Device Management

### 9.1 Devices Modal

The "Manage Devices" button at the bottom of the sidebar opens the
devices modal (`#devices-modal`), a centred overlay titled "Connected
Devices".

### 9.2 Device List

`GET /api/auth/devices` (protected) returns a JSON array of device
objects:

```json
[
  {
    "id": "<hex-ed25519-public-key>",
    "name": "<device-name>",
    "joined_at": 1711756800,
    "last_seen": 1711756900
  }
]
```

The credential field is intentionally omitted from the list response --
public keys are not credentials.

Each device is rendered as a row showing:
- Device name (bold). If the device matches the current session's
  credential, "(this device)" is shown in green.
- Join timestamp and last-seen timestamp as relative time strings
  (e.g. "2h ago", "3d ago").
- A "Revoke" button (red outline) -- not shown for the current device.

### 9.3 Revoking a Device

Clicking "Revoke" on a device triggers a confirmation dialog:
`Revoke "<name>"? They will need a new join code to reconnect.`

On confirmation, `DELETE /api/auth/devices/<id>` is called. The server
calls `ColonyTrust::revoke_device()`, which:

1. Decodes the hex device ID to a 32-byte public key.
2. Calls `TrustGroup::revoke_device()` with reason `ForcedRemoval`.
3. Removes the device from the `last_seen` map.
4. Persists the updated device registry to `devices.toml`.

On success, `200 OK` is returned and the device list is refreshed. On
failure (unknown device ID), `404 Not Found` is returned.

### 9.4 QR Join from Dashboard

Clicking "Add Device (QR)" in the devices modal calls `GET
/api/auth/qr-join` and displays:

- The QR code SVG (white background, 12px padding, 8px border-radius).
- The join code in monospace, 18px, accent colour, letter-spacing 2px.
- A countdown timer.

#### 9.4.1 Countdown Timer

The timer starts at 300 seconds (5 minutes) and decrements every
second. Display format: `Scan to join. Expires in M:SS`.

Timer colour changes:
- Default (`--text-dim`): more than 60 seconds remaining.
- Yellow (`--yellow`): 60 seconds or fewer remaining.
- Red (`--red`): expired (0 seconds remaining).

When expired, the timer text changes to: "Expired -- tap Add Device
(QR) for a new code". The interval is cleared.

When the devices modal is closed, the QR section is hidden and the
countdown interval is cleared.

---

## 10. Conformance

An implementation claiming conformance to ANTHILL-ONBOARDING:

1. MUST provide an installation mechanism that builds the binary,
   places it in the system PATH, creates the config directory, and
   registers a platform-appropriate service.

2. MUST implement the `--doctor` command that checks all items listed
   in Section 3.2 and reports results with severity levels.

3. MUST generate join codes in the `xxxx-xxxx-xxxx` format with 48
   bits of entropy and a 300-second TTL.

4. MUST support both CLI (`--join-code`, `--qr-join`) and web API
   (`/api/auth/qr-join`) join code generation.

5. MUST persist join codes to disk so that CLI-generated codes are
   consumable by the web server and vice versa.

6. MUST implement the device provisioning flow described in Section 5:
   join screen, code submission, credential storage in `localStorage`,
   and hash-based pre-fill from QR scan.

7. MUST guard all protected API routes with credential-based
   authentication via the `X-Credential` header.

8. MUST display the empty colony state described in Section 6 when no
   ANTs are configured.

9. MUST implement the create ANT modal with the fields and validation
   rules described in Section 7.

10. MUST implement the device management modal with listing, revocation,
    and QR countdown timer as described in Section 9.

---

## 11. References

- ANTHILL-TRUST -- Trust groups, device certificates, Ed25519 keys.
- ANTHILL-DASHBOARD -- Web dashboard layout and real-time channels.
- ANTHILL-COLONY -- Colony supervisor, ANT lifecycle management.
- R2-PROVISION -- R2 device provisioning specification.
- R2-TRUST -- R2 trust group lifecycle and join code protocol.
- RFC 2119 -- Requirement level keywords.
