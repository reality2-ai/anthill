# Commands

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

Everything else sent to an ANT is dispatched as a prompt to the AI backend. Multiple messages run concurrently.
