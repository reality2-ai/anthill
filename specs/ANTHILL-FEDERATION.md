# ANTHILL-FEDERATION: Distributed Deployment and Relay Protocol

| Field      | Value                                                        |
|------------|--------------------------------------------------------------|
| Version    | 0.1 Draft                                                    |
| Date       | 2026-03-30                                                   |
| Status     | Draft                                                        |
| Depends on | ANTHILL-COLONY, ANTHILL-TRUST, R2-INTERNET, R2-TRANSPORT     |
| Related    | ANTHILL-COMMS, ANTHILL-DASHBOARD                             |

> The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT",
> "SHOULD", "SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this
> document are to be interpreted as described in RFC 2119.

---

## 1. Introduction

Anthill federation enables a single colony to span multiple machines.  The
web dashboard, reasoning engines, and browser clients are all devices in the
same R2-TRUST trust group, connected by a WebSocket relay protocol that
carries Anthill operations as JSON messages.

Federation separates the concerns of *serving the UI* from *running AI
inference*.  A cloud VM can host the web dashboard on a public address while
one or more reasoning engines run on private machines -- behind NAT, on a
GPU workstation, or co-located with sensitive data that must not leave the
premises.  Browsers connect to the web server; the web server relays
requests to reasoning engines; reasoning engines relay responses back.  The
browser sees no difference between a local ANT and a remote one.

### 1.1 Scope

This specification defines:

- Deployment topologies supported by federation.
- The component topology within a single colony trust group.
- The relay WebSocket protocol between web server and reasoning engine(s).
- The mapping of Anthill operations to R2 event names.
- The Web Gateway sentant that routes requests on the web server.
- The Reasoning Node relay listener that exposes local ANTs to the colony.
- Configuration schema for enabling and tuning federation.
- Security considerations for distributed deployment.

### 1.2 Relationship to R2-INTERNET

R2-INTERNET (R2-INTERNET section 4) specifies a general-purpose relay for forwarding
opaque R2-WIRE frames between hives.  The Anthill relay protocol defined
here operates at a higher layer: it carries application-level JSON messages
(chat requests, AI progress, status updates) over authenticated WebSocket
connections between components that are all members of the same colony trust
group.

A future version of this specification MAY migrate the relay protocol to
native R2-WIRE frames carried by an R2-INTERNET relay, eliminating the need
for Anthill-specific relay code.

### 1.3 Relationship to R2-TRANSPORT

R2-TRANSPORT defines the Internet transport binding (ID 3, bitmask 0x08)
and the WebSocket framing used by R2-INTERNET.  Anthill federation uses
WebSocket as its transport but currently carries its own JSON envelope
rather than R2-WIRE extended frames.  The authentication model (device
credential HMAC at connection time) is consistent with R2-TRANSPORT section 7.2.

### 1.4 Implementation Status

> **Note:** This specification is forward-looking.  The relay protocol
> (section 4), Web Gateway (section 6), Reasoning Node (section 7), and configuration
> schema (section 8) are **implemented** in the Rust codebase (`src/relay.rs`,
> `src/web.rs`, `src/supervisor.rs`).  The R2 event mapping (section 5) is
> **design only** -- the current implementation uses direct JSON relay
> messages rather than R2-WIRE events.  Sections marked *[NOT YET
> IMPLEMENTED]* describe planned behaviour.

### 1.5 Terminology

| Term              | Definition                                                                                          |
|-------------------|-----------------------------------------------------------------------------------------------------|
| Colony            | A group of ANTs sharing a single trust group identity (`colony.key`).                                |
| Relay             | The WebSocket connection between a web server and a reasoning engine within the same colony.         |
| Web Gateway       | The component on the web server that routes browser requests to reasoning engines.                    |
| Reasoning Node    | A machine running the ANT supervisor, AI workers, and a relay listener.                              |
| Proxy BotHandle   | A local registry entry on the web server that forwards requests to a remote reasoning engine.        |
| Device Credential | An Ed25519 key pair issued to a colony member at provisioning time (R2-TRUST).                       |

