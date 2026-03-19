# ANTHILL-COLONY: Colony Architecture

**Version:** 0.1 Draft
**Date:** 2026-03-20
**Status:** Draft
**Depends on:** ANTHILL-INTRO, R2-TRUST, R2-SENTANT, R2-PLUGIN

---

## 1. Introduction

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT", "SHOULD", "SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this document are to be interpreted as described in RFC 2119.

An Anthill **colony** is a single server instance hosting one or more ANTS. This specification defines the colony lifecycle, ANT structure, supervision model, trust group integration, and device provisioning.

### 1.1 Terminology

| Term | Definition |
|------|-----------|
| **Colony** | A running Anthill server instance. One trust group, one supervisor, one or more ANTS. |
| **ANT** | Autonomous iNTelligenceS. An AI agent with its own identity, memory, and working directory. |
| **Supervisor** | The top-level process that discovers, spawns, and monitors ANTS. |
| **Queen** | The server itself. The key holder of the colony trust group. |
| **Device** | A client (phone, laptop) that has joined the colony trust group. |
| **Join code** | A short-lived token used to provision a new device into the colony. |

---

## 2. Colony Lifecycle

### 2.1 Startup

The supervisor MUST:

1. Load `supervisor.toml` from the config directory (or use defaults).
2. Discover ANT configurations by scanning the `ants/` subdirectory for `ant.toml` files.
3. For each discovered ANT, load its configuration and spawn it on a dedicated thread.
4. Load the colony trust group from `colony.key` and `devices.toml`.
5. Start the web server.

If no ANTS are discovered, the supervisor MUST still start the web server so that ANTS can be created via the dashboard.

### 2.2 ANT States

An ANT MUST be in exactly one of these states:

| State | Description |
|-------|-------------|
| **Configured** | `ant.toml` exists on disk but the ANT is not running. |
| **Running** | The ANT's event bus is active and processing events. |
| **Stopped** | The ANT was running but has exited (crash or shutdown). |
| **Error** | The ANT failed to start or encountered a fatal error. |

The web dashboard MUST distinguish between Configured and Running ANTS in its listing.

### 2.3 Crash Recovery

When a Running ANT stops unexpectedly:

1. The supervisor MUST mark it as Stopped in the registry.
2. If `restart_on_crash` is true, the supervisor MUST restart the ANT after `restart_delay_secs × attempt_number` seconds.
3. If `max_restarts` is exceeded, the supervisor MUST NOT restart the ANT and MUST log an error.
4. Restart attempts reset when the ANT runs successfully for longer than 60 seconds.

### 2.4 Hot-Add

New ANTS created via the web dashboard or filesystem:

1. The web server signals the supervisor via a reload channel.
2. The supervisor re-scans the `ants/` directory.
3. New ANTS (present on disk but not in the running set) are spawned.
4. Existing ANTS are NOT restarted.

---

## 3. ANT Structure

### 3.1 R2 Event Bus

Each ANT runs an R2 event bus on a dedicated thread (using a `LocalSet` for the `!Send` EventBus). The bus contains:

- **One conductor sentant** (pure FSM) — routes commands to plugin calls.
- **One AI plugin** — manages the AI worker, message dispatch, task tracking.
- **Zero or one Telegram plugin** — bridges Telegram bot API.
- **Zero or one Slack plugin** — bridges Slack Socket Mode API.

### 3.2 Working Directory

Each ANT MUST have a working directory with the following structure:

```
working/
├── memory/
│   ├── knowledge.json        # Popperian knowledge graph (ANTHILL-MEMORY)
│   ├── knowledge-archive.json # Archived low-confidence edges
│   ├── episodes.json          # Episodic memory (conversation summaries)
│   └── {chat_id}.md          # Per-user freeform memory
├── files/                    # User-uploaded files
├── repos/                    # Cloned git repositories (excluded from backup)
└── .git/                     # Auto-managed backup repository
```

The `repos/` directory MUST be excluded from git backup via `.gitignore` (repositories have their own version control).

### 3.3 Configuration

