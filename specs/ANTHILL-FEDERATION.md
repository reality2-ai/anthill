# ANTHILL-FEDERATION: Inter-Colony ANT Communication

**Version:** 0.1.0 (design)
**Date:** 2026-03-22
**Status:** Draft
**Depends on:** ANTHILL-COLONY, R2-WIRE, R2-TRUST, R2-ROUTE

---

## 1. Overview

ANTs in the same colony communicate via the file-based inbox/outbox mechanism
(intra-colony, same filesystem). ANTs in different colonies — on different
machines but in the same trust group — communicate via R2 wire protocol
over TCP/IP (inter-colony federation).

From an ANT's perspective, consulting a peer on another machine is identical
to consulting one locally: `talk_to_ant("remote-gaea", "what about X?")`.
The transport layer handles the routing transparently.

## 2. Architecture

```
Colony A (alfred)                    Colony B (other-machine)
┌─────────────────┐                 ┌─────────────────┐
│ Wilbur  Gaea    │                 │ Remote-ANT      │
│ EventBus × 2    │                 │ EventBus × 1    │
│                 │                 │                 │
│ ┌─────────────┐ │  R2-WIRE/TCP   │ ┌─────────────┐ │
│ │ Transport   │◄├─────────────────├►│ Transport   │ │
│ │ Plugin      │ │  (Tailscale)   │ │ Plugin      │ │
│ │ (Extended)  │ │                 │ │ (Extended)  │ │
│ └─────────────┘ │                 │ └─────────────┘ │
│                 │                 │                 │
│ Trust Group: same colony.key      │ Trust Group: same│
└─────────────────┘                 └─────────────────┘
```

## 3. R2 Protocol Usage

### 3.1 Wire Format

Inter-colony messages use **R2-WIRE Extended format** (22-byte header):
- `target`: trust_group_hash(4) + hive_hash(4)
- `event_hash`: FNV-1a hash of event name (e.g., `colony.query`)
- CBOR payload (Standard mode, string keys)
- HMAC-SHA256 tag (32 bytes) for authentication

### 3.2 Trust Authentication

Both colonies share the same trust group (colony.key). Messages are:
- **Signed** with the shared HMAC key (HK derived from colony key via HKDF)
- **Verified** on receipt — reject if HMAC fails
- **Not encrypted** for intra-group (same trust group = shared DEK)
  - Tailscale provides transport encryption anyway

### 3.3 Addressing

ANTs are addressed by hive + class:
- `hive`: FNV-1a hash of the colony's device ID (from colony.key)
- `class`: `ai.reality2.ant.<name>` (e.g., `ai.reality2.ant.gaea`)

Event types:
- `colony.query` — ask an ANT a question
- `colony.response` — answer from an ANT

### 3.4 Transport

TCP over Tailscale. Each Anthill listens on a configurable port.
Connection management:
- Persistent TCP connections between known peers
- Reconnect on failure with exponential backoff
- Heartbeat every 60 seconds to maintain route confidence

## 4. Components to Build

### 4.1 New R2 Crate: r2-transport (or extend r2-wire)

TCP transport layer that frames r2-wire messages over TCP streams:
- Length-prefixed framing: `[u32 big-endian length][r2-wire bytes]`
- TLS optional (Tailscale already encrypts)
- Connection pool management
- Async (tokio) for Anthill's runtime

### 4.2 New Anthill Plugin: FederationBridge

R2 plugin that bridges between:
- Local EventBus events → outgoing r2-wire messages
- Incoming r2-wire messages → local EventBus events

Replaces the file-based inbox/outbox for remote ANTs.
Local ANTs continue to use the file mechanism.

### 4.3 Configuration

In `supervisor.toml`:
```toml
[federation]
enabled = true
listen_port = 3001
peers = ["alfred.tailnet:3001", "other-machine.tailnet:3001"]
```

### 4.4 Discovery

Phase 1: Explicit peer list in config (static).
Phase 2: Automatic discovery via trust group device registry.
Phase 3: R2-BEACON over Tailscale multicast (future).

## 5. Message Flow

### 5.1 Outbound (Wilbur on alfred asks Remote-ANT on other-machine)

1. Wilbur's AI calls `talk_to_ant("Remote-ANT", "question")`
2. MCP tool writes to colony_outbox
3. Worker polls outbox, sees target is remote (not in local ants/)
4. FederationBridge encodes as R2-WIRE Extended message:
   - event_hash = fnv1a("colony.query")
   - target = trust_group_hash + remote_hive_hash
   - CBOR payload with from, message, chat_id
   - HMAC-SHA256 signed with trust group HK
5. Sends over TCP to other-machine:3001
6. Worker continues with other work (fire-and-forget)

### 5.2 Inbound (Remote-ANT responds)

1. FederationBridge on alfred receives TCP message
2. Verifies HMAC with shared HK — reject if invalid
3. Decodes R2-WIRE Extended message
4. Extracts CBOR payload (from, message, chat_id)
5. Writes to local colony_inbox for the target ANT
6. Worker polls inbox (5-second interval), processes as colony request
7. Response forwarded back to Wilbur's chat

## 6. Implementation Order

1. **TCP framing** — length-prefixed r2-wire over TCP (new module in r2-wire or separate crate)
2. **FederationBridge plugin** — listen + connect, encode/decode, HMAC sign/verify
3. **Configuration** — supervisor.toml federation section
4. **Outbox routing** — detect remote targets, route through FederationBridge
5. **Testing** — two Anthills on Tailscale, inter-colony /ask

## 7. Security

- Same trust group = same HMAC key. Messages authenticated but not encrypted by R2.
- Tailscale provides transport-level encryption (WireGuard).
- Rate limiting on inbound messages (prevent flooding).
- Message size limit (64KB, matching R2-WIRE Extended max).

## 8. Relationship to R2 Routing

This design uses **direct TCP connections** (point-to-point), not the full
R2 mesh routing (spray-and-wait). The mesh routing is designed for BLE/LoRa
constrained environments. TCP over Tailscale provides reliable delivery.

Future: if Anthill deploys on constrained devices or needs multi-hop routing,
the full R2-ROUTE spray-and-wait can be layered on top of the same r2-wire
framing.