---

## 2. Deployment Topologies

Anthill supports three deployment topologies.  All three use the same relay
protocol; they differ only in how many machines are involved and where each
component runs.

### 2.1 Single Machine (Default)

All components run on one machine.  The web server, supervisor, ANTs, AI
workers, and knowledge graphs share a single process.

```
localhost:
  Web server (port 3000)
  Supervisor with ANTs
  AI workers
  Knowledge graphs
```

No relay configuration is needed.  This is the current default and MUST
remain backward-compatible.

### 2.2 Cloud Web + Local Reasoning

The web dashboard runs on a cloud VM with a public IP address.  One or more
reasoning engines run on private machines connected via Tailscale VPN or
direct TLS.

```
cloud-vm (public IP, HTTPS):
  Web Gateway (port 443)
  Colony trust group member (device)

home-server (behind NAT, Tailscale):
  Reasoning Node with ANTs
  AI workers (Claude CLI, Ollama, API backends)
  Knowledge graphs
  Relay listener (port 3001)

Transport: Tailscale VPN (RECOMMENDED), direct IP + TLS, or localhost
```

The web server connects outbound to the reasoning engine's relay listener.
If both machines are on the same Tailscale network, no firewall
configuration is required.

### 2.3 Multi-Reasoning-Node

One web server connects to multiple reasoning engines.  Each engine hosts a
subset of the colony's ANTs.  The web server presents all ANTs in a unified
dashboard.

```
cloud-vm:
  Web Gateway — relay connects to both engines

reasoning-node-1 (GPU server):
  ANTs for coding tasks
  Ollama with large models
  Relay listener (port 3001)

reasoning-node-2 (CPU server):
  ANTs for general tasks
  Claude CLI, API backends
  Relay listener (port 3001)

All three are devices in the same colony trust group.
```

The Web Gateway MUST maintain connections to all configured reasoning
engines concurrently.  If a reasoning engine disconnects, the Web Gateway
MUST remove its proxy BotHandles from the registry and MUST attempt
reconnection with exponential backoff (section 4.6).

---

## 3. Component Topology

All components in a federated Anthill deployment are **devices in the same
colony trust group**.  This is intra-group communication as defined by
R2-TRUST sections 3 through 6.  It is **not** entanglement (R2-TRUST section 7), which is
reserved for bilateral peering between different trust groups.

```
+-- Colony Trust Group (colony.key) -----------------------------------+
|                                                                       |
|  +-- Web Server (device: provisioned) ---------------------------+   |
|  |  Serves dashboard, handles browser WebSocket connections      |   |
|  |  Relay WebSocket client -- connects to reasoning engines      |   |
|  |  Routes requests transparently (local or remote ANTs)         |   |
|  +-------------------------------+-------------------------------+   |
|                                  | relay WebSocket                    |
|                                  | (device credential HMAC auth)      |
|  +-- Reasoning Engine 1 --------+-------------------------------+   |
|  |  (device: provisioned)                                        |   |
|  |  Runs ANT supervisor + AI workers                             |   |
|  |  Knowledge graph storage and rumination                       |   |
|  |  Relay WebSocket listener (port 3001)                         |   |
|  +---------------------------------------------------------------+   |
|                                                                       |
|  +-- Reasoning Engine 2 (OPTIONAL) -----------------------------+   |
|  |  (device: provisioned)                                        |   |
|  |  Additional ANTs, different backends, GPU resources           |   |
|  +---------------------------------------------------------------+   |
|                                                                       |
|  +-- Browser (device: provisioned via join code) ----------------+   |
|  |  WebSocket to web server, device credential HMAC auth         |   |
|  +---------------------------------------------------------------+   |
|                                                                       |
+-----------------------------------------------------------------------+
```

### 3.1 Trust Model

All devices in the colony share a single trust group identity derived from
one `colony.key` (Ed25519 signing key, generated on first run).  Each
device -- web server, reasoning engine, browser -- MUST be provisioned via
a join code and MUST receive an Ed25519 device credential at provisioning
time.

