# ANTHILL-INTRO: Introduction to Anthill

**Version:** 0.2.0
**Date:** 2026-03-20
**Status:** Informative (not normative)

---

## 1. What is Anthill?

Anthill is a platform for hosting **ANTS** (Autonomous iNTelligenceS) — AI agents that run persistently on a server and are accessible from any device. A user's phone, laptop, or tablet connects to their colony of ANTS via a web dashboard, Telegram, or Slack. The ANTS do the thinking; the devices are just windows.

The problem Anthill solves: AI tools like Claude Code and OpenAI Codex are powerful but require a terminal on a capable machine. Anthill runs them on a server — always on, always reachable — and bridges the gap to lightweight clients.

## 2. Relationship to Reality2

Anthill is built on [Reality2 (R2)](https://github.com/reality2-ai/r2-specifications) architectural principles:

- **Sentants are pure FSMs.** The conductor sentant receives events, makes decisions, and emits actions. It holds no I/O state, no channels, no shared memory. (R2-SENTANT)
- **Plugins handle all I/O.** The AI plugin, Telegram plugin, and Slack plugin perform I/O on behalf of sentants. (R2-PLUGIN)
- **Events carry decisions (<256 bytes).** The event bus carries control signals. Large data (messages, files, AI responses) flows through plugin-to-plugin data planes. (R2-WIRE)
- **Trust groups for security.** Device provisioning, authentication, and message signing follow R2-TRUST. Each colony is a trust group; the server is the key holder.

Anthill is a **single-hive** R2 deployment. It does not use the mesh networking, beacon, or routing layers of R2 — it runs as a standalone server accessed over HTTP/WebSocket. Future versions may use R2-WIRE for direct hive-to-hive communication.

## 3. Design Principles

### 3.1 The server is the queen

The Anthill server is the key holder of the colony trust group. It does not join the colony — it *is* the colony. All client devices (phones, laptops) join via provisioned credentials.

### 3.2 ANTS are workers, not interfaces

An ANT is an AI agent with persistent memory, a working directory, and connections to one or more AI backends. Users don't interact with the AI backend directly — they interact with the ANT, which manages context, memory, and task dispatch.

### 3.3 Memory is conjectural

Knowledge is not binary (known/unknown). All relationships in an ANT's knowledge graph are **conjectures** with confidence weights. Conjectures gain strength through surviving refutation, not through confirmation. This follows Karl Popper's epistemology of science. See ANTHILL-MEMORY.

### 3.4 Multi-backend, not multi-model

Anthill abstracts AI backends (Claude Code, OpenAI Codex, Ollama, Gemini) behind a unified worker interface. An ANT can be configured to use one or more backends with fallback. The ANT's personality and memory are independent of which backend processes a given request.

Ollama also provides **embeddings** (via nomic-embed-text) for semantic knowledge graph search, enabling queries like "the project Roy is working on" to find relevant graph nodes even when exact labels don't appear in the message. When Ollama embeddings are unavailable, retrieval falls back to keyword extraction.

### 3.5 Devices are viewers

Client devices do not run AI. They are windows into the colony. A phone scanning a QR code joins the trust group and can interact with any ANT the user has access to. The same conversation is accessible from any device.

## 4. Architecture Overview

```
                    ┌─────────────────────────────────────┐
                    │          Anthill Server              │
                    │                                     │
                    │  ┌──────────┐  ┌──────────┐        │
                    │  │  ANT #1  │  │  ANT #2  │  ...   │
                    │  │          │  │          │        │
                    │  │ Conductor│  │ Conductor│        │
                    │  │ AI Plugin│  │ AI Plugin│        │
                    │  │ TG Plugin│  │ TG Plugin│        │
                    │  │ Knowledge│  │ Knowledge│        │
                    │  │  Graph   │  │  Graph   │        │
                    │  └──────────┘  └──────────┘        │
                    │                                     │
                    │  ┌──────────────────────────┐      │
                    │  │       Supervisor          │      │
                    │  │  Web Server (Axum)        │      │
                    │  │  Trust Group (R2-TRUST)   │      │
                    │  │  History Store            │      │
                    │  └──────────────────────────┘      │
                    └──────────┬──────────────────────────┘
                               │
              ┌────────────────┼────────────────┐
              │                │                │
        ┌─────────┐     ┌─────────┐     ┌─────────┐
        │  Phone  │     │ Laptop  │     │Telegram │
        │ (Web)   │     │ (Web)   │     │  Bot    │
        └─────────┘     └─────────┘     └─────────┘
```

### 4.1 Components

| Component | Role |
|-----------|------|
| **Supervisor** | Discovers ANTs, spawns them on dedicated threads, monitors for crashes, handles restarts |
| **ANT** | An R2 event bus with a conductor sentant, AI plugin, and optional Telegram/Slack plugins |
| **Conductor** | Pure FSM sentant. Routes commands (dispatch, cancel, status, help) to plugin calls |
| **AI Plugin** | Holds all I/O state. Dispatches messages to the worker, manages task tracking, sends responses |
| **AI Worker** | Background async loop. Spawns AI backend processes, streams progress, handles fallback |
| **Knowledge Graph** | Per-ANT Popperian knowledge store. Persisted to JSON, cached in memory. Semantic retrieval via Ollama embeddings |
| **Web Server** | Axum HTTP/WebSocket server. Serves the dashboard SPA, proxies to ANTs |
| **Trust Group** | R2-TRUST colony. Manages device provisioning, authentication, message signing |

## 5. Specification Conventions

All normative Anthill specifications use RFC 2119 keywords (MUST, SHOULD, MAY, etc.).

Section numbering is hierarchical: §3.2.1 is subsection 1 of section 3.2.

Dependencies between specifications are listed in the header of each document.
