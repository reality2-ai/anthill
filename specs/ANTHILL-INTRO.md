# ANTHILL-INTRO: Introduction, Vision, and Reading Guide

| Field      | Value                                                        |
|------------|--------------------------------------------------------------|
| Version    | 0.1 Draft                                                    |
| Date       | 2026-03-30                                                   |
| Status     | Draft                                                        |
| Depends on | R2-WIRE, R2-SENTANT, R2-TRUST                                |
| Related    | All ANTHILL-* specifications                                 |

> The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT",
> "SHOULD", "SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this
> document are to be interpreted as described in RFC 2119.

---

## 1. Introduction

Anthill is an AI reasoning engine built on Reality2 (R2). It runs autonomous
AI agents called ANTs (Autonomous iNTelligenceS) that maintain persistent
knowledge through a Popperian epistemology -- ideas are conjectures that earn
confidence through surviving genuine refutation, not through confirmation.

Anthill is the first production R2 application above the hardware layer,
proving the sentant architecture works for AI agents. Each ANT is an R2
sentant with IPUCO properties: it is Identifiable, Persistent, Ubiquitous,
Connected, and Observable. ANTs think autonomously, communicate in colonies,
and expose their knowledge through multiple interfaces.

### 1.1 Scope

The ANTHILL specification suite defines an AI agent platform where:

- Each ANT is an R2 sentant with IPUCO properties.
- Knowledge is stored as a directed graph with Bayesian confidence, managed
  by the Thurisaz engine.
- ANTs think autonomously when idle (rumination).
- ANTs communicate with each other within a colony.
- Users interact via web dashboard, Telegram, or Slack.
- Multiple AI backends (Claude, Codex, Gemini, Ollama) are supported with
  automatic fallback.

### 1.2 Design Principles

1. **Everything is events (R2-WIRE).** All behaviour is triggered by events
   flowing through the R2 wire protocol. There are no hidden side-channels.

2. **All knowledge is conjectural.** Ideas earn confidence by surviving
   refutation (Popper), never by accumulating confirmation.

3. **Diversity of evidence creates strength -- not repetition.** Ten
   independent sources outweigh a hundred echoes of one.

4. **Harmful ideas must work harder to survive.** The Thurisaz engine applies
   a beneficial impact bias so that conjectures with negative impact require
   stronger evidence to persist.

5. **Beliefs decay without fresh evidence.** Fading foundations ensure that
   stale knowledge loses confidence over time rather than ossifying.

6. **ANTs modify their own thinking process.** Self-modification allows an
   ANT to adjust its rumination strategy, evidence weighting, and topic
   priorities based on experience.

7. **Security is structural.** A 256-byte event limit on the R2 wire
   prevents prompt injection via the event bus. Events carry decisions;
   content travels on the plugin data plane.

8. **Validated writes only.** AI workers MUST NOT edit knowledge graph files
   directly. All mutations pass through the Thurisaz engine, which validates
   structure and recalculates confidence.

### 1.3 Terminology

| Term              | Definition                                                                                          |
|-------------------|-----------------------------------------------------------------------------------------------------|
| ANT               | Autonomous iNTelligenceS. An AI agent implemented as an R2 sentant.                                 |
| Colony            | A group of ANTs that share a trust group and can exchange knowledge.                                 |
| Hive              | The runtime host for one or more colonies.                                                          |
| Sentant           | An R2 entity with IPUCO properties: Identifiable, Persistent, Ubiquitous, Connected, Observable.    |
| Trust Group       | An R2-TRUST boundary that governs which sentants may communicate.                                   |
| Conjecture        | A knowledge-graph node representing a belief. Always provisional; never proven, only corroborated.   |
| Refutation        | Evidence or reasoning that challenges an existing conjecture, potentially lowering its confidence.   |
| Rumination        | Autonomous background thinking performed by an ANT when it is idle.                                 |
| Knowledge Graph   | A directed graph of conjectures, evidence, and relations maintained per ANT.                        |
| Topic Graph       | A higher-level directed graph of topics that organises the knowledge graph into navigable clusters.  |
| Meta-graph        | The graph-of-graphs that links an ANT's knowledge graph, topic graph, and colony-shared knowledge.  |
| Evidence Type     | The category of evidence supporting a conjecture (observation, testimony, analysis, inference, etc.).|
| Bayes Factor      | The likelihood ratio used by Thurisaz to update a conjecture's confidence given new evidence.        |
| Confidence        | The current belief strength of a conjecture, expressed as a probability derived from log-odds.       |
| Log-odds          | The internal representation of confidence: log(p / (1 - p)). Additive under Bayesian update.        |
| Beneficial Impact | A Thurisaz bias dimension: conjectures flagged as harmful require stronger evidence to survive.      |
| Citation          | A reference linking evidence to its source, stored in the knowledge graph via `graph_add_citation`.  |
| Worker            | A subprocess managed by the ANT conductor that executes AI inference against an external backend.    |

