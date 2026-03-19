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
| ANTHILL-WORKER | Multi-backend fallback | ●●●○○ | Claude + Codex tested | Gemini/Ollama detection only |
| ANTHILL-WORKER | Worker supervision | ●●●○○ | Timeout, stderr capture, stall detect | Running in production |
| ANTHILL-WORKER | Follow-up queue | ●●○○○ | Implemented | Needs more real-world testing |
| ANTHILL-WORKER | Question relay | ●○○○○ | Implemented | Depends on AskUserQuestion usage |
| ANTHILL-WORKER | Stream-json parsing | ●●●●○ | Claude + Codex formats | Text block fallback added |
| ANTHILL-THEMATIC | Document chunking | ●●●○○ | Short/long docs, paragraph breaks | Overlap + progress guarantee |
| ANTHILL-THEMATIC | Phase prompts + parsing | ●●○○○ | JSON parse, fence stripping | Needs real document testing |
| ANTHILL-THEMATIC | Graph integration | ●○○○○ | Prompt template only | AI does the integration |
| ANTHILL-WEB | Dashboard SPA | ●●●●○ | Running in production | Responsive, PWA |
| ANTHILL-WEB | WebSocket protocol | ●●●○○ | HMAC signing, snapshot, events | Unsigned fallback for HTTP |
| ANTHILL-WEB | File management | ●●○○○ | Upload, download, delete | Auth-aware download fixed |
| ANTHILL-WEB | Device QR provisioning | ●●●○○ | CLI + web QR, countdown timer | Tested on mobile |

**Legend:** ●○○○○ = implemented but untested in production, ●●●●● = battle-tested and stable.

This table is updated as features are tested and refined. Confidence grows through use, not declaration.
