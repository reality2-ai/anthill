# Anthill — Specification Suite

Specifications for **Anthill**: a platform for hosting Autonomous iNTelligenceS (ANTS) — AI agents that run on a server and are accessible from any device via web, Telegram, and Slack.

Anthill is built on [Reality2 (R2)](https://github.com/reality2-ai/r2-specifications) principles: sentants are pure FSMs, plugins handle I/O, events carry decisions.

## Reading Order

**Start here:** [ANTHILL-INTRO](ANTHILL-INTRO.md) — vision, architecture, and relationship to R2.

### Architecture
1. [ANTHILL-INTRO](ANTHILL-INTRO.md) — Vision, philosophy, design principles
2. [ANTHILL-COLONY](ANTHILL-COLONY.md) — Colony model: supervisor, ANTs, trust groups, device provisioning

### Memory and Knowledge
3. [ANTHILL-MEMORY](ANTHILL-MEMORY.md) — Popperian knowledge graph, episodic memory, query API, consolidation

### Workers and AI
4. [ANTHILL-WORKER](ANTHILL-WORKER.md) — AI worker lifecycle, multi-backend, supervision, follow-ups

### Document Analysis
5. [ANTHILL-THEMATIC](ANTHILL-THEMATIC.md) — Thematic analysis pipeline for document→graph conversion

### Interfaces
6. [ANTHILL-WEB](ANTHILL-WEB.md) — Web dashboard, WebSocket protocol, authentication

## Specification Confidence

How well-tested and stable is each area? Confidence grows through implementation, testing, and surviving real-world use — not through assertion.

| Spec | Area | Confidence | Tests | Notes |
|------|------|-----------|-------|-------|
| ANTHILL-COLONY | Supervisor lifecycle | ●●●●○ | Crash recovery, hot-add, restart limits | Running in production |
| ANTHILL-COLONY | Trust / provisioning | ●●●○○ | Join codes, Ed25519 auth, device revoke | QR join tested, multi-device tested |
| ANTHILL-COLONY | ANT configuration | ●●●●○ | TOML roundtrip, typed serialisation | Stable schema |
| ANTHILL-MEMORY | Knowledge graph structure | ●●●○○ | Load/save, node/edge CRUD, rendering | New — needs production use |
| ANTHILL-MEMORY | Popperian confidence | ●●○○○ | Strengthen/weaken/contradict/decay | Implemented, not yet AI-exercised |
| ANTHILL-MEMORY | Query API | ●●○○○ | About, Path, ByKind, Uncertain | Implemented, not yet prompt-integrated at scale |
| ANTHILL-MEMORY | Episodic memory | ●○○○○ | Load/save/search | Implemented, AI not yet writing episodes |
| ANTHILL-MEMORY | Consolidation | ●●○○○ | Dedup, merge, collapse, contradiction | Implemented, not yet run at scale |
| ANTHILL-MEMORY | Keyword extraction (i18n) | ●●●○○ | English, French, German tested | No CJK support |
| ANTHILL-WORKER | Multi-backend fallback | ●●●○○ | Claude + Codex + Ollama tested | Ollama integrated, Gemini detection only |
| ANTHILL-WORKER | Ollama embeddings | ●●○○○ | nomic-embed-text integration | Semantic search with keyword fallback |
| ANTHILL-WORKER | Worker supervision | ●●●○○ | Timeout, stderr capture, stall detect | Running in production |
| ANTHILL-WORKER | Follow-up queue | ●●●○○ | Auto-followup, ! interrupt | Auto-queue on message-while-running |
| ANTHILL-WORKER | Web command routing | ●●●○○ | /help /status /usage /ants /cancel via web | Previously Telegram/Slack only |
| ANTHILL-WORKER | Question relay | ●○○○○ | Implemented | Depends on AskUserQuestion usage |
| ANTHILL-WORKER | Stream-json parsing | ●●●●○ | Claude + Codex formats | Text block fallback added |
| ANTHILL-THEMATIC | Document chunking | ●●●○○ | Short/long docs, paragraph breaks | Overlap + progress guarantee |
| ANTHILL-THEMATIC | Phase prompts + parsing | ●●○○○ | JSON parse, fence stripping | Needs real document testing |
| ANTHILL-THEMATIC | Graph integration | ●○○○○ | Prompt template only | AI does the integration |
| ANTHILL-THEMATIC | Spec generation (/specify) | ●●○○○ | Implemented | Generates RFC 2119 specs from code |
| ANTHILL-THEMATIC | Test vectors (/test-vectors) | ●●○○○ | Implemented | Generates test cases from code or specs |
| ANTHILL-WEB | Dashboard SPA | ●●●●○ | Running in production | Responsive, PWA |
| ANTHILL-WEB | WebSocket protocol | ●●●○○ | HMAC signing, snapshot, events | Unsigned fallback for HTTP |
| ANTHILL-WEB | File management | ●●○○○ | Upload, download, delete | Auth-aware download fixed |
| ANTHILL-WEB | Device QR provisioning | ●●●○○ | CLI + web QR, countdown timer | Tested on mobile |
| ANTHILL-WEB | Slash command autocomplete | ●●●○○ | Menu renders, Tab/Enter selects | Web UI only |
| ANTHILL-WEB | ANT not-running feedback | ●●●○○ | Error on send to stopped ANT | Prevents silent message loss |
| ANTHILL-WORKER | UTF-8 safety | ●●●●○ | Char/word-boundary slicing | Māori macrons, emoji safe |
| ANTHILL-COLONY | Supervisor crash broadcasts | ●●●○○ | Crash/restart events to web UI | Real-time status updates |
| ANTHILL-COLONY | Doctor diagnostics | ●●●○○ | CLI + web API | Checks all prerequisites and config |
| ANTHILL-MEMORY | Edge views (MAGMA) | ●●○○○ | Semantic, temporal, causal, entity | Multi-perspective graph queries |
| ANTHILL-MEMORY | Temporal validity (Zep) | ●●○○○ | valid_from/valid_until on edges | Time-scoped knowledge |
| ANTHILL-MEMORY | Provenance tracking | ●●○○○ | Source field on edges | "Why do I believe this?" tracing |
| ANTHILL-MEMORY | Community detection | ●○○○○ | Connected component analysis | GraphRAG-inspired, during consolidation |
| ANTHILL-MEMORY | Episode entity linking | ●●○○○ | episodes.entities[] → graph nodes | Cross-reference narrative and structure |
| ANTHILL-MEMORY | Embedding retrieval | ●●○○○ | Ollama nomic-embed-text | Semantic search with keyword fallback |
| ANTHILL-MEMORY | Confidence decay (time) | ●●●○○ | 24h idle trigger | Time-based, not just request-count |
| ANTHILL-WORKER | Sensitive op restriction | ●●●○○ | /analyse /specify /test-vectors | Blocked from Telegram/Slack |

**Legend:** ●○○○○ = implemented but untested in production, ●●●●● = battle-tested and stable.

This table is updated as features are tested and refined. Confidence grows through use, not declaration.
