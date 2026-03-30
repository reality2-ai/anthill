# ANTHILL-TRUST: Colony Security and Device Provisioning

| Field      | Value                                                        |
|------------|--------------------------------------------------------------|
| Version    | 0.1 Draft                                                    |
| Date       | 2026-03-30                                                   |
| Status     | Draft                                                        |
| Depends on | R2-TRUST, R2-PROVISION                                       |
| Related    | ANTHILL-COLONY, ANTHILL-ONBOARDING, ANTHILL-WEB              |

> The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT",
> "SHOULD", "SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this
> document are to be interpreted as described in RFC 2119.

---

## 1. Introduction

Anthill implements the R2-TRUST specification to secure colony
communications and control device access. The colony operates as an
R2-TRUST trust group: the server is the key holder (the queen) and
every browser, phone, or CLI session is a provisioned device (a viewer)
that MUST present a valid credential to interact.

Security is defence in depth. Anthill applies seven layers, each
independently enforceable:

1. **Network encryption** -- Tailscale (WireGuard) or equivalent VPN.
2. **Transport encryption** -- HTTPS via Tailscale certificates or a
   reverse proxy.
3. **Trust group authentication** -- R2-TRUST colony membership with
   Ed25519 device credentials.
4. **HMAC envelope signing** -- HMAC-SHA256 signatures on every
   WebSocket message.
5. **R2-WIRE event limit** -- 256-byte maximum event size prevents
   prompt injection via the event bus.
6. **Validated writes** -- all knowledge graph mutations pass through
   the Thurisaz engine; AI workers MUST NOT edit graph files directly.
7. **Channel-level restrictions** -- sensitive commands are restricted
   to the web dashboard, which carries the strongest authentication.

### 1.1 Terminology

| Term              | Definition                                                                                             |
|-------------------|--------------------------------------------------------------------------------------------------------|
| Colony key        | An Ed25519 signing key (`colony.key`) held by the server. The root of trust for the colony.            |
| Device credential | An Ed25519 private key seed (hex-encoded, 64 hex characters / 32 bytes) issued to a device on joining. |
| Join code         | A 48-bit single-use token formatted as `xxxx-xxxx-xxxx`, valid for 5 minutes (300 seconds).            |
| HMAC envelope     | An HMAC-SHA256 signed wrapper around every WebSocket message, providing integrity and replay protection.|
| Queen             | The server process that holds the colony key and issues device credentials.                            |
| Viewer            | A provisioned device (browser, phone, CLI) that has been issued a credential.                          |
| Trust group       | An R2-TRUST boundary governing which devices MAY communicate with the colony.                          |

---

## 2. Colony Key

### 2.1 Key Type

The colony key MUST be an Ed25519 signing key as defined by RFC 8032.
The key is stored as a 32-byte seed, hex-encoded, in the file
`colony.key`.

### 2.2 Generation

On first run, if `colony.key` does not exist, the server MUST generate
a new Ed25519 signing key using a cryptographically secure random number
generator (`OsRng`). The server MUST persist the hex-encoded 32-byte
seed to `colony.key` before accepting any connections.

### 2.3 Storage

The colony key MUST be stored at:

```
~/.config/anthill/colony.key
```

The file MUST contain exactly 64 hexadecimal characters (the 32-byte
Ed25519 seed). On load, the server MUST verify that the decoded seed is
exactly 32 bytes; if not, the server MUST refuse to start.

### 2.4 Load or Create

The server follows a load-or-create pattern:

1. If `colony.key` exists, read the hex seed and restore the
   `TrustGroup` from the existing key and the device registry
   (`devices.toml`).
2. If `colony.key` does not exist, generate a new `TrustGroup`,
   persist the seed, and start with an empty device registry.

### 2.5 Key Export and Import

The CLI MUST support the following flags for key management:

- `--export-key` -- print the hex-encoded colony key seed to stdout.
- `--import-key <hex>` -- write the provided seed to `colony.key`,
  replacing any existing key.

