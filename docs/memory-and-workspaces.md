# Memory & Workspaces

## Workspace structure

Each ANT has a working directory (set in `ant.toml`):

```
<working_dir>/
├── .git/                      # Auto-initialised for backups
├── .gitignore                 # Auto-created: excludes repos/
├── memory/
│   ├── knowledge.json         # Popperian knowledge graph (shared, structured)
│   ├── knowledge-archive.json # Archived low-confidence edges
│   ├── episodes.json          # Episodic memory (conversation summaries)
│   ├── 123456789.md           # Per-user memory (Telegram user)
│   └── 0.md                   # Per-user memory (web dashboard user)
├── files/                     # User-uploaded files
└── repos/                     # Cloned git repositories (excluded from backup)
```

## Three memory systems

### 1. Knowledge graph (`knowledge.json`)

A directed graph of entities and conjectural relationships, following **Popperian epistemology** — all knowledge is conjecture, strengthened through surviving refutation.

**Nodes** represent entities: people, projects, servers, tools, concepts, decisions, events, facts.

**Edges** are conjectures with:
- **Confidence** (0.0–1.0) — how strong this conjecture is
- **Tests** / **survived** — how many times it's been tested and survived
- **Basis** — how it was formed: `observed` (0.7), `told` (0.6), `inferred` (0.4), `assumed` (0.3)
- **Importance** — how central this is, grows with reference count
- **Time decay** — untested conjectures lose ~5% confidence per month

The AI is instructed to maintain the graph after every response: adding entities, testing existing conjectures, weakening contradictions.

**Querying:** The graph supports structured queries — "what do I know about X?" traverses from a node; "how is X connected to Y?" finds paths with cumulative confidence. See [ANTHILL-MEMORY](../specs/ANTHILL-MEMORY.md) for the full query API.

**Consolidation:** Periodically, the graph is consolidated — duplicate nodes merged, parallel edges combined, chains collapsed, contradictions flagged.

**Archiving:** Edges that fall below 10% confidence are moved to `knowledge-archive.json` — preserved for the record but no longer in the active graph.

### 2. Episodic memory (`episodes.json`)

Timestamped conversation summaries — what happened, who was involved, what was decided. The knowledge graph captures *facts*; episodes capture *stories*.

The AI writes an episode after significant conversations. Recent episodes and keyword-matching episodes are included in the prompt.

### 3. Per-user memory (`{chat_id}.md`)

Freeform notes about individual users — name, role, preferences, what they're working on. Each user (identified by Telegram chat ID, or `0` for web) has their own file.

## How memory appears in the AI prompt

The system prompt includes (in order):
1. **[KNOWLEDGE GRAPH]** — relevant entities and relationships from the graph (confidence-qualified)
2. **[EPISODES]** — recent and relevant conversation summaries
3. **[USER MEMORY]** — the current user's freeform notes

For small graphs (≤30 nodes), the full graph is shown. For larger graphs, the system extracts entity names from the user's message and uses graph traversal to render only the relevant context.

Each section is capped (graph: 4K, episodes: 2K, user memory: 4K) to avoid bloating the prompt.

## Populating the knowledge graph

The graph is populated in three ways:

1. **Automatic** — the AI updates the graph after every conversation turn (adding entities, testing conjectures)
2. **Document analysis** — `/analyse <file>` runs thematic analysis (Braun & Clarke) on a document and extracts structured knowledge
3. **Reflection** — `/reflect` performs meta-analysis on the graph itself, finding patterns, contradictions, and opportunities to consolidate

For migrating existing flat memory (`ant.md`) to the graph: `/analyse memory/ant.md`

## Conversation continuity

Sessions survive restarts — the AI backend resumes the most recent conversation in the working directory. Use `/new` to start fresh.

The `/followup` command queues a message for when the current task finishes, dispatched with session continuity (`-c` flag).

## Git backups

The working directory is a git repository. Changes are auto-committed on a schedule.

### Enable backups

In `ant.toml`:

```toml
[claude]
backup_interval_hours = 6    # commit every 6 hours (0 = disabled)
```

### Encrypted backups

```toml
[claude]
encrypt_backups = true       # encrypt memory/ and files/ before git commit
```

Uses XChaCha20-Poly1305 with a key derived from the colony key. Git history contains ciphertext; the working directory stays plaintext.

### Push to GitHub

```bash
# IMPORTANT: always use --private
gh repo create your-org/anthill-my-ant --private

cd /path/to/working_dir
git remote add origin https://github.com/your-org/anthill-my-ant.git
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
| `memory/knowledge.json` | Yes | Popperian knowledge graph |
| `memory/episodes.json` | Yes | Episodic memory |
| `memory/*.md` | Yes | Per-user memory |
| `files/` | Yes | User-uploaded files |
| `repos/` | **No** | Cloned repos have their own git history |