Authentication for relay connections uses the same device-credential HMAC
mechanism as browser WebSocket connections (ANTHILL-TRUST).  The device
presents its credential at WebSocket upgrade time; the colony key holder
verifies membership.

### 3.2 Transport Options

All transport options carry the same device-credential authentication:

| Transport           | Description                                                  | Encryption       |
|---------------------|--------------------------------------------------------------|------------------|
| Tailscale VPN       | All devices on the same Tailscale network. RECOMMENDED.      | WireGuard        |
| Direct IP + TLS     | Reasoning engine on public internet with `wss://`.           | TLS 1.3          |
| Localhost           | Both components on same machine. `ws://127.0.0.1`.           | None required    |

Implementations SHOULD use Tailscale for deployments where the reasoning
engine is behind NAT.  Tailscale provides WireGuard encryption, automatic
NAT traversal, and zero firewall configuration.

For internet-facing deployments without a VPN, implementations MUST use TLS
(`wss://`).  The R2-TRUST HMAC layer provides authentication and integrity
even on encrypted transports, preventing man-in-the-middle attacks at the
application layer.

---

## 4. Relay Protocol

The relay protocol carries Anthill operations over a WebSocket connection
between the web server and a reasoning engine.

### 4.1 Connection Establishment

The web server initiates the WebSocket connection to the reasoning engine's
relay endpoint.  The reasoning engine MUST listen for relay connections on a
configurable port (default: 3001) at the path `/relay`.

```
GET /relay?credential=<hex>&device_id=<hex> HTTP/1.1
Upgrade: websocket
Connection: Upgrade
Sec-WebSocket-Version: 13
```

The reasoning engine MUST authenticate the connecting device as a member of
the colony trust group by verifying the device credential.  If
authentication fails, the reasoning engine SHOULD log the attempt and MAY
still accept the connection for same-machine deployments where transport
security is provided by the OS.

> **[NOT YET IMPLEMENTED]:** In production, the reasoning engine MUST
> reject unauthenticated connections with WebSocket close code 4401
> (Unauthorized).  The current implementation accepts unauthenticated
> connections to simplify same-machine deployment.

### 4.2 Message Envelope

Every WebSocket text message is a JSON envelope:

```json
{
  "device_id": "<hex public key>",
  "timestamp": 1711756800,
  "signature": "<HMAC-SHA256 hex>",
  "payload": "<inner JSON string>"
}
```

The `payload` field contains a serialized `RelayMessage` (section 4.3).

Authentication for relay connections happens once at WebSocket upgrade time.
After successful authentication, the session is trusted.  Per-message HMAC
verification within the envelope is OPTIONAL for intra-trust-group relay
connections.  The envelope wrapper is retained for consistency with the
browser protocol and to carry `device_id` and `timestamp` for logging.

> **[NOT YET IMPLEMENTED]:** Per-message HMAC verification SHOULD be
> enabled for relay connections traversing untrusted networks (direct
> IP + TLS without VPN).

### 4.3 Message Types

The relay protocol defines the following message types, discriminated by the
`relay_type` field in the JSON payload:

#### 4.3.1 Web Server to Reasoning Engine

| Type       | Fields                                           | Description                                |
|------------|--------------------------------------------------|--------------------------------------------|
| `Chat`     | `bot`, `chat_id`, `message`, `task_id`, `source` | Route a user message to a specific ANT.    |
| `Cancel`   | `bot`, `task_id`                                 | Cancel a running task on a specific ANT.   |
| `FollowUp` | `bot`, `task_id`, `message`, `chat_id`, `source` | Queue a follow-up for a running task.      |
| `ListAnts` | (none)                                           | Request the list of ANTs on this engine.   |

#### 4.3.2 Reasoning Engine to Web Server

| Type        | Fields                | Description                                          |
|-------------|-----------------------|------------------------------------------------------|
| `Event`     | `event_json`          | A serialized `WsEvent` (progress, message, status).  |
| `AntList`   | `ants`                | Declares which ANTs this engine hosts.                |

