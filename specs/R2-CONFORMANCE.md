# Reality2 Specification Conformance Analysis

**Version:** 0.1.0
**Date:** 2026-03-23
**Anthill Version:** 0.9.0

---

## 1. Overview

Anthill is built on [Reality2 (R2)](https://reality2.ai), an event-driven architecture designed for IoT, wearables, and autonomous agents. This document assesses Anthill's conformance against the core R2 specifications: R2-CBOR, R2-FNV, R2-WIRE, and R2-TRUST.

Anthill is the first production R2 application outside hardware — proving the architecture works for server-side AI agents, not just sensors and microcontrollers.

---

## 2. Conformance Summary

| R2 Specification | Status | Notes |
|---|---|---|
| Sentant as pure FSM | **Full** | ConductorSentant is stateless, passes SM-1 through SM-6 conformance vectors |
| Plugins handle all I/O | **Full** | TelegramPlugin, SlackPlugin, AiPlugin isolate all external I/O |
| 256-byte event limit | **Enforced** | `MAX_ACTION_PAYLOAD = 256` in r2-engine; full text flows through data plane |
| Events carry decisions, plugins carry data | **Full** | Event payloads are small CBOR maps (~30 bytes); message text in shared queues |
| R2-TRUST | **Full** | Ed25519 certificates, HKDF key derivation, X25519 join protocol, XChaCha20-Poly1305 |
| R2-CBOR | **Partial** | Standard mode used correctly for events; knowledge graph uses ciborium (full CBOR) |
| R2-FNV | **Full** | Event hash routing uses FNV-1a with canonical lowercasing |
| R2-WIRE | **Not used** | Crate available and tested; not needed for single-instance deployment |

---

## 3. Detailed Assessment

### 3.1 Sentant Architecture — Full Conformance

The conductor sentant (`src/sentants/conductor.rs`) is a pure finite state machine:

- Single state (`STATE_READY = 0`)
- Receives events, emits actions via `ActionBuf` — no I/O, no channels, no shared state
- Handles 13+ command types via small CBOR payloads (< 256 bytes)
- `handle_event()` is synchronous and non-blocking
- Actions are declarative — the sentant says _what_ should happen, plugins decide _how_

The `Sentant` trait (`crates/r2-engine/src/sentant.rs`) enforces purity at the type level. Given the same events, the conductor always produces the same actions. This is the core R2 guarantee: deterministic decision-making with all side effects delegated to plugins.

Conformance tests pass against R2 engine test vectors SM-1 through SM-6.

### 3.2 Plugin Architecture — Full Conformance

All I/O is abstracted into plugins following the R2 pattern:

| Plugin | I/O Responsibility |
|---|---|
| **AiPlugin** | Claude Code subprocess management, task tracking, follow-up queues, file system access |
| **TelegramPlugin** | Telegram Bot API via teloxide, incoming/outgoing message routing |
| **SlackPlugin** | Slack Socket Mode WebSocket, message threading |

Each plugin implements the `Plugin` trait with `execute(command, data) -> PluginResult`. Plugins can define their own command bytes and manage their own state — the sentant never sees it.

The data plane separation is clean:
- **Event bus** — small hash + metadata (< 256 bytes)
- **Message queue** — full text/binary data via `Arc<Mutex<VecDeque>>`
- **Direct channels** — tokio `mpsc`/`broadcast` for I/O operations

### 3.3 256-Byte Event Limit — Enforced

The 256-byte limit is structurally enforced at two levels:

```
PayloadBuf:  inline [u8; 256] storage (action.rs)
QueuedEvent: inline [u8; 256] storage (queue.rs)
```

Full message text (user input, AI responses, files) flows through the data plane — shared queues that plugins read from directly. The conductor sentant only receives small CBOR maps like `{0: cmd_type, 1: chat_id}` (~30 bytes), never raw content.

This separation is what makes prompt injection via the event bus structurally impossible — the sentant never sees the content it's making decisions about.

**Note:** Payloads exceeding 256 bytes are silently truncated, not rejected. This matches R2's non-blocking philosophy but could mask bugs.

### 3.4 R2-TRUST — Full Conformance

The trust group implementation (`src/trust.rs`) wraps `r2_trust::TrustGroup` and uses the full R2-TRUST API:

- **Device certificates** — Ed25519-signed, with KeyHolder and Member roles
- **Key derivation** — HKDF-SHA256 splits shared secret into DEK (encryption) and HK (authentication)
- **Join protocol** — 128-bit join codes with 5-minute TTL, X25519 key exchange, XChaCha20-Poly1305 encrypted responses
- **Revocation** — RevocationSet tracks revoked devices with reasons
- **Persistence** — Colony key at `~/.config/anthill/colony.key`, devices at `devices.toml`, join codes at `join-codes.toml`

WebSocket authentication uses HMAC-SHA256 signed envelopes with timestamps for replay protection. REST API authentication uses the `X-Credential` header with hex-encoded Ed25519 seed.

### 3.5 R2-CBOR — Partial Conformance

R2-CBOR defines two encoding modes:
- **Compact** — integer keys only, max 180 bytes (BLE/LoRa)
- **Standard** — string keys allowed, max 65535 bytes (IP networks)

Anthill uses R2-CBOR correctly for **event payloads** — Standard mode with integer keys, well within size limits. The `r2_cbor` crate is used directly in the conductor and plugins for encoding/decoding event data.

However, **knowledge graph storage** uses the `ciborium` crate (full CBOR with string keys and serde integration) rather than constrained R2-CBOR. This is a justified deviation:
- Knowledge graphs need string keys for flexible schema
- R2-CBOR's integer-key constraint was designed for microcontroller events
- `ciborium` integrates cleanly with serde for complex nested structures
- The R2-CBOR spec explicitly allows Standard mode for IP transports

### 3.6 R2-FNV — Full Conformance

Event routing uses FNV-1a hashing for event name → 32-bit hash mapping. The `r2_fnv` crate provides canonical lowercasing and whitespace stripping as specified.

### 3.7 R2-WIRE — Not Used (By Design)

The `r2_wire` crate is available in `crates/r2-wire/` and passes all conformance tests (Compact and Extended frame formats, HMAC envelope support). However, it is not used for internal communication.

**Rationale:** Anthill is a single-instance deployment. R2-WIRE was designed for mesh networking between devices (BLE, LoRa, TCP). Internal sentant-to-plugin communication uses local dispatch through the event bus — no wire framing needed.

R2-WIRE would be integrated at the gateway layer if Anthill supported multi-instance fleet deployment or inter-colony communication.

---

## 4. Architectural Observations

### 4.1 Web Server as Parallel I/O Path

The web server (`src/web.rs`) sits outside the plugin architecture as a parallel I/O transport. It sends messages to the AI plugin via tokio channels rather than through the event bus.

Some web commands (`/reflect`, `/reprocess-graphs`, `/citations`) bypass the sentant entirely — they operate directly on the knowledge store or send requests to the AI worker. This is pragmatic for server-side operations but represents a second entry path into the system.

**Impact:** Low risk. These commands perform maintenance operations that don't need sentant decision-making. The trust group authentication still applies.

### 4.2 Data Plane Design

R2's "events carry decisions, plugins carry data" principle is well-implemented through three data planes:

1. **Event bus** — hash-routed, < 256 bytes, sentant-visible
2. **Shared queues** — full message text, plugin-to-plugin only
3. **Broadcast channels** — WebSocket events, status updates

This three-layer design emerged naturally from the constraint that AI chat messages (potentially thousands of characters) cannot fit in 256-byte event payloads. The solution — store text in a shared queue and pass only a reference through the bus — is idiomatic R2.

### 4.3 Single-Instance vs Mesh

The R2 architecture was designed for mesh networking across heterogeneous devices. Anthill uses a subset: single-instance, server-side, IP-only. This means:

- R2-WIRE framing is unnecessary (no multi-hop routing)
- Compact CBOR mode is unnecessary (no BLE/LoRa constraints)
- Trust group is colony-scoped (no cross-colony peering)

These are valid simplifications for the deployment context. The architecture could be extended to multi-instance if needed — the R2-WIRE and peering infrastructure is already available in the crate dependencies.

---

## 5. Conformance Gaps and Future Work

| Gap | Severity | Path to Resolution |
|---|---|---|
| Web commands bypass sentant | Low | Route web commands through event bus for audit trail |
| Silent payload truncation | Low | Add debug-mode warning when truncation occurs |
| No R2-WIRE usage | N/A | Add at gateway layer if multi-instance fleet is needed |
| ciborium for graph storage | N/A | Justified deviation — R2-CBOR unsuitable for graph schema |
| No Compact CBOR mode | N/A | Not applicable to server deployment |

---

## 6. Conclusion

Anthill is a **faithful R2 implementation** for server-side AI agents. The core architectural principles — pure FSM sentants, plugin-isolated I/O, 256-byte event limit, decision/data separation, trust group security — are all structurally enforced in the code, not just documented.

The deviations from the full R2 specification (no R2-WIRE, ciborium for storage, web bypass) are justified by the deployment context and explicitly allowed by the R2 specifications. The foundation is in place for future extensions (multi-instance, cross-colony peering) without architectural changes.
