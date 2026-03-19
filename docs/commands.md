# Commands

## Anthill commands

Handled locally by the conductor — always responsive, even while workers are running.

| Command | Description |
|---|---|
| `/help` or `/start` | Show available commands |
| `/ants` | Show running workers and what they're working on |
| `/usage` | Show session statistics |
| `/cancel` | Cancel the most recent worker |
| `/cancel <id>` | Cancel a specific worker by ID (shown in `/ants`) |
| `/cancel all` | Cancel all running workers |
| `/new` | Start a fresh conversation |

## AI backend commands

These slash commands are passed through to the AI backend (when supported):

| Command | Description |
|---|---|
| `/compact` | Condense conversation context |
| `/cost` | Show token/cost usage |
| `/model` | Show or change the AI model |
| `/memory` | Manage Claude's memory files |
| `/clear` | Clear conversation history |

## Raw mode keys

For `mode = "raw"` (persistent PTY). Useful for navigating TUI applications:

| Send | Does |
|---|---|
| `/enter` | Confirm |
| `/esc` | Cancel |
| `/up` `/down` `/left` `/right` | Arrow keys |
| `/tab` | Tab completion |
| `/ctrl-c` | Interrupt |
| `/ctrl-d` | EOF/exit |
| `/ctrl-z` | Suspend |
| `/space` | Space/toggle |

Everything else in raw mode is sent as text followed by Enter.