---

## 2. Relationship to Reality2

Anthill maps onto the R2 layered architecture as follows:

| Layer | R2 Spec    | Anthill Role                                                        |
|-------|------------|---------------------------------------------------------------------|
| L0    | R2-WIRE    | Event encoding, 256-byte envelope, CBOR serialisation               |
| L1    | R2-XPORT   | Transport bindings (TCP, WebSocket, IPC)                            |
| L2    | R2-ROUTE   | Event routing between sentants within a node                        |
| L3    | R2-MESH    | Multi-node mesh for federated colonies                              |
| L4    | R2-DISCO   | Service discovery, ANT advertisement                                |
| L5    | R2-TRUST   | Colony trust groups, device provisioning, capability tokens          |
| L6    | R2-GQL     | GraphQL API via Phoenix/Absinthe (Elixir rebuild)                   |
| L7    | --         | Anthill sentants (ANTs), plugins (AI, Telegram, Slack, Web)         |

### 2.1 Sentant Model

Sentants are pure state machines (IPUCO). They hold state and transition
logic but perform no I/O themselves. All external interaction -- AI
inference, messaging, file access -- is handled by plugins.

### 2.2 Event vs Data Plane

Events on the R2 wire carry decisions and are limited to 256 bytes. This
constraint is a security boundary: it prevents large payloads (and therefore
prompt-injection vectors) from traversing the event bus.

The plugin data plane carries content of unlimited size. AI prompts,
knowledge graph snapshots, and user messages travel on the data plane, never
on the event bus.

---

## 3. Architecture Overview

Anthill is composed of six cooperating subsystems:

```
+-------------------------------------------------------+
|                   Web Dashboard                       |
|          (Phoenix Channels, REST, GraphQL)             |
+-------------------------------------------------------+
|       Messaging Adapters (Telegram, Slack)             |
+-------------------------------------------------------+
|               Colony Supervisor                        |
|   (ANT lifecycle, trust groups, rumination scheduler)  |
+-------------------------------------------------------+
|                 ANT Sentant                            |
|    (Conductor FSM + AI / KG / Messaging plugins)      |
+-------------------------------------------------------+
|                  AI Worker                             |
| (Subprocess mgmt, multi-backend, automatic fallback)  |
+-------------------------------------------------------+
|               Knowledge Store                          |
|       (CBOR + Git, Thurisaz confidence engine)         |
+-------------------------------------------------------+
```

**Colony Supervisor.** Manages ANT lifecycle: creation, suspension,
resumption, and shutdown. Schedules rumination rounds. Enforces trust group
boundaries.

**ANT Sentant.** The conductor finite-state machine that orchestrates an
ANT's behaviour. Receives events, delegates to plugins, and transitions
state.

**AI Worker.** Manages subprocesses for AI inference. Supports Claude, Codex,
Gemini, and Ollama backends. Implements automatic fallback: if the primary
backend is unavailable, the worker MUST attempt the next configured backend
before reporting failure.

**Knowledge Store.** Persists the knowledge graph as CBOR files under Git
version control. The Thurisaz engine applies Bayesian updates, evidence
diversity scoring, beneficial impact bias, and fading foundations.

**Web Dashboard.** A Phoenix-based interface providing real-time updates via
channels, a REST API for integrations, and a GraphQL endpoint (Absinthe) for
structured queries.

**Messaging Adapters.** Telegram and Slack plugins that translate platform
messages into R2 events and relay ANT responses back to users.

### 3.1 R2 Stack Dependencies

Anthill depends on the following R2 core specifications. Implementations
MUST satisfy the referenced specs. The table also records current
implementation status in r2-core (Rust crates and Elixir NIFs).