Key import MUST overwrite any existing `colony.key`. Implementations
SHOULD warn the operator that importing a new key invalidates all
previously issued device credentials.

### 2.6 Key Protection

The colony key is the root of trust. Anyone with access to `colony.key`
can generate join codes and provision devices. Operators MUST restrict
filesystem permissions on the config directory. The key MUST NOT be
committed to version control or transmitted over unencrypted channels.

---

## 3. Device Provisioning

Device provisioning follows a four-step flow: generate, present, join,
store.

### 3.1 Generate Join Code

The server or CLI generates a join code as follows:

1. Generate 6 random bytes (48 bits of entropy) using `OsRng`.
2. Pad to a 16-byte value (remaining 10 bytes set to zero) for
   compatibility with the `r2_trust::join::JoinCode` structure.
3. Set the expiry to the current time plus `DEFAULT_JOIN_CODE_TTL_SECS`
   (300 seconds / 5 minutes).
4. Format the first 6 bytes as a human-readable string:
   `xxxx-xxxx-xxxx` (three groups of 4 hex characters, hyphen-separated).
5. Inject the code into the `TrustGroup` and persist active codes to
   `join-codes.toml` for CLI-to-server handoff.

A join code MUST be single-use. Once consumed by a successful join, the
code MUST be removed from the active set and from disk.

A join code MUST expire after 300 seconds. The server MUST reject any
join attempt with an expired code.

### 3.2 Present Join Code

The join code MAY be presented to the user via:

- **QR code** -- the `GET /api/auth/qr-join` endpoint (authenticated)
  returns an SVG QR code encoding the join URL with the code embedded.
  The web dashboard displays this with a 5-minute countdown timer.
- **Text code** -- the CLI `--join-code` flag prints the formatted code
  to stdout. The CLI `--qr-join` flag prints a terminal QR code.
- **Custom hostname** -- `--qr-join --hostname <host>` overrides the
  hostname embedded in the QR URL.

### 3.3 Join

A client joins the colony by sending its join code and a device name:

1. The client sends `POST /api/auth/join` with JSON body:
   ```json
   { "code": "xxxx-xxxx-xxxx", "device_name": "My Phone" }
   ```
2. The server reloads `join-codes.toml` from disk (in case a CLI
   process wrote the code).
3. The server parses the code. Both short format (`xxxx-xxxx-xxxx`,
   6 bytes) and full format (`xxxx-xxxx-xxxx-xxxx`, 16 bytes) MUST be
   accepted for backwards compatibility.
4. The server generates a new Ed25519 keypair for the device using
   `OsRng`.
5. The server calls `TrustGroup::process_join_request`, which validates
   the code, consumes it (single-use), and issues an R2-TRUST device
   certificate with a TTL of `DEFAULT_CERT_TTL_SECS`.
6. On success, the server returns:
   ```json
   {
     "device_id": "<hex-encoded Ed25519 public key>",
     "credential": "<hex-encoded Ed25519 private key seed>",
     "name": "<device name>"
   }
   ```
7. On failure (invalid, expired, or already-consumed code), the server
   MUST return HTTP 403 Forbidden.

If `device_name` is empty, the server MUST default to `"unnamed device"`.

### 3.4 Store

After a successful join:

- **Client side:** the credential (hex-encoded Ed25519 seed) and
  device ID MUST be stored in the browser's `localStorage`. The
  credential MUST be sent with every subsequent API request.
- **Server side:** the device record (public key, name, certificate,
  join timestamp, last-seen timestamp) MUST be persisted to
  `devices.toml` in the config directory. The format is TOML with a
  `[devices]` table keyed by hex-encoded public key.

### 3.5 Bootstrap (Empty Colony)