#### 4.3.3 Bidirectional

| Type        | Fields                  | Description                           |
|-------------|-------------------------|---------------------------------------|
| `Heartbeat` | `engine_id`, `timestamp` | Periodic liveness signal.             |

### 4.4 Initial Handshake

After WebSocket upgrade, the web server MUST send a `ListAnts` message.
The reasoning engine MUST respond with an `AntList` message containing
metadata for each locally hosted ANT:

```json
{
  "relay_type": "AntList",
  "ants": [
    {
      "name": "alfred",
      "display_name": "Alfred",
      "status": "running"
    }
  ]
}
```

On receiving the `AntList`, the web server MUST register a proxy BotHandle
for each remote ANT in its local BotRegistry.  Proxy BotHandles forward
`CliRequest` messages through the relay as `Chat` relay messages.  The
display name SHOULD be suffixed with `" (remote)"` to distinguish remote
ANTs in the dashboard.

### 4.5 Heartbeat

Both sides MUST send a `Heartbeat` message every 30 seconds.  If no
message of any kind is received from the peer within 90 seconds (3x the
heartbeat interval), the connection MUST be considered stale and SHOULD be
closed.

The 30-second interval is consistent with R2-TRANSPORT section 6.1 and R2-INTERNET
section 2.6.

### 4.6 Reconnection

On connection loss, the web server MUST attempt reconnection with
exponential backoff:

```
delays: [1s, 2s, 4s, 8s, 16s, 30s, 30s, 30s, ...]
```

The backoff cap is 30 seconds.  Reconnection attempts MUST continue
indefinitely -- the reasoning engine may restart or become reachable again
at any time.

On disconnection, the web server MUST remove all proxy BotHandles
associated with the disconnected reasoning engine from the BotRegistry.
On successful reconnection, the web server MUST re-request the ant list and
re-register proxy BotHandles.

### 4.7 Event Forwarding

The reasoning engine MUST subscribe to its local `global_tx` broadcast
channel and forward every `WsEvent` to connected web servers as a
`RelayMessage::Event`.  The web server MUST deserialize the inner
`WsEvent` and re-broadcast it on its own `global_tx`, making remote ANT
events available to browser WebSocket clients.

This forwarding is transparent: a browser client receives `WsEvent` messages
in the same format regardless of whether the originating ANT is local or
remote.

---

## 5. R2 Event Mapping

> **[NOT YET IMPLEMENTED]:** This section defines how Anthill operations
> map to R2 event names for use with native R2-WIRE event routing.  The
> current implementation uses the JSON relay protocol (section 4) rather than
> R2-WIRE frames.  When Anthill migrates to R2-WIRE events, these event
> names SHALL be used.

Anthill defines the following R2 event names.  Each is hashed with FNV-1a
32-bit for use in R2-WIRE compact and extended headers.

| Event Name            | FNV-1a Hash       | Direction                  | Description                                           |
|-----------------------|-------------------|----------------------------|-------------------------------------------------------|
| `anthill.request`     | `fnv1a_32(...)` | Web Server -> Reasoning Node | User message routed to reasoning engine for processing. |
| `anthill.response`    | `fnv1a_32(...)` | Reasoning Node -> Web Server | AI response routed back to the web server for delivery. |
| `anthill.progress`    | `fnv1a_32(...)` | Reasoning Node -> Web Server | Task progress (thinking, tool use, reading, writing) forwarded in real time. |
| `anthill.status`      | `fnv1a_32(...)` | Reasoning Node -> Web Server | ANT status changes (idle, running, error, suspended).   |
| `anthill.cancel`      | `fnv1a_32(...)` | Web Server -> Reasoning Node | Cancel request forwarded to the reasoning engine.        |

### 5.1 Event Payloads

Event payloads SHALL be CBOR-encoded (R2-WIRE data plane) and MUST NOT
exceed the R2-WIRE 256-byte event envelope limit.  Large content (message
text, AI responses) travels on the plugin data plane, not in the event
payload.

#### EVENT_ANTHILL_REQUEST