| R2 Spec | Anthill Usage | r2-core Crate | NIF | Elixir Module | Status |
|---------|---------------|---------------|-----|---------------|--------|
| R2-WIRE | Event framing, 256-byte limit | r2-wire | wire.rs | — | Built |
| R2-FNV | Event name hashing | r2-fnv | — | — | Built |
| R2-CBOR | Knowledge graph serialisation | r2-cbor | cbor.rs | — | Built |
| R2-TRUST | Colony trust group, device provisioning, HMAC | r2-trust | trust.rs | — | Built |
| R2-SENTANT | ANT as sentant, IPUCO, conductor FSM | r2-engine | — | Anthill.Ant.Conductor | Partial |
| R2-DEF | Sentant definition format (YAML automations) | — | — | Anthill.Definitions | Partial |
| R2-PLUGIN | Plugin model (AI, Knowledge, Telegram, etc.) | r2-engine (plugin trait) | — | Anthill.Plugins.* | Partial |
| R2-GQL | GraphQL management plane | — | — | AnthillWeb.Schema | Partial |
| R2-KNOWLEDGE | Knowledge graph data model | — | epistemic.rs | Anthill.Plugins.KnowledgeHandler | Needs work |
| R2-TRANSPORT | Transport binding abstraction | r2-transport | — | — | Built |
| R2-INTERNET | WebSocket relay for federation | — | — | — | Not started |
| R2-PROVISION | Device provisioning UX flow | — | — | — | Needs work |
| TH-WEAVE | Epistemic mathematics (Bayesian) | — | epistemic.rs | — | NIF only |
| TH-REP | Source reputation scoring | — | — | — | Needs work |

**Gaps for the Elixir rebuild:**

1. **R2-KNOWLEDGE Elixir module** — the KnowledgeHandler plugin exists but
   needs the full validated store, consolidation, and CBOR persistence.
2. **R2-INTERNET** — WebSocket relay protocol for multi-node federation.
   Not yet specified or built.
3. **R2-PROVISION Elixir flow** — join codes, QR generation, device
   credential issuance. Currently in Rust only.
4. **TH-REP Elixir module** — reputation registry. Currently in Rust only.
5. **R2-GQL schema completion** — the Absinthe schema has R2 types, queries,
   mutations, and subscriptions started but needs full coverage of
   knowledge graph operations and rumination control.
6. **R2-DEF runtime loader** — definitions.ex parses YAML but needs full
   automation loading and validation.

---

## 4. Specification Suite

The ANTHILL suite comprises 20 specifications organised into four categories.

### 4.1 Core Architecture

| Spec ID            | Name                      | Description                                                                 |
|--------------------|---------------------------|-----------------------------------------------------------------------------|
| ANTHILL-INTRO      | Introduction              | This document. Vision, terminology, reading guide.                          |
| ANTHILL-SENTANT    | ANT Sentant               | Conductor FSM, state transitions, IPUCO mapping, plugin contract.           |
| ANTHILL-KNOWLEDGE  | Knowledge Graph            | Graph schema, node/edge types, CBOR encoding, Git persistence.             |
| ANTHILL-THURISAZ   | Thurisaz Engine            | Bayesian update rules, evidence diversity, fading foundations, impact bias. |
| ANTHILL-WORKER     | AI Worker                  | Subprocess lifecycle, multi-backend dispatch, fallback, timeout policy.    |
| ANTHILL-COLONY     | Colony Management          | Supervisor behaviour, ANT lifecycle, rumination scheduling.                |
| ANTHILL-FEDERATION | Federation                 | Cross-node colony mesh, knowledge synchronisation, conflict resolution.    |

### 4.2 System Operations

| Spec ID            | Name                      | Description                                                                 |
|--------------------|---------------------------|-----------------------------------------------------------------------------|
| ANTHILL-TRUST      | Trust & Security           | Trust groups, capability tokens, 256-byte event limit enforcement.         |
| ANTHILL-ONBOARDING | Onboarding                 | First-run provisioning, ANT creation wizard, default configurations.       |
| ANTHILL-CONFIG     | Configuration              | Runtime configuration schema, environment variables, secrets management.   |
| ANTHILL-STORAGE    | Storage                    | CBOR file layout, Git repository structure, backup and restore procedures. |
| ANTHILL-OBSERVE    | Observability              | Logging, metrics, tracing, health checks, supervisor status reporting.     |
| ANTHILL-UPGRADE    | Upgrade & Migration        | Version migration paths, schema evolution, rolling upgrade procedures.     |

### 4.3 User Experience

| Spec ID            | Name                      | Description                                                                 |
|--------------------|---------------------------|-----------------------------------------------------------------------------|
| ANTHILL-CHAT       | Chat Interface             | Conversation protocol, message formatting, context windowing.              |
| ANTHILL-DASHBOARD  | Web Dashboard              | Phoenix UI, real-time channels, graph visualisation, admin controls.       |
| ANTHILL-WEB        | Web API                    | REST endpoints, GraphQL schema, authentication, rate limiting.             |
| ANTHILL-TELEGRAM   | Telegram Adapter           | Bot registration, command mapping, media handling, group support.          |
| ANTHILL-SLACK      | Slack Adapter              | App manifest, slash commands, interactive messages, workspace binding.     |