When the colony has no provisioned devices (`TrustGroup` is empty), the
`GET /api/auth/status` endpoint returns `{ "empty": true }`. The web
dashboard MUST detect this state and guide the user through initial
provisioning, typically by displaying a join code generated from the
CLI.

---

## 4. Authentication

### 4.1 Public Routes (No Authentication)

The following routes MUST NOT require authentication:

| Method | Path                | Purpose                                 |
|--------|---------------------|-----------------------------------------|
| GET    | `/`                 | HTML page, static assets, icons         |
| GET    | `/manifest.json`    | PWA manifest                            |
| GET    | `/sw.js`            | Service worker                          |
| GET    | `/icon-*.svg`       | Application icons                       |
| GET    | `/logo.svg`         | Application logo                        |
| GET    | `/vendor/*`         | Vendored JavaScript libraries           |
| POST   | `/api/auth/join`    | Join colony with a join code            |
| POST   | `/api/auth/verify`  | Check if a credential is valid          |
| GET    | `/api/auth/status`  | Query whether the colony is empty       |

### 4.2 Protected Routes (Authentication Required)

All other `/api/*` routes MUST require a valid credential. The
credential MUST be provided in the `X-Credential` HTTP header.

Protected routes include but are not limited to:

| Method | Path                             | Purpose                        |
|--------|----------------------------------|--------------------------------|
| GET    | `/api/ants`                      | List all ANTs                  |
| POST   | `/api/ants/{id}/chat`            | Send message to an ANT         |
| GET    | `/api/ants/{id}/graph`           | Retrieve knowledge graph       |
| GET    | `/api/ants/{id}/export`          | Export knowledge graph          |
| POST   | `/api/ants/create`               | Create a new ANT               |
| DELETE | `/api/ants/{id}`                 | Delete an ANT                  |
| GET    | `/api/auth/devices`              | List provisioned devices       |
| DELETE | `/api/auth/devices/{id}`         | Revoke a device                |
| GET    | `/api/auth/qr-join`              | Generate QR join code          |
| GET    | `/api/backends`                  | List AI backends               |
| GET    | `/api/doctor`                    | System health check            |

### 4.3 WebSocket Authentication

The WebSocket endpoint `GET /ws` requires authentication via query
parameters:

```
/ws?credential=<hex>&device_id=<hex>
```

The server MUST verify the credential before upgrading the connection.
If the credential is invalid or missing, the server MUST return HTTP
401 Unauthorized and MUST NOT upgrade to WebSocket.

### 4.4 Auth Middleware

Protected API routes are guarded by an Axum middleware layer that:

1. Extracts the `X-Credential` header value.
2. If the header is missing or empty, returns HTTP 401 Unauthorized.
3. Derives the Ed25519 public key from the credential (treating the
   64-hex-character credential as a private key seed).
4. Looks up the derived public key in the `TrustGroup` member list.
5. If the member is found, updates the last-seen timestamp and allows
   the request to proceed.
6. If the member is not found, returns HTTP 401 Unauthorized.

The credential MUST be 64 hexadecimal characters (32 bytes). Any other
length MUST be rejected.

### 4.5 Credential Verification Endpoint

`POST /api/auth/verify` accepts `{ "credential": "<hex>" }` and returns:

- `{ "authenticated": true, "device_name": "<name>" }` if valid.
- `{ "authenticated": false }` if invalid.

This endpoint MUST NOT consume any tokens or modify server state (aside
from updating last-seen). It is used by clients to check stored
credentials on page load.

---

## 5. WebSocket Security

### 5.1 HMAC-SHA256 Envelope

Every WebSocket message -- both client-to-server and server-to-client
-- MUST be wrapped in a signed envelope. The envelope format is:

```json
{
  "device_id": "<hex public key or 'server'>",
  "timestamp": <unix timestamp in seconds>,
  "signature": "<hex HMAC-SHA256>",
  "payload": "<JSON string>"
}
```

### 5.2 Signing

The HMAC-SHA256 signature is computed as:

```
HMAC-SHA256(credential_bytes, device_id + ":" + timestamp + ":" + payload)
```

Where:

- `credential_bytes` is the raw bytes of the hex-decoded credential.
- `device_id` is the sender's hex-encoded public key (or `"server"` for
  server-originated messages).
- `timestamp` is the current Unix time in seconds.
- `payload` is the JSON-encoded message content as a string.
- The three fields are concatenated with `:` (colon) separators.

The resulting HMAC tag is hex-encoded for the `signature` field.

### 5.3 Verification

On receiving a WebSocket message, the server MUST:

1. Parse the message as a `WsEnvelope` (JSON object with `device_id`,
   `timestamp`, `signature`, and `payload` fields).
2. Check that the absolute difference between the message timestamp and
   the current server time does not exceed `MAX_MESSAGE_AGE_SECS`
   (60 seconds). If it does, the server MUST reject the message.
3. Recompute the HMAC-SHA256 over `device_id + ":" + timestamp + ":"
   + payload` using the device's credential as the key.
4. Compare the recomputed tag to the provided signature using
   constant-time comparison. If they do not match, the server MUST
   reject the message and log a warning.
5. If the envelope is valid, extract `payload` and process it as a
   WebSocket command.

If a message does not parse as a signed envelope, the server MAY fall
back to treating the raw text as a plain command for backwards
compatibility, provided the WebSocket connection was already
authenticated at upgrade time.

### 5.4 Replay Protection

The 60-second timestamp window (`MAX_MESSAGE_AGE_SECS`) provides replay
protection. Messages older than 60 seconds MUST be rejected. Clock skew
between client and server MUST be less than 60 seconds for signed
messages to be accepted.

### 5.5 Server-to-Client Signing

The server signs outgoing messages using the device's credential as the
HMAC key and `"server"` as the `device_id`. This allows clients to
verify that messages originate from the authenticated server and have
not been tampered with in transit.

---

## 6. Device Management

### 6.1 List Devices

The `GET /api/auth/devices` endpoint (authenticated) returns a JSON
array of all provisioned devices. Each device record includes:

| Field       | Type   | Description                                      |
|-------------|--------|--------------------------------------------------|
| `id`        | string | Hex-encoded Ed25519 public key (64 characters).   |
| `name`      | string | Human-readable device name.                       |
| `joined_at` | number | Unix timestamp when the device joined the colony. |
| `last_seen` | number | Unix timestamp of the device's last API request.  |

The `credential` field MUST be empty in list responses. The server MUST
NOT return device credentials (private key seeds) in any list or query
operation.

Devices MUST be returned sorted by `joined_at` in ascending order.

### 6.2 Revoke Device

The `DELETE /api/auth/devices/{id}` endpoint (authenticated) revokes a
device by its hex-encoded public key. On revocation:

1. The server calls `TrustGroup::revoke_device` with reason
   `ForcedRemoval`.
2. The device's last-seen record is removed.
3. The updated device registry is persisted to `devices.toml`.
4. The server returns HTTP 200 on success.
5. If the device ID is invalid or not found, the server returns HTTP
   404 Not Found.

A revoked device MUST NOT be able to authenticate. To rejoin, the
device requires a new join code and receives a new credential.

### 6.3 Device Metadata

Each device record persisted to `devices.toml` MUST include:

- `public_key` -- hex-encoded Ed25519 public key.
- `name` -- human-readable name provided at join time.
- `certificate` -- hex-encoded R2-TRUST device certificate bytes.
- `joined_at` -- Unix timestamp of provisioning.
- `last_seen` -- Unix timestamp of most recent authenticated request.

### 6.4 Persistence Across Restarts

The device registry MUST survive server restarts. On startup, the
server MUST load `devices.toml` and reconstruct the `TrustGroup`
member list from the stored certificates. Join codes that have not
expired MUST be loaded from `join-codes.toml`.

---

