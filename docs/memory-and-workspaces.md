# Memory & Workspaces

## Workspace structure

Each ANT has a working directory (set in `ant.toml`):

```
<working_dir>/
├── .git/           # Auto-initialised for backups
├── .gitignore      # Auto-created: excludes repos/
├── memory/         # Per-user persistent memory
│   ├── 123456789.md    # Telegram user
│   └── 0.md            # Web dashboard user
└── repos/          # Cloned git repositories (excluded from backup)
```

On every invocation, the ANT is automatically told about this structure — where to clone repos, where memory lives, what's backed up. You don't need to explain it.

## Per-user memory

Each user (identified by Telegram chat ID, or `0` for web dashboard) gets a persistent memory file.

The ANT is instructed to:
- **Read** the file at the start of each conversation
- **Update** it when learning something worth remembering (preferences, project context, decisions)
- **Clean up** outdated entries

Memory persists across:
- Messages within a session
- Bot restarts
- Conversation resets (`/new`)

If you tell the ANT "I prefer Python" today, it remembers next week.

## Conversation continuity

The ANT always uses `claude -p --continue` to resume the most recent Claude Code session. Sessions survive restarts. Use `/new` to start fresh.

## Git backups

The working directory is a git repository. Changes are auto-committed on a schedule.

### Enable backups

In `ant.toml`:

```toml
[claude]
backup_interval_hours = 6    # commit every 6 hours (0 = disabled)
```

### Push to GitHub

```bash
# Create a private repo
gh repo create your-org/anthill-my-ant --private

# Set up the remote
cd /path/to/working_dir
git remote add origin https://github.com/your-org/anthill-my-ant.git
git add -A && git commit -m "Initial commit"
git push -u origin master
```

In `ant.toml`:

```toml
[claude]
backup_remote = "origin"    # push after each commit
```

### What gets backed up

| Path | Backed up | Why |
|---|---|---|
| `memory/` | Yes | Per-user persistent memory |
| Files the ANT creates | Yes | Working artifacts |
| `repos/` | **No** | Cloned repos have their own git history |

### View history

```bash
cd /path/to/working_dir
git log --oneline
git diff HEAD~1
git show HEAD:memory/123456789.md
```