### 4.4 Analysis & Output

| Spec ID            | Name                      | Description                                                                 |
|--------------------|---------------------------|-----------------------------------------------------------------------------|
| ANTHILL-RUMINATE   | Rumination                 | Autonomous thinking triggers, topic selection, depth control, self-mod.    |
| ANTHILL-EXPORT     | Export & Reporting         | Knowledge export formats, AI-written summaries, citation reports.          |

### 4.5 Dependency Graph

```
ANTHILL-INTRO
  |
  +-- ANTHILL-SENTANT ------+-- ANTHILL-WORKER
  |       |                 |
  |       +-- ANTHILL-KNOWLEDGE -- ANTHILL-THURISAZ
  |       |                        |
  |       +-- ANTHILL-RUMINATE ----+
  |
  +-- ANTHILL-COLONY -------+-- ANTHILL-TRUST
  |       |                 |
  |       +-- ANTHILL-FEDERATION
  |
  +-- ANTHILL-CHAT ---------+-- ANTHILL-DASHBOARD
  |                         |
  +-- ANTHILL-WEB ----------+
  |
  +-- ANTHILL-TELEGRAM
  +-- ANTHILL-SLACK
  +-- ANTHILL-EXPORT -------+-- ANTHILL-THURISAZ
  |
  +-- ANTHILL-CONFIG
  +-- ANTHILL-STORAGE
  +-- ANTHILL-OBSERVE
  +-- ANTHILL-ONBOARDING ---+-- ANTHILL-CONFIG
  +-- ANTHILL-UPGRADE ------+-- ANTHILL-STORAGE
```

---

## 5. Reading Guide

### 5.1 For Newcomers

Start here to understand what Anthill does and how to use it:

1. **ANTHILL-INTRO** -- this document.
2. **ANTHILL-CHAT** -- how to talk to an ANT.
3. **ANTHILL-DASHBOARD** -- navigating the web interface.

### 5.2 For Architects

Understand the internal design and knowledge model:

1. **ANTHILL-SENTANT** -- the conductor FSM and plugin architecture.
2. **ANTHILL-KNOWLEDGE** -- graph schema and persistence.
3. **ANTHILL-THURISAZ** -- the Bayesian confidence engine.
4. **ANTHILL-WORKER** -- AI backend management.
5. **ANTHILL-RUMINATE** -- autonomous thinking.

### 5.3 For Operators

Deploy, secure, and maintain an Anthill instance:

1. **ANTHILL-COLONY** -- supervisor and ANT lifecycle.
2. **ANTHILL-TRUST** -- security model and trust groups.
3. **ANTHILL-ONBOARDING** -- first-run setup.
4. **ANTHILL-CONFIG** -- configuration reference.
5. **ANTHILL-OBSERVE** -- monitoring and health checks.

### 5.4 For Elixir Developers

Contribute to the Elixir rebuild:

1. **ANTHILL-SENTANT** -- OTP process mapping for the conductor FSM.
2. **ANTHILL-WORKER** -- port/NIF integration for AI backends.
3. **ANTHILL-WEB** -- Phoenix endpoints and Absinthe schema.
4. **ANTHILL-FEDERATION** -- distributed Erlang and mesh networking.

### 5.5 For Integration Developers

Connect external systems to Anthill:

1. **ANTHILL-WEB** -- REST and GraphQL API contracts.
2. **ANTHILL-TELEGRAM** -- Telegram bot integration.
3. **ANTHILL-SLACK** -- Slack app integration.
4. **ANTHILL-EXPORT** -- extracting knowledge and reports.

---

## 6. Conformance

An implementation claiming conformance to the Anthill specification suite
MUST implement all REQUIRED behaviours defined in ANTHILL-SENTANT,
ANTHILL-KNOWLEDGE, ANTHILL-THURISAZ, ANTHILL-WORKER, and ANTHILL-COLONY.

Support for ANTHILL-TELEGRAM, ANTHILL-SLACK, ANTHILL-FEDERATION, and
ANTHILL-EXPORT is OPTIONAL but, where implemented, MUST conform to the
corresponding specification.

---

## 7. References

- Popper, K. R. *The Logic of Scientific Discovery*. Routledge, 1959.
- RFC 2119. Bradner, S. "Key words for use in RFCs to Indicate Requirement
  Levels." IETF, 1997.
- R2-WIRE, R2-SENTANT, R2-TRUST -- Reality2 specification suite.
- CBOR (RFC 7049) -- Concise Binary Object Representation.
