# Memory & Workspaces

## Workspace structure

Each ANT has a working directory (set in `ant.toml`):

```
<working_dir>/
├── .git/                         # Thinking journal — every mutation auto-committed
├── .gitignore                    # Auto-created: excludes repos/
├── memory/
│   ├── knowledge.cbor            # Meta-graph (CBOR binary, ~46% smaller than JSON)
│   ├── graphs/
│   │   ├── <topic>.cbor          # Topic graphs (CBOR binary)
│   │   └── <topic>-archive.json  # Archived low-confidence edges
│   ├── episodes.json             # Episodic memory (conversation summaries)
│   ├── thinking_process.md       # ANT's self-evolved reasoning methodology
│   ├── questions.json            # Questions for the human from rumination
│   ├── rumination_log.json       # History of rumination cycles
│   ├── reputation.json           # Source reputation registry
│   ├── 123456789.md              # Per-user memory (Telegram user)
│   └── 0.md                      # Per-user memory (web dashboard user)
├── files/                        # User-uploaded files
└── repos/                        # Cloned git repositories (excluded from backup)
```

## The Knowledge Store

### Architecture

All knowledge graph access goes through the `KnowledgeStore` trait — a validated API boundary that the AI cannot bypass. The AI interacts through MCP tools that call trait methods. Direct file editing is not supported.

```
MCP tools / Web API / Rumination engine
    |
    v
KnowledgeStore trait (validated writes, anti-bias enforcement)
    |
    v
LiveKnowledgeStore (in-memory cache, RwLock<HashMap<String, KnowledgeGraph>>)
    |
    v
GraphEngine (petgraph, Bayesian updates, queries)
    |
    v
CborGitBackend (CBOR serialisation, atomic writes, auto-commit)
```

### CBOR storage