ANT configuration is stored in `ant.toml` using the following schema:

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `name` | string | directory name | Display name in the web UI |
| `telegram.token` | string | env `TELOXIDE_TOKEN` | Telegram bot token |
| `telegram.allow` | int[] | [] (all) | Allowed Telegram chat IDs |
| `slack.bot_token` | string | — | Slack bot token (xoxb-...) |
| `slack.app_token` | string | — | Slack app-level token (xapp-...) |
| `claude.backends` | string[] | ["claude"] | AI backends in priority order |
| `claude.working_dir` | string | auto | Working directory path |
| `claude.system_prompt` | string | — | ANT personality prompt |
| `claude.skip_permissions` | bool | true | Skip AI permission prompts |
| `claude.sync_channels` | bool | false | Forward messages across channels |
| `claude.encrypt_backups` | bool | false | Encrypt memory/ and files/ in git |
| `claude.backup_interval_hours` | int | 0 | Auto-backup interval (0 = disabled) |
| `claude.worker_timeout_secs` | int | 600 | Kill workers after this many seconds idle |

---

## 4. Trust Group

### 4.1 Colony as Trust Group

The colony is an R2-TRUST trust group. The server is the key holder. Client devices are members.

- **Colony key** (`colony.key`): Ed25519 signing key. Generated on first run.
- **Device registry** (`devices.toml`): Persisted device certificates and metadata.
- **Join codes** (`join-codes.toml`): Short-lived provisioning tokens, shared between CLI and server.

### 4.2 Device Provisioning

Three provisioning methods:

1. **QR scan** (web UI): Authenticated user generates a QR code containing `http://<host>:<port>/#join=<code>`. New device scans, opens web app, joins automatically.
2. **QR scan** (CLI): `anthill --qr-join` generates a QR code in the terminal.
3. **Manual code**: `anthill --join-code` generates a text code. User enters it in the web app's join screen.

Join codes:
- MUST be 48 bits of entropy (displayed as `xxxx-xxxx-xxxx` hex).
- MUST expire after 300 seconds (5 minutes).
- MUST be single-use (consumed on successful join).
- MUST be persisted to disk so CLI-generated codes are visible to the running server.

### 4.3 Authentication

All protected API endpoints require an `X-Credential` header containing the device's hex-encoded Ed25519 private key seed.

WebSocket messages are signed with HMAC-SHA256 (using the credential as the HMAC key) for transport integrity. The server verifies signatures and rejects stale messages (>60 seconds).

**Two distinct layers:**
- **Device identity**: Ed25519 via R2-TRUST (certificates, join protocol).
- **Transport signing**: HMAC-SHA256 (browser `crypto.subtle` compatibility).

### 4.4 First Device Bootstrap

The first device cannot join via the web UI (the UI requires authentication). The first device MUST be provisioned via the CLI:

```
anthill --qr-join --hostname <tailscale-hostname>
```

Subsequent devices can be provisioned from the web UI by an already-authenticated device.

---

## 5. Supervisor Configuration

`supervisor.toml` schema:

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `ants_dir` | string | "ants" | Subdirectory containing ANT configs |
| `restart_on_crash` | bool | true | Auto-restart crashed ANTS |
| `restart_delay_secs` | int | 5 | Base restart delay (multiplied by attempt) |
| `max_restarts` | int | 10 | Maximum consecutive restarts (0 = unlimited) |
| `http_port` | int | 3000 | Web dashboard port |
| `http_bind` | string | "0.0.0.0" | Bind address |

---

## 6. Security Considerations

1. **Colony key is the root of trust.** If compromised, all device credentials and encrypted backups are compromised. The key MUST be stored securely and backed up to a password manager.
2. **Join codes are short-lived** but grant permanent colony membership. The 5-minute window limits exposure.
3. **WebSocket signing** prevents message tampering but does not encrypt content. Use HTTPS (via Tailscale `serve`) for confidentiality.
4. **Telegram/Slack tokens** are stored in plaintext in `ant.toml`. File permissions SHOULD restrict access to the owning user.
