<p align="center">
  <img src="docs/logo.svg" alt="Anthill" width="600">
</p>

<p align="center">
  <em>A colony for <strong>ANTS</strong> — Autonomous iNTelligenceS</em>
</p>

---

Anthill runs AI agents on a server and lets you interact with them from any device — phone, laptop, tablet — via a built-in web dashboard, [Telegram](https://telegram.org/), or [Slack](https://slack.com/). It uses [Reality2](https://reality2.ai) (R2), an event-driven architecture where sentants (pure state machines) make decisions and plugins handle all I/O — connecting to AI backends like [Claude](https://www.anthropic.com/), [ChatGPT](https://openai.com/), or local models via [Ollama](https://ollama.com/).

Each ANT has its own personality, workspace, persistent memory, and can run multiple tasks concurrently. Access is secured by R2's trust group model — devices join the colony via one-time codes, and every request is authenticated with HMAC-signed envelopes. The web dashboard is accessed securely over [Tailscale](https://tailscale.com/).

> Anthill runs on **Linux**, **macOS**, and **FreeBSD**. The install script auto-detects your platform. See [Prerequisites](docs/prerequisites.md) for setup details.

## Quick start

```bash
git clone https://github.com/reality2-ai/anthill.git
cd anthill
./install.sh                     # builds, installs binary, sets up service
anthill --qr-join                # show QR code — scan with phone to join
# Or: anthill --join-code        # text code for manual entry
# Open http://localhost:3000 (or your Tailscale hostname)
# Create your first ANT from the web dashboard (+ button)
```

## How it works

[Reality2](https://reality2.ai) (R2) is a software stack for wearables, IoT, and autonomous agents. Anthill is the first production R2 application outside hardware — proving the architecture works for AI agents, not just sensors.

**Sentants** are pure state machines — they receive events, make decisions, emit actions. No I/O, no side effects. Given the same events, they always produce the same output.

**Plugins** are service adapters — they bridge external systems (AI backends, Telegram, Slack, web servers) into the R2 event bus. All I/O happens here.

**Events carry decisions** (< 256 bytes). **Plugins carry data** (unlimited). This separation is enforced by design — the 256-byte limit ensures sentants never see raw content, making prompt injection via the event bus structurally impossible.

![Anthill Architecture](https://mermaid.ink/img/Z3JhcGggTFIKICAgIHN1YmdyYXBoIFZpZXdlcnMKICAgICAgICBQaG9uZVtQaG9uZSBicm93c2VyXQogICAgICAgIExhcHRvcFtMYXB0b3AgYnJvd3Nlcl0KICAgICAgICBUR1tUZWxlZ3JhbSBhcHBdCiAgICAgICAgU0xbU2xhY2sgYXBwXQogICAgZW5kCgogICAgc3ViZ3JhcGggU2VydmVyW1NlcnZlciAtIHRoZSBRdWVlbl0KICAgICAgICBzdWJncmFwaCBBTlRbRWFjaCBBTlRdCiAgICAgICAgICAgIHN1YmdyYXBoIEZTTXNbU2VudGFudCAtIHB1cmUgRlNNXQogICAgICAgICAgICAgICAgQ1NbQ29uZHVjdG9yXQogICAgICAgICAgICBlbmQKICAgICAgICAgICAgc3ViZ3JhcGggUGx1Z2luc1tQbHVnaW5zIC0gYWxsIEkvT10KICAgICAgICAgICAgICAgIEFQW0FJUGx1Z2luXQogICAgICAgICAgICAgICAgVFBbVGVsZWdyYW1QbHVnaW5dCiAgICAgICAgICAgICAgICBTUFtTbGFja1BsdWdpbl0KICAgICAgICAgICAgZW5kCiAgICAgICAgICAgIHN1YmdyYXBoIE1lbW9yeVtNZW1vcnldCiAgICAgICAgICAgICAgICBLR1tLbm93bGVkZ2UgR3JhcGhdCiAgICAgICAgICAgICAgICBFUFtFcGlzb2Rlc10KICAgICAgICAgICAgICAgIFVNW1VzZXIgTWVtb3J5XQogICAgICAgICAgICBlbmQKICAgICAgICAgICAgV1tXb3JrZXIgKyBXYXRjaGRvZ10KICAgICAgICBlbmQKICAgICAgICBTVVBbU3VwZXJ2aXNvcl0KICAgICAgICBUUlVTVFtSMi1UUlVTVCBDb2xvbnldCiAgICAgICAgV0VCW1dlYiBTZXJ2ZXJdCiAgICBlbmQKCiAgICBzdWJncmFwaCBCYWNrZW5kc1tBSSBCYWNrZW5kc10KICAgICAgICBDbGF1ZGVbQ2xhdWRlIENvZGVdCiAgICAgICAgQ29kZXhbT3BlbkFJIENvZGV4XQogICAgICAgIE9MW09sbGFtYV06OjpmdXR1cmUKICAgIGVuZAoKICAgIFBob25lIDwtLT58dHJ1c3QgZ3JvdXAgYXV0aHwgV0VCCiAgICBMYXB0b3AgPC0tPnx0cnVzdCBncm91cCBhdXRofCBXRUIKICAgIFRHIDwtLT4gVFAKICAgIFNMIDwtLT4gU1AKICAgIFdFQiA8LS0+fGV2ZW50c3wgQ1MKICAgIFRQIDwtLT58ZXZlbnRzfCBDUwogICAgU1AgPC0tPnxldmVudHN8IENTCiAgICBDUyA8LS0+fHBsdWdpbl9jYWxsfCBBUAogICAgQVAgPC0tPnxkYXRhIHBsYW5lfCBUUAogICAgQVAgPC0tPnxkYXRhIHBsYW5lfCBTUAogICAgQVAgPC0tPnxkYXRhIHBsYW5lfCBXRUIKICAgIEFQIC0tPiBXCiAgICBXIDwtLT4gQ2xhdWRlCiAgICBXIDwtLT4gQ29kZXgKICAgIFcgLS4tPnxjb21pbmcgc29vbnwgT0wKICAgIFcgLS0+IEtHCiAgICBXIC0tPiBFUAoKICAgIGNsYXNzRGVmIGZ1dHVyZSBzdHJva2UtZGFzaGFycmF5OiA1IDUsb3BhY2l0eTowLjUK)

## Memory: Popperian knowledge graph

Each ANT maintains a **knowledge graph** where all relationships are **conjectures** — not facts. Following Karl Popper's epistemology, knowledge gains strength through surviving refutation, not through confirmation.

```
Roy (person)
  → works_on → Anthill [●●●● 85%, 12× tested]
  → prefers → Rust     [●●●○ 62%, 3× tested]

Anthill (project)
  → deployed_on → Alfred  [●●●○ 72%]
  → written_in → Rust     [●●●● 90%]
  → may_target → ESP32-S3 [●●○○ 35%, assumed, untested]
```

**How confidence works:**
- New conjectures start at 0.3–0.7 depending on how they were formed (observed > told > inferred > assumed)
- Surviving a test (encountered in conversation, no contradiction) → confidence increases
- Failing a test (evidence weakens it) → confidence decreases
- Direct contradiction → confidence drops 70%
- Untested conjectures decay ~5% per month
- Below 15% → hidden from the AI prompt. Below 10% → archived

Three memory systems work together:

| System | File | Purpose |
|---|---|---|
| **Knowledge graph** | `memory/knowledge.json` | Entities and conjectural relationships (shared across all users) |
| **Episodic memory** | `memory/episodes.json` | Timestamped conversation summaries — what happened, not just what's true |
| **Per-user memory** | `memory/{chat_id}.md` | Individual preferences, name, role |

The AI actively maintains all three after every response — adding entities, testing conjectures, writing episode summaries. A **graph query API** supports traversal ("what do I know about X?"), path-finding ("how is X connected to Y?"), and uncertainty queries ("what am I unsure about?").

See [ANTHILL-MEMORY](specs/ANTHILL-MEMORY.md) for the full specification.

## Features

- **Multiple ANTS** — each with its own personality, workspace, and optional Telegram/Slack
- **Popperian knowledge graph** — structured memory where relationships are conjectures that strengthen through surviving refutation, not confirmation
- **Episodic memory** — conversation summaries capture the narrative, not just facts
- **Graph query API** — traversal, path-finding, kind filtering, uncertainty queries — all with confidence scores
- **Multi-backend AI** — Claude Code, OpenAI Codex (Ollama, Gemini coming). Automatic fallback on failure/rate limits
- **Worker supervision** — watchdog per task, stall detection, timeout killing, stderr capture
- **Follow-up queue** — inject context into running tasks; answers routed to the right worker
- **Web dashboard** — responsive PWA, real-time progress, reply-to-message, file browser
- **QR device provisioning** — scan to join the colony from any phone
- **Trust group security** — R2-TRUST Ed25519 identity, HMAC-signed WebSocket, join codes
- **Concurrent tasks** — multiple workers per ANT; `/status` shows live progress per worker
- **Git-backed workspace** — auto-committed on schedule, optionally encrypted and pushed
- **Cross-channel sync** — messages forwarded between web, Telegram, and Slack (opt-in)
- **Auto-restart** — supervisor with exponential backoff, hot-add of new ANTS
- **Cross-platform** — Linux (systemd), macOS (launchd), FreeBSD (rc.d)

## Documentation

| Guide | What it covers |
|---|---|
| [Prerequisites](docs/prerequisites.md) | Rust, AI backend, Telegram/Slack, Tailscale |
| [Getting Started](docs/getting-started.md) | Install, create your first ANT, send a message |
| [Production Setup](docs/production-setup.md) | Multiple ANTS, supervisor, auto-start |
| [Web Dashboard](docs/web-dashboard.md) | Tailscale HTTPS, PWA, cross-device history |
| [Configuration](docs/configuration.md) | Full reference for supervisor.toml and ant.toml |
| [Commands](docs/commands.md) | Telegram and dashboard commands |
| [Memory & Workspaces](docs/memory-and-workspaces.md) | Knowledge graph, episodic memory, git backups |
| [Architecture](docs/architecture.md) | R2 sentant engine, events, plugins |
| [Troubleshooting](docs/troubleshooting.md) | Common issues and fixes |
| [Security](docs/security.md) | Trust groups, device provisioning, access control |

## Specifications

Formal specifications for Anthill's architecture, following the R2-specifications style:

| Spec | What it covers |
|---|---|
| [ANTHILL-INTRO](specs/ANTHILL-INTRO.md) | Vision, R2 relationship, design principles |
| [ANTHILL-COLONY](specs/ANTHILL-COLONY.md) | Supervisor, ANT lifecycle, trust groups, provisioning |
| [ANTHILL-MEMORY](specs/ANTHILL-MEMORY.md) | Popperian knowledge graph, episodic memory, query API |
| [ANTHILL-WORKER](specs/ANTHILL-WORKER.md) | AI worker lifecycle, multi-backend, supervision |
| [ANTHILL-WEB](specs/ANTHILL-WEB.md) | Web dashboard, WebSocket protocol, REST API |

## Security

Anthill implements defence in depth:

| Layer | What it does | Applies to |
|---|---|---|
| [Tailscale](https://tailscale.com/) | Network encryption (WireGuard) — only your devices can reach the server | Web dashboard |
| HTTPS | Transport encryption via Tailscale's automatic certificates | Web dashboard |
| Trust group | Authentication — devices join with one-time codes, every request verified | Web dashboard |
| HMAC envelopes | Message integrity — every WebSocket message signed, timestamped against replay | Web dashboard |
| XChaCha20-Poly1305 | Payload encryption for backups (opt-in) | Git backup |
| R2 architecture | Structural isolation — sentants see only 12-byte decisions, never raw content | All channels |

The **web dashboard** carries all six layers — use it for sensitive operations. **Telegram and Slack** are convenience channels with weaker security (third-party TLS, no message signing). See [Security](docs/security.md) for details.

## Why not just use OpenClaw?

[OpenClaw](https://www.zdnet.com/article/openclaw-moltbot-clawdbot-5-reasons-viral-ai-agent-security-nightmare/) went viral as an "AI that actually does things" — but security researchers [flagged serious concerns](https://www.zdnet.com/article/openclaw-moltbot-clawdbot-5-reasons-viral-ai-agent-security-nightmare/). Anthill addresses each one:

| OpenClaw problem | Anthill approach |
|---|---|
| **Exposed credentials** — leaked API keys, no auth | Trust group — colony.key stays on server, join codes expire in 5 min |
| **No authentication** — instances open on the internet | Auth middleware on every endpoint, HMAC-signed WebSocket |
| **Full system access** — broad permissions, no isolation | Per-ANT workspaces, sentant/plugin separation |
| **Prompt injection** — malicious content hijacks the agent | 256-byte event limit, untrusted content never reaches sentants |
| **Malicious plugins** — fake extensions, backdoored skills | No marketplace — plugins compiled from source |
| **Viral fakes** — scam repos, impostor extensions | Self-hosted single binary, no third-party downloads |

The security is structural (enforced by architecture), not aspirational (hoping users configure it right).

## License

MIT OR Apache-2.0