Graphs are stored as CBOR binary files using the [ciborium](https://crates.io/crates/ciborium) crate. CBOR is approximately 46% smaller than equivalent JSON while being serde-compatible — the same Rust structs serialize to both formats.

File layout:
- `memory/knowledge.cbor` — the meta-graph
- `memory/graphs/<topic>.cbor` — topic-specific graphs

The backend reads legacy `.json` files when no CBOR file exists, enabling seamless migration from older versions. New writes always go to CBOR.

### Validated writes

All mutations pass through builder types that enforce constraints at construction time:

- **`ValidatedNode`** — label must be non-empty and under 200 characters; kind must be one of the 21 supported types (person, project, server, tool, concept, decision, event, fact, theory, mechanism, principle, constraint, problem, claim, open_question, implementation, entity, spec, repo, platform, framework)
- **`ValidatedEdge`** — from/to labels and relation must be non-empty; basis must be observed/told/inferred/assumed; beneficial_impact is clamped to [-1.0, 1.0]
- **`ValidatedEvidence`** — evidence type must be one of the 12 defined types; test description must be non-empty; source reputation is clamped to [0.0, 1.0]

Invalid data is rejected with a descriptive error message that the AI receives through MCP.

### Git auto-commit

Every `save_graph()` call:
1. Writes CBOR to a temporary file
2. Calls `fsync` for durability
3. Renames atomically (prevents corruption on power loss)
4. Stages the `memory/` directory
5. Commits with a descriptive message (e.g. "knowledge: update anthill")

The git history becomes a **thinking journal** — you can trace exactly how and when every belief was formed, tested, strengthened, or abandoned.

## Bayesian confidence dynamics (Thurisaz engine)

### Log-odds representation

Confidence is stored as log-odds for numerical stability during sequential updates:

```
log_odds' = log_odds + ln(BF_adjusted)
probability = 1 / (1 + e^(-log_odds))
```

Log-odds are clamped to [-6.9, 6.9], corresponding to probability [0.001, 0.999]. No conjecture reaches certainty.

### Evidence types and Bayes factors

Each evidence type has a predefined base Bayes factor:

| Evidence Type | Base BF | Meaning |
|---|---|---|
| `refutation_survived` | 2.5 | Actively tried to disprove, claim held |
| `refutation_failed` | 0.1 | Actively tried to disprove, claim failed |
| `competition_won` | 2.0 | Won head-to-head against rival hypothesis |
| `corroboration` | 2.0 | Supporting evidence in another source |
| `pattern_transfer` | 1.8 | Cross-domain insight strengthens this |
| `human_attestation` | 1.5 | User confirmed or corrected |
| `consistency` | 1.5 | Consistent with existing graph |
| `synthesis` | 1.2 | Transitive inference from two strong edges |
| `inconsequential_search` | 1.0 | Searched but found nothing — **no change** |
| `inconsistency` | 0.4 | Inconsistent with existing graph |
| `competition_lost` | 0.3 | Lost head-to-head against rival |
| `contradiction` | 0.3 | Contradicting evidence found |

**InconsequentialSearch deserves emphasis**: searching for counter-evidence and finding nothing does NOT strengthen a belief. This is a structural enforcement of "absence of evidence is not evidence of absence." Only RefutationSurvived — an active, genuine attempt to disprove that fails — strengthens a claim.

### Reputation-weighted evidence

Source reliability modulates evidence strength:

```
BF_adjusted = BF_base ^ (0.5 + 0.5 * reputation)
```

- r=0.0 (untrusted): BF is square-rooted (dampened)
- r=0.5 (neutral): BF^0.75
- r=1.0 (fully trusted): full BF

Reputation can only attenuate, never amplify beyond the base BF.

### Anti-confirmation bias

Two structural mechanisms prevent the AI's training bias toward agreement from corrupting the knowledge graph:

**1. Evidence diversity ceiling** — confidence is capped based on how many different evidence types appear in the trail:

| Evidence types | Max confidence |
|---|---|
| 0-1 | 70% |
| 2 | 85% |
| 3 | 92% |
| 4+ | 99% |

An idea that has only been corroborated — even 100 times — can never exceed 70% confidence. To reach high confidence, it needs to survive refutation, win competitions, receive human attestation, show consistency with other knowledge, etc. Diversity of evidence, not quantity.

**2. Consecutive-confirmation dampening** — if the last 5+ evidence entries are all positive (BF > 1.0), the system dampens the update by pulling confidence back toward its pre-update value. Real knowledge encounters friction.

**3. Confirmation bias detection** — the system inspects the evidence trail and warns when patterns look suspicious:
- 5+ consecutive positive updates with zero negative
- High positive rate (>85%) with low type diversity (<= 2 types) across 5+ entries

Warnings are returned to the caller (and through MCP to the AI) as part of the `EdgeUpdate` result.

### Fading foundations

Beliefs decay toward uncertainty (p=0.5, log-odds=0) without fresh evidence:

```
log_odds(t) = log_odds(t_last) * 2^(-elapsed / half_life)
```

| Decay category | Half-life | Example |
|---|---|---|
| Fact | 30 days | "Anthill is written in Rust" |
| Decision | 14 days | "We chose petgraph over SurrealDB" |
| Observation | 7 days | "Alfred is running v0.4.0" |
| Inference | 3 days | "This architecture seems scalable" |
| Assumption | 1 day | "The user probably wants X" |

Decay is inferred from the edge's basis: `observed` -> Observation, `told` -> Fact, `inferred` -> Inference, `assumed` -> Assumption.

This resolves Agrippa's trilemma (Peijnenburg & Atkinson, 2017): you don't need an absolute foundation if foundations fade. Epistemic chains converge without requiring certainty at any point.

### Darwinian competition fields

Each edge carries additional fields for evolutionary competition:

- **`beneficial_impact`** (-1.0 to 1.0) — positive values mean the idea benefits people and the planet. Used as a fitness modifier in relevance scoring: `fitness = 1.0 + 0.2 * beneficial_impact` (range 0.8-1.2). Not censorship — a bias toward the constructive.
- **`corroboration_strength`** (0.0+) — how strongly this edge is supported by neighbouring edges in the graph. Computed during consolidation. Higher = better-connected in the knowledge network.
- **`competition_group`** (string) — edges with the same group ID are competing hypotheses. The rumination engine detects and runs competitions automatically.

The combined relevance score is:

```
relevance = confidence * importance * fitness * network_bonus
```

Where `network_bonus = 1.0 + 0.1 * corroboration_strength`.

## Three memory systems

### 1. Knowledge graphs (CBOR)

A directed graph of entities and conjectural relationships, following **Popperian epistemology** with **Bayesian confidence dynamics**.

**Nodes** represent entities: people, projects, servers, tools, concepts, decisions, events, facts, theories, mechanisms, principles, constraints, problems, claims, open questions, implementations, entities, specs, repos, platforms, frameworks.

**Edges** are conjectures with full epistemic metadata: log-odds confidence, evidence log, justificatory chain, decay category, beneficial impact, corroboration strength, competition group, MAGMA-inspired views, temporal validity, and provenance tracking.

**Querying:** The graph supports structured queries — "what do I know about X?" traverses from a node; "how is X connected to Y?" finds paths with cumulative confidence; "what am I unsure about?" returns edges below a threshold; "why do I believe this?" returns the justificatory chain. See [ANTHILL-MEMORY](../specs/ANTHILL-MEMORY.md) for the full query API.

**Consolidation:** Periodically, the graph is consolidated — duplicate nodes merged (Levenshtein fuzzy matching), parallel edges combined (MAX confidence, cap 0.95), chains collapsed, contradictions flagged, communities detected, corroboration strength recomputed.

**Archiving:** Edges that fall below 10% confidence are moved to archive files.

### 2. Episodic memory (`episodes.json`)

Timestamped conversation summaries — what happened, who was involved, what was decided. The knowledge graph captures *facts*; episodes capture *stories*.

The AI writes an episode after significant conversations. Recent episodes and keyword-matching episodes are included in the prompt. Episodes link to knowledge graph nodes via an `entities[]` field for cross-referencing narrative and structured knowledge.

### 3. Per-user memory (`{chat_id}.md`)

Freeform notes about individual users — name, role, preferences, what they're working on. Each user (identified by Telegram chat ID, or `0` for web) has their own file.

## Rumination

When idle, ANTs think autonomously. The rumination engine runs periodic cycles that include:

1. **Corroboration strength computation** — measure how strongly each edge is supported by its network neighbourhood
2. **Synthesis** — find A->B->C paths where no A->C edge exists; create transitive inferences (BF=1.2); no AI tokens required
3. **Undetermined connections** — investigate '?' edges; ask AI to determine the relationship or flag a question
4. **Darwinian competition** — detect competing hypotheses; pit them head-to-head; award CompetitionWon/CompetitionLost evidence
5. **Cross-domain pattern transfer** — find structural similarities between topic graphs; award PatternTransfer evidence (BF=1.8)
6. **Active refutation** — select important but uncertain edges; actively try to disprove them; RefutationSurvived (BF=2.5) or RefutationFailed (BF=0.1)
7. **Contradiction resolution** — find edge pairs where both cannot be true; send to AI for resolution
8. **Autonomous initiative** — identify knowledge gaps; write questions to `questions.json` for the human
9. **Meta-rumination** — review the thinking process itself; read and potentially modify `thinking_process.md`

After each cycle: consolidation, orphan linking, git commit ("rumination cycle complete").

### Questions queue

When rumination encounters gaps it cannot fill autonomously — undetermined connections, unresolvable contradictions, missing context — it writes questions to `memory/questions.json`. These are surfaced to the human via the `/questions` command or the web dashboard.

### Self-evolving methodology

Each ANT maintains `memory/thinking_process.md` — a document describing its evolved approach to reasoning. This file is loaded into the system prompt and is itself a conjecture: during meta-rumination, the ANT can review its recent performance and modify its own methodology. The content of this file is wholly written by the ANT.

## How memory appears in the AI prompt

The system prompt includes (in order):
1. **[KNOWLEDGE GRAPH]** — relevant entities and relationships from the graph (confidence-qualified)
2. **[EPISODES]** — recent and relevant conversation summaries
3. **[USER MEMORY]** — the current user's freeform notes
4. **[THINKING PROCESS]** — the ANT's self-evolved methodology (up to 2KB)

For small graphs (<=30 nodes), the full graph is shown. For larger graphs, the system extracts entity names from the user's message and uses graph traversal (or semantic nearest-neighbour when embeddings are available) to render only the relevant context.

Each section is capped (graph: ~4K, episodes: ~2K, user memory: ~4K, thinking process: ~2K) under a total 16KB system prompt budget.

Edges below `MIN_PROMPT_CONFIDENCE` (0.15) are excluded from the prompt. High-confidence edges (>=0.8) are rendered without qualifiers. Lower-confidence edges include a visual confidence indicator: `[62%]`.

## Populating the knowledge graph

The graph is populated in four ways:

1. **Automatic** — the AI applies evidence through MCP tools after every conversation turn
2. **Document analysis** — `/analyse <file>` runs thematic analysis (Braun & Clarke) on a document and extracts structured knowledge
3. **Reflection** — `/reflect` performs meta-analysis on the graph itself, finding patterns, contradictions, and opportunities to consolidate
4. **Rumination** — the autonomous thinking engine creates synthesis edges, runs competitions, resolves contradictions, and transfers patterns across domains

## Conversation continuity

Sessions survive restarts — the AI backend resumes the most recent conversation in the working directory. Use `/new` to start fresh.

The `/followup` command queues a message for when the current task finishes, dispatched with session continuity (`-c` flag).

## Git backups

The working directory is a git repository. Changes are auto-committed on every knowledge store mutation. Periodic backups can also be configured on a schedule.

### Enable scheduled backups

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
| `memory/*.cbor` | Yes | Knowledge graphs (CBOR binary) |
| `memory/graphs/*.cbor` | Yes | Topic graphs |
| `memory/episodes.json` | Yes | Episodic memory |
| `memory/*.md` | Yes | Per-user memory, thinking process |
| `memory/*.json` | Yes | Questions, rumination log, reputation |
| `files/` | Yes | User-uploaded files |
| `repos/` | **No** | Cloned repos have their own git history |