```
Fields:
  ant_id    : text       — target ANT name
  task_id   : uint32     — unique task identifier
  chat_id   : int64      — originating conversation
  source    : text       — "web", "telegram", "slack"
  category  : text / nil — engine category hint ("intellectual", "fast", etc.)
```

The full message text MUST be delivered via the plugin data plane (shared
queue or relay WebSocket), not in this event.

#### EVENT_ANTHILL_RESPONSE

```
Fields:
  ant_id       : text       — originating ANT name
  task_id      : uint32     — matching task identifier
  chat_id      : int64      — destination conversation
  backend_used : text       — which AI backend produced the response
  cost_usd     : float / nil — estimated cost in USD
```

The response text MUST be delivered via the plugin data plane.

#### EVENT_ANTHILL_PROGRESS

```
Fields:
  ant_id        : text   — originating ANT name
  task_id       : uint32 — matching task identifier
  progress_type : text   — "thinking", "tool_use", "reading", "writing", "running"
  detail        : text   — human-readable description (truncated to fit envelope)
```

#### EVENT_ANTHILL_STATUS

```
Fields:
  ant_id : text — ANT name
  status : text — "idle", "running", "error", "suspended", "configured"
```

#### EVENT_ANTHILL_CANCEL

```
Fields:
  ant_id  : text   — target ANT name
  task_id : uint32 — task to cancel
```

---

## 6. Web Gateway Sentant

> **[PARTIALLY IMPLEMENTED]:** The Web Gateway routing logic is implemented
> in `src/relay.rs` as the `WebGateway` struct.  It is not yet modelled as
> a formal R2 sentant with FSM transitions.  This section describes the
> target architecture.

The Web Gateway is a sentant running on the web server that routes browser
requests to the appropriate reasoning engine.

### 6.1 States

```
idle --> routing --> awaiting_response --> idle
```

| State              | Description                                                          |
|--------------------|----------------------------------------------------------------------|
| `idle`             | No active request being routed. Accepting new requests.              |
| `routing`          | Looking up the target ANT and forwarding the request.                |
| `awaiting_response`| Request forwarded; waiting for response or progress events.          |

The gateway MAY handle multiple concurrent requests.  Each request
transitions independently through the state machine.

### 6.2 Responsibilities

1. **Request routing.** On receiving a browser request for a specific ANT,
   the gateway MUST determine whether the ANT is local or remote and route
   accordingly:
   - Local ANT: send `CliRequest` via the in-process `mpsc` channel.
   - Remote ANT: send `RelayMessage::Chat` over the relay WebSocket.

2. **ANT-to-node mapping.** The gateway MUST maintain a mapping of ANT
   names to reasoning nodes, built from `AntList` responses received during
   relay handshake (section 4.4).

3. **Progress forwarding.** The gateway MUST forward `WsEvent` messages
   from reasoning engines to browser WebSocket clients in real time.

4. **Disconnection handling.** When a reasoning engine disconnects, the
   gateway MUST remove the corresponding proxy BotHandles and MUST report
   affected ANTs as unreachable in the dashboard.

### 6.3 Event Subscriptions

| Event                   | Action                                                    |
|-------------------------|-----------------------------------------------------------|
| `anthill.response`      | Deliver response to originating browser client.            |
| `anthill.progress`      | Forward progress to subscribed browser clients.            |
| `anthill.status`        | Update ANT status in dashboard.                            |

### 6.4 Event Emissions

| Event                   | Trigger                                                   |
|-------------------------|-----------------------------------------------------------|
| `anthill.request`       | Browser sends a chat message to an ANT.                    |
| `anthill.cancel`        | Browser requests task cancellation.                        |

---

## 7. Reasoning Node

A reasoning node runs the full ANT supervisor (ANTHILL-COLONY) with an
additional relay listener that exposes its local ANTs to the colony.

### 7.1 Relay Listener

The reasoning node MUST listen for WebSocket connections on the configured
relay port (default: 3001) at the path `/relay`.  The listener is
implemented as a lightweight Axum HTTP server separate from the main
dashboard web server.

