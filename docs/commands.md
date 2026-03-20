# Commands

Type `/` in the web dashboard to see the autocomplete menu with all available commands.

## Anthill commands

Handled locally by the conductor — always responsive, even while workers are running.

| Command | Description |
|---|---|
| `/help` or `/start` | Show available commands |
| `/status` | Live view of each worker: backend, progress, elapsed time, follow-ups |
| `/ants` | Show running workers and what they're working on |
| `/usage` | Show session statistics |
| `/cancel` | Cancel the most recent worker |
| `/cancel <id>` | Cancel a specific worker by ID (shown in `/status`) |
| `/cancel all` | Cancel all running workers |
| `/followup <text>` | Queue a message for when the current task finishes (session continuity) |
| `/new` | Start a fresh conversation |

## Analysis commands

AI-driven pipelines using thematic analysis methodology.

| Command | Description |
|---|---|
| `/analyse <file>` | Run thematic analysis on a document — extract entities, themes, relationships → knowledge graph |
| `/reflect` | Meta-analysis of the knowledge graph itself — find patterns, test conjectures, consolidate |
| `/specify <file>` | Generate a formal specification (RFC 2119 style) from source code |
| `/test-vectors <file>` | Generate test cases from source code or a specification |

### How analysis commands work

All four follow the same thematic analysis pattern (Braun & Clarke, 2022):

1. **Familiarise** — read and chunk the source material
2. **Code** — extract entities, behaviors, or concepts
3. **Theme** — group codes into higher-level patterns
4. **Review** — validate against the source
5. **Refine** — name, merge, build relationships
6. **Integrate** — write to knowledge graph, spec file, or test file

`/analyse` and `/reflect` update the knowledge graph. `/specify` writes a spec to `specs/`. `/test-vectors` outputs test cases and Rust `#[test]` stubs.

## AI backend commands

These slash commands are passed through to the AI backend (when supported):

| Command | Description |
|---|---|
| `/compact` | Condense conversation context |
| `/cost` | Show token/cost usage |
| `/model` | Show or change the AI model |
| `/memory` | Manage memory files |
| `/clear` | Clear conversation history |

## CLI commands

Run from the server terminal:

| Command | Description |
|---|---|
| `anthill --qr-join` | Generate QR code — scan with phone to join colony |
| `anthill --qr-join --hostname X` | QR with custom hostname in URL |
| `anthill --join-code` | Generate a text join code |
| `anthill --export-key` | Show colony key (for password manager) |
| `anthill --export-key --qr` | Show colony key as QR code |
| `anthill --import-key <key>` | Restore colony key from backup |
| `anthill --doctor` | Diagnostic check — verifies Rust, Claude, Codex, Ollama, models, Git, Tailscale, config, colony key, ANTs, devices, service status |

## Special prefixes

| Prefix | Description |
|---|---|
| `!` | **Interrupt**: cancel the running task and restart with both the original prompt and the new message combined. Example: `! actually use the v2 API instead` |

## Auto-followup

When exactly one task is running, any new message you send is automatically queued as a follow-up — no need to type `/followup`. The queued message runs with session continuity when the current task completes.

If multiple tasks are running, messages start new concurrent tasks as before.

Everything else sent to an ANT is dispatched as a prompt to the AI backend. Multiple messages run concurrently.