## 7. Transport Security

Anthill employs a layered approach to transport security. Each layer is
independently valuable; together they provide defence in depth.

### 7.1 Tailscale (WireGuard)

Operators SHOULD deploy Anthill behind Tailscale or an equivalent
WireGuard-based mesh VPN. This provides:

- Authenticated, encrypted network tunnels between all devices.
- No exposed ports on the public internet.
- Identity tied to the Tailscale account (MagicDNS, ACLs).

This is the RECOMMENDED deployment model.

### 7.2 HTTPS

Operators SHOULD enable HTTPS, either via:

- `tailscale serve` -- automatically provisions TLS certificates for
  the Tailscale hostname.
- A reverse proxy (nginx, Caddy) with certificates from Let's Encrypt
  or another CA.

HTTPS protects against passive eavesdropping and active interception on
the transport layer.

### 7.3 Trust Group Authentication

Above the transport layer, the R2-TRUST trust group provides
application-level authentication. Even if an attacker gains network
access (e.g. compromises the VPN), they cannot interact with the colony
without a valid device credential obtained through the join code flow.

### 7.4 XChaCha20-Poly1305 Backup Encryption

When `encrypt_backups = true` is set in `ant.toml`, the server encrypts
`memory/` and `files/` directories using XChaCha20-Poly1305 before each
git commit. The encryption key is derived by taking the SHA-256 hash of
the credential bytes. A random 24-byte nonce is generated per
encryption operation using `OsRng`. The ciphertext format is
`base64(nonce || ciphertext)`.

This layer is OPTIONAL. It protects knowledge graph data at rest in git
repositories, which is important when backup remotes are hosted on
third-party services.

---

## 8. Security Boundaries

Not all communication channels provide the same security guarantees.
The following table summarises what each channel provides:

| Channel        | Network Encryption   | Transport TLS | Trust Group Auth | HMAC Signing | Sensitive Ops |
|----------------|----------------------|---------------|------------------|--------------|---------------|
| Web dashboard  | Tailscale (RECOMMENDED) | Yes (RECOMMENDED) | Yes (REQUIRED) | Yes (REQUIRED) | Allowed |
| Telegram       | Third-party TLS only | Telegram servers | No             | No           | Blocked       |
| Slack          | Third-party TLS only | Slack servers    | No             | No           | Blocked       |

### 8.1 Web Dashboard

The web dashboard carries all seven security layers and is the
strongest channel. All operations, including sensitive commands, are
available through this interface.

### 8.2 Telegram and Slack

Telegram and Slack adapters rely on the respective platform's TLS for
transport encryption. They do not participate in the R2-TRUST trust
group. Messages are authenticated only by the platform's own bot token
and, for Telegram, the `allow` list of permitted chat IDs.

The following commands MUST be restricted to the web dashboard and MUST
be blocked from Telegram and Slack:

- `/analyse <file>` -- thematic analysis of workspace files.
- `/specify <file>` -- specification generation from workspace files.
- `/test-vectors <file>` -- test vector generation from workspace files.

These commands operate on files in the workspace and produce structured
output that requires the full trust group authentication context.
Implementations MUST reject these commands on messaging adapter channels
and SHOULD return an error message directing the user to the web
dashboard.

### 8.3 Telegram Allow List

Operators MUST configure the `allow` list in the Telegram adapter
configuration to restrict which chat IDs can interact with the ANT.
Without an allow list, anyone who discovers the bot username can send
commands.

---

## 9. R2 Architecture Security

The R2 architecture provides two structural security guarantees that
operate independently of authentication and encryption.

### 9.1 256-Byte Event Limit

All events on the R2 wire (R2-WIRE) are limited to 256 bytes. This
constraint is a security boundary: it prevents large payloads -- and
therefore prompt-injection vectors -- from traversing the event bus.

Events carry decisions (state transitions, routing commands). Content
of unlimited size (AI prompts, knowledge graph snapshots, user messages)
travels on the plugin data plane, never on the event bus.