### 7.2 Authentication

The relay listener MUST authenticate connecting devices as members of the
colony trust group.  Authentication uses the device credential presented as
query parameters at WebSocket upgrade time:

```
GET /relay?credential=<hex>&device_id=<hex> HTTP/1.1
```

The listener MUST verify the credential against the colony key.  On
authentication failure, the listener MUST reject the connection.

> **[NOT YET IMPLEMENTED]:** The current implementation accepts
> unauthenticated connections for ease of same-machine deployment.
> Production deployments MUST enforce authentication.

### 7.3 ANT Exposure

On receiving a `ListAnts` message, the reasoning node MUST respond with an
`AntList` containing metadata for every ANT registered in its local
BotRegistry.  The response MUST include the ANT's name, display name, and
current status.

### 7.4 Request Processing

On receiving a `Chat` message, the reasoning node MUST:

1. Look up the target ANT by name in the local BotRegistry.
2. If found, send a `CliRequest` to the ANT's `request_tx` channel.
3. If not found, log a warning.  The reasoning node SHOULD NOT send an
   error response -- the web server will detect the missing ANT through
   status monitoring.

On receiving a `Cancel` message, the reasoning node MUST abort the
identified task on the target ANT.

On receiving a `FollowUp` message, the reasoning node MUST queue the
follow-up in the target ANT's follow-up queue for delivery when the
current task completes.

### 7.5 Event Broadcasting

The reasoning node MUST subscribe to its local `global_tx` broadcast
channel and forward all `WsEvent` messages to every connected web server
as `RelayMessage::Event` messages.  This ensures that AI progress, task
completion, and status changes are visible in the dashboard in real time.

---

## 8. Configuration

Federation is configured via the `[relay]` section of `supervisor.toml`.

### 8.1 Reasoning Engine Configuration

To enable the relay listener on a reasoning engine:

```toml
[relay]
engine_listener = true
engine_port = 3001
```

| Field              | Type    | Default | Description                                        |
|--------------------|---------|---------|----------------------------------------------------|
| `engine_listener`  | bool    | `false` | Enable the relay WebSocket listener.                |
| `engine_port`      | uint16  | `3001`  | Port for the relay listener.                        |

### 8.2 Web Server Configuration

To connect the web server to one or more remote reasoning engines:

```toml
[relay]
remote_engines = [
    "ws://reasoning-host-1:3001/relay",
    "ws://reasoning-host-2:3001/relay",
]
credential = "<device credential hex>"
device_id = "<device public key hex>"
```

| Field              | Type       | Default | Description                                        |
|--------------------|------------|---------|----------------------------------------------------|
| `remote_engines`   | string[]   | `[]`    | WebSocket URLs of remote reasoning engines.         |
| `credential`       | string     | `""`    | Device credential for relay authentication.         |
| `device_id`        | string     | `""`    | Device public key for relay authentication.         |

If `credential` is empty, the web server SHOULD use the colony key holder
credential (for same-machine deployment).

### 8.3 ANT Placement

> **[NOT YET IMPLEMENTED]:** ANT placement is currently implicit -- each
> reasoning node runs whichever ANTs are configured in its local `ants/`
> directory.  This section describes planned explicit placement.

Each ANT's `ant.toml` MAY include a `node` field specifying which
reasoning node SHOULD host it:

```toml
name = "Code Assistant"
node = "gpu-server"

[ai]
default_category = "intellectual"
```

The supervisor SHOULD use the `node` field to determine where to spawn each
ANT.  If the specified node is unavailable, the supervisor MAY fall back to
local execution or MAY leave the ANT in a `configured` (not running) state.

### 8.4 Provisioning Workflow

To set up a distributed deployment:

1. **Create the colony** on the reasoning engine (generates `colony.key` on
   first run):
   ```bash
   anthill --supervise
   ```

2. **Provision the web server as a device**:
   ```bash
   # On reasoning engine:
   anthill --generate-join-code
   # Output: a1b2-c3d4-e5f6  (48-bit single-use, valid 5 minutes)

   # On web server:
   anthill --join a1b2-c3d4-e5f6
   # Provisions device, saves credential
   ```

3. **Configure relay** on both machines (section 8.1, section 8.2).

4. **Start both**:
   ```bash
   # Reasoning engine (has ANTs):
   anthill --supervise

   # Web server (no local ANTs, dashboard only):
   anthill --supervise
   ```

The web server connects to the reasoning engine via relay, discovers its
ANTs through `ListAnts`/`AntList` exchange, and registers proxy handles.
Browsers connecting to the web server see all ANTs in one unified dashboard.

---

## 9. Security Considerations

### 9.1 Single Colony Trust Group

All components MUST belong to the same colony trust group.  There is one
`colony.key` -- the Ed25519 root of trust.  Devices provision via join
codes with a 5-minute expiry (ANTHILL-TRUST).  Revoking a device credential
MUST immediately prevent that device from establishing new relay
connections.

### 9.2 Device Authentication

Every relay connection MUST be authenticated with a device credential at
WebSocket upgrade time.  The credential MUST be verified against the colony
key before the session is trusted.

### 9.3 Replay Protection

The message envelope includes a `timestamp` field.  Implementations SHOULD
reject envelopes with timestamps more than 60 seconds in the past.  This
prevents replay attacks where a captured envelope is re-sent after the
original connection has closed.

> **[NOT YET IMPLEMENTED]:** The current implementation does not enforce
> timestamp-based replay protection on relay envelopes.  This MUST be
> added before internet-facing deployment.

### 9.4 API Key Isolation

AI backend API keys (OpenAI, Anthropic, etc.) MUST remain on the reasoning
node.  They MUST NOT be transmitted over relay connections, stored on the
web server, or exposed to browser clients.  Each ANT's `ant.toml` on its
reasoning node holds backend-specific configuration including API key
environment variable names.

### 9.5 Transport Encryption

| Deployment                | Required Transport Security                               |
|---------------------------|-----------------------------------------------------------|
| Same machine (localhost)  | None required; OS process isolation is sufficient.         |
| Tailscale VPN             | WireGuard encryption provided by Tailscale.                |
| Direct internet           | TLS (`wss://`) REQUIRED.                                   |

For internet-facing reasoning engines, the relay listener MUST be served
over TLS.  Self-signed certificates MAY be used when the web server is
configured to trust the reasoning engine's certificate.

### 9.6 Event Bus Security

The 256-byte R2-WIRE event envelope limit (R2-WIRE section 3) prevents
prompt injection via the event bus.  In the relay protocol, message content
(user messages, AI responses) travels in the `payload` field of the JSON
envelope, not in R2-WIRE events.  This separation MUST be maintained: relay
messages carry content on the data plane; R2 events carry only identifiers
and control signals.

---

## 10. Conformance

### 10.1 REQUIRED

An implementation claiming conformance to ANTHILL-FEDERATION MUST:

1. Support all three deployment topologies (section 2).
2. Implement the relay WebSocket protocol (section 4) including all message
   types defined in section 4.3.
3. Authenticate relay connections using device credentials (section 4.1).
4. Send heartbeat messages every 30 seconds (section 4.5).
5. Implement exponential backoff reconnection on connection loss (section 4.6).
6. Forward `WsEvent` messages from reasoning engines to browser clients
   transparently (section 4.7).
7. Remove proxy BotHandles on reasoning engine disconnection (section 4.6).
8. Keep API keys isolated on reasoning nodes (section 9.4).
9. Use TLS for internet-facing relay connections (section 9.5).

### 10.2 RECOMMENDED

An implementation SHOULD:

1. Use Tailscale for deployments where reasoning engines are behind NAT.
2. Enforce timestamp-based replay protection on relay envelopes (section 9.3).
3. Suffix remote ANT display names with `" (remote)"` in the dashboard.
4. Log relay connection and disconnection events.
5. Support per-message HMAC verification for relay connections traversing
   untrusted networks.

### 10.3 OPTIONAL

An implementation MAY:

1. Implement the R2 event mapping (section 5) for native R2-WIRE event routing.
2. Support explicit ANT placement via the `node` field in `ant.toml` (section 8.3).
3. Implement knowledge graph synchronisation across reasoning nodes (future).
4. Support cross-colony federation via R2-TRUST entanglement (future).

---

## 11. Conjectures

| ID  | Conjecture | Falsification |
|-----|-----------|---------------|
| FED-001 | Relay forwarding adds < 100ms round-trip latency for AI progress events between reasoning engine and browser on broadband. | Measure end-to-end latency from `WsEvent` emission on reasoning node to browser receipt. If median > 100ms, profile the relay path. |
| FED-002 | A single web server can relay events from 10 concurrent reasoning engines without perceptible dashboard lag. | Connect 10 reasoning engines, each generating progress events at peak rate. Measure dashboard frame rate and event delivery latency. |
| FED-003 | Exponential backoff reconnection restores relay connectivity within 60 seconds of reasoning engine restart. | Restart reasoning engine, measure time from process ready to first successful relay handshake. |
| FED-004 | JSON envelope overhead (vs. binary R2-WIRE frames) is acceptable for the expected event rate (< 100 events/second per reasoning engine). | Benchmark JSON serialisation/deserialisation at 100 events/second. If CPU exceeds 5% on target hardware, consider migration to R2-WIRE binary. |
| FED-005 | Browser clients cannot distinguish local ANTs from remote ANTs in terms of response latency or event completeness. | Compare task completion time and progress event count for the same ANT running locally vs. remotely. |

---

## 12. References

- R2-INTERNET. Reality2 Internet Transport Specification, v0.1. 2026.
- R2-TRANSPORT. Reality2 Transport Binding Specification, v0.1. 2026.
- R2-TRUST. Reality2 Trust Group Specification.
- R2-WIRE. Reality2 Wire Protocol Specification.
- ANTHILL-COLONY. Colony Supervisor and ANT Lifecycle, v0.1. 2026.
- ANTHILL-TRUST. Trust and Security, v0.1. 2026.
- RFC 2119. Bradner, S. "Key words for use in RFCs to Indicate Requirement
  Levels." IETF, 1997.
- Tailscale. https://tailscale.com/ — WireGuard-based mesh VPN.

---

## Appendix A: Relay Message Schema (JSON)

```json
// Chat (web server -> reasoning engine)
{
  "relay_type": "Chat",
  "bot": "alfred",
  "chat_id": 12345,
  "message": "explain this code",
  "task_id": 42,
  "source": "web"
}

// Cancel (web server -> reasoning engine)
{
  "relay_type": "Cancel",
  "bot": "alfred",
  "task_id": 42
}

// FollowUp (web server -> reasoning engine)
{
  "relay_type": "FollowUp",
  "bot": "alfred",
  "task_id": 42,
  "message": "also check the tests",
  "chat_id": 12345,
  "source": "web"
}

// ListAnts (web server -> reasoning engine)
{
  "relay_type": "ListAnts"
}

// AntList (reasoning engine -> web server)
{
  "relay_type": "AntList",
  "ants": [
    { "name": "alfred", "display_name": "Alfred", "status": "running" },
    { "name": "coder", "display_name": "Code Assistant", "status": "idle" }
  ]
}

// Event (reasoning engine -> web server)
{
  "relay_type": "Event",
  "event_json": "{\"TaskProgress\":{\"bot\":\"alfred\",\"task_id\":42,\"kind\":\"tool_use\",\"detail\":\"Reading file\"}}"
}

// Heartbeat (bidirectional)
{
  "relay_type": "Heartbeat",
  "engine_id": "reasoning-node-1",
  "timestamp": 1711756800
}
```

---

## Appendix B: Revision History

| Version | Date       | Changes                                                                     |
|---------|------------|-----------------------------------------------------------------------------|
| 0.1     | 2026-03-30 | Initial draft -- relay protocol, deployment topologies, component topology, R2 event mapping (design), configuration schema, security considerations |