Implementations MUST enforce the 256-byte limit on all events.
Implementations MUST NOT allow events larger than 256 bytes to be
serialised, transmitted, or processed.

### 9.2 Validated Writes

AI workers MUST NOT edit knowledge graph files directly. All mutations
to the knowledge graph MUST pass through the Thurisaz engine, which
validates structure, recalculates confidence scores, and enforces
schema constraints.

This provides structural isolation: even if an AI backend produces
malicious or malformed output, the Thurisaz engine rejects invalid
mutations before they reach the knowledge store. The guarantee is
enforced by architecture, not by prompting.

### 9.3 Separation of Concerns

The combination of event-limit enforcement and validated writes means
that:

1. The event bus cannot carry prompt-injection payloads (too large).
2. The knowledge store cannot be corrupted by AI output (validated).
3. These guarantees hold regardless of the AI backend's behaviour.

---

## 10. Conformance

### 10.1 REQUIRED

An implementation claiming conformance to ANTHILL-TRUST:

1. MUST generate and persist an Ed25519 colony key on first run
   (Section 2).
2. MUST implement the join code flow: generate, present, join, store
   (Section 3).
3. MUST enforce that join codes are single-use and expire after 300
   seconds (Section 3.1).
4. MUST authenticate all protected API routes via the `X-Credential`
   header (Section 4.2).
5. MUST authenticate WebSocket connections via credential query
   parameter before upgrading (Section 4.3).
6. MUST sign all WebSocket messages with HMAC-SHA256 envelopes
   (Section 5.1).
7. MUST reject WebSocket messages with timestamps older than 60 seconds
   (Section 5.4).
8. MUST use constant-time comparison for HMAC signature verification
   (Section 5.3).
9. MUST NOT return device credentials in list or query responses
   (Section 6.1).
10. MUST persist the device registry to survive server restarts
    (Section 6.4).
11. MUST block sensitive commands (`/analyse`, `/specify`,
    `/test-vectors`) from Telegram and Slack channels (Section 8.2).
12. MUST enforce the 256-byte event limit on the R2 wire (Section 9.1).
13. MUST route all knowledge graph mutations through the Thurisaz
    engine (Section 9.2).

### 10.2 RECOMMENDED

1. Operators SHOULD deploy behind Tailscale or equivalent WireGuard
   mesh VPN (Section 7.1).
2. Operators SHOULD enable HTTPS via `tailscale serve` or a reverse
   proxy (Section 7.2).
3. Operators SHOULD configure the Telegram `allow` list (Section 8.3).
4. Operators SHOULD review provisioned devices periodically and revoke
   unrecognised entries (Section 6.2).
5. Operators SHOULD use private repositories for git backup remotes.
6. Operators SHOULD enable `encrypt_backups` when using third-party
   git hosting (Section 7.4).

### 10.3 OPTIONAL

1. Implementations MAY support key export and import via CLI flags
   (Section 2.5).
2. Implementations MAY support QR code presentation of join codes
   (Section 3.2).
3. Implementations MAY support XChaCha20-Poly1305 backup encryption
   (Section 7.4).

---

## 11. References

- RFC 2119. Bradner, S. "Key words for use in RFCs to Indicate
  Requirement Levels." IETF, 1997.
- RFC 8032. Josefsson, S. and Liusvaara, I. "Edwards-Curve Digital
  Signature Algorithm (EdDSA)." IETF, 2017.
- R2-TRUST -- Reality2 trust group specification.
- R2-PROVISION -- Reality2 device provisioning UX flow.
- R2-WIRE -- Reality2 wire protocol and event framing.
- ANTHILL-COLONY -- Colony management and ANT lifecycle.
- ANTHILL-ONBOARDING -- First-run provisioning and setup.
- ANTHILL-WEB -- Web API endpoints and authentication.
