# ANTHILL-RUMINATION: Autonomous Thinking Engine

| Field      | Value                                                        |
|------------|--------------------------------------------------------------|
| Version    | 0.1 Draft                                                    |
| Date       | 2026-03-30                                                   |
| Status     | Draft                                                        |
| Depends on | ANTHILL-KNOWLEDGE, ANTHILL-THURISAZ, ANTHILL-WORKER          |
| Related    | ANTHILL-COMMS, ANTHILL-COLONY                                |

> The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT",
> "SHOULD", "SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this
> document are to be interpreted as described in RFC 2119.

---

## 1. Introduction

ANTs think autonomously when they are idle. This background cognition is
called *rumination*. Rumination is not random exploration -- it follows a
structured epistemic cycle designed to strengthen good ideas, weaken bad
ones, find gaps, and resolve contradictions.

The cycle is grounded in Popperian epistemology: ideas earn confidence by
surviving genuine refutation attempts, never by accumulating confirmation.
A belief that has been actively challenged with external evidence and
survived is stronger than one that has simply never been questioned.

Rumination operates in two modes:

1. **Automatic** -- triggered by the maintenance loop when the ANT detects
   sustained idleness. Executes a full multi-phase epistemic cycle.
2. **Manual** -- triggered by the `/ruminate` or `/citations` commands.
   Dispatches focused, independent tasks in parallel.

Both modes send work to the AI worker subsystem (ANTHILL-WORKER) with
`source` set to `"rumination"` and `chat_id` set to the reserved constant
`-1` to avoid collision with real user sessions.

Every rumination prompt MUST end with the stop directive defined in
Section 4.9, which instructs the AI to complete its work, write any
unresolvable questions to `memory/questions.json`, and halt without
requesting further input.

---

## 2. Configuration

Rumination behaviour is controlled by the `RuminationConfig` structure,
serialised in the `[claude.rumination]` section of the ANT's `ant.toml`
configuration file.

| Field                      | Type       | Default | Description                                                              |
|----------------------------|------------|---------|--------------------------------------------------------------------------|
| `enabled`                  | `bool`     | `false` | Enable the automatic rumination engine.                                  |
| `interval_secs`            | `u64`      | `7200`  | Minimum interval between automatic rumination cycles (seconds).          |
| `min_idle_secs`            | `u64`      | `300`   | Minimum continuous idle time before a cycle MAY begin (seconds).         |
| `topics`                   | `Vec<String>` | `[]` | Topic graphs to focus on. Empty means all non-meta graphs.               |
| `refutation_enabled`       | `bool`     | `true`  | Enable the active refutation phase (Section 4.5).                        |
| `synthesis_enabled`        | `bool`     | `true`  | Enable the synthesis phase (Section 4.2).                                |
| `contradiction_resolution` | `bool`     | `true`  | Enable the contradiction resolution phase (Section 4.6).                 |
| `initiative_enabled`       | `bool`     | `false` | Enable the autonomous initiative phase (Section 4.7).                    |

An implementation MUST respect the `enabled` flag: when `false`, no
automatic rumination cycles SHALL be initiated. Manual rumination
(`/ruminate`, `/citations`) MUST remain available regardless of this flag.

When `topics` is non-empty, only graphs whose names appear in the list
SHALL be considered during topic iteration. The `meta` graph MUST always
be excluded from topic iteration.

### 2.1 Example Configuration

```toml
[claude.rumination]
enabled = true
interval_secs = 3600
min_idle_secs = 600
topics = ["philosophy", "engineering"]
refutation_enabled = true
synthesis_enabled = true
contradiction_resolution = true
initiative_enabled = true
```

---

## 3. Idle Detection

The maintenance loop runs as a background tokio task per ANT. It wakes
every 300 seconds (5 minutes) to check whether rumination conditions are
met.

### 3.1 Idle State Machine

The implementation MUST track three timestamps:

- `last_rumination` -- the `Instant` of the most recent rumination cycle.
- `idle_since` -- the `Instant` when the ANT first became idle, or `None`
  if the ANT is currently busy.
- `now` -- the current `Instant` at each wake.

On each wake, the implementation MUST:

1. Acquire the shared `TaskMap` lock and check whether it is empty.
2. If the map is empty (ANT is idle):
   a. If `idle_since` is `None`, set it to `now`.
   b. Compute `idle_duration = now - idle_since`.
   c. Compute `since_last = now - last_rumination`.
   d. If `idle_duration >= min_idle_secs` AND `since_last >= interval_secs`,
      initiate a rumination cycle and set `last_rumination = now`.
3. If the map is not empty (ANT is busy):
   a. Reset `idle_since` to `None`.

This design ensures that:

- Brief idle gaps (less than `min_idle_secs`) do not trigger rumination.
- Rumination never fires more frequently than `interval_secs`.
- Busy periods reset the idle counter, so the ANT must be continuously
  idle for the minimum duration.

### 3.2 Startup Delay

The maintenance loop MUST wait 60 seconds after spawning before its first
check, to allow the ANT's other subsystems time to initialise.

### 3.3 Prerequisites

A rumination cycle MUST NOT start unless both of the following are
available:

- `request_tx` -- an unbounded channel sender to the AI worker.
- `tasks` -- a shared `TaskMap` for idle detection.

If either is `None`, the automatic rumination engine SHALL be silently
disabled.

---

## 4. Automatic Rumination Cycle

When idle detection triggers a cycle, the implementation MUST execute the
following phases in order. Each phase is an atomic "thought" -- the
knowledge store's `begin_thought()` / `end_thought()` bracketing MUST be
used where the phase modifies the graph directly (phases 4.1 and 4.2).
Phases that dispatch prompts to the AI worker (4.3 through 4.8) create
work items that the worker processes asynchronously.

After all phases complete, the implementation MUST:

1. Post a human-readable summary of the cycle to the ANT's chat history.
2. Run a full consolidation pass (Section 10).
3. Create a Git commit of `memory/` with the message
   `"rumination cycle complete"`.

### 4.1 Corroboration Strength Computation

**Scope:** All graphs with at least 2 nodes.

The implementation MUST call `compute_corroboration_strength()` on every
qualifying graph. This recalculates how well each edge is supported by
the surrounding graph structure (see ANTHILL-THURISAZ). The phase MUST be
wrapped in a `begin_thought()` / `end_thought()` pair.

### 4.2 Synthesis

**Guard:** `synthesis_enabled` MUST be `true`.

Synthesis finds transitive paths A -> B -> C in the graph and creates
direct A -> C edges, conjecturing that the transitive relationship holds.

For each topic graph, the implementation MUST:

1. Call `synthesis_candidates(topic, 5)` to retrieve up to 5 candidate
   transitive paths.
2. For each candidate `(a_id, c_id, b_label, r1, r2)`:
   a. Construct a relation label: `"{r1} (via {b_label})"`.
   b. Construct a context string recording the transitive reasoning.
   c. Create a `KnowledgeEdge` with `basis = Inferred`.
   d. Add the edge to the graph via `add_edge_by_id(topic, a_id, c_id, edge)`.
3. If any edges were created, broadcast a graph update event.
4. Log an entry with `kind = "synthesis"`.

Synthesis is computationally cheap -- it operates entirely on graph
structure and consumes no AI tokens. The resulting edges are inferred
conjectures with moderate confidence, subject to future refinement or
refutation.

The phase MUST be wrapped in a `begin_thought()` / `end_thought()` pair.

### 4.3 Undetermined Connections

This phase investigates edges whose relation is the literal string `"?"`
-- connections that exist in the graph but whose nature has not been
established.

The implementation MUST:

1. Collect undetermined connections across all filtered topic graphs via
   `undetermined_connections(topic, 10)`.
2. Batch up to 8 connections into a single AI prompt.
3. Dispatch the prompt to the AI worker with `new_session = true` and
   `source = "rumination"`.
4. Log an entry with `kind = "undetermined"`.

The prompt MUST instruct the AI to, for each connection:

- Search external sources for how the concepts relate.
- Examine other connections in the graph for context.
- If a relationship can be determined: replace `"?"` with a proper
  relation name and set `basis` to `"inferred"` or `"observed"`.
- If the concepts genuinely do not relate: remove the `"?"` edge entirely.
- Add a citation if an external source was found.

### 4.4 Cross-Domain Pattern Transfer

**Precondition:** At least 2 filtered topic graphs MUST exist.

This phase compares structural patterns across different topic graphs.
When a relationship pattern in one domain mirrors a pattern in another,
the insight can strengthen both.

The implementation MUST:

1. Compare every pair of filtered topic graphs using
   `cross_domain_patterns(topic_a, topic_b, 1)`.
2. Select the best match (first found).
3. Dispatch a prompt instructing the AI to:
   a. Analyse whether insights from one domain can inform the other.
   b. If the transfer is valid, update the weaker edge with
      `evidence_type = "pattern_transfer"`.
   c. Consider whether a deeper underlying principle is revealed.
4. Log an entry with `kind = "pattern_transfer"` and topic
   `"{topic_a} <-> {topic_b}"`.

### 4.5 Active Refutation

**Guard:** `refutation_enabled` MUST be `true`.

Active refutation is the epistemic core of rumination. The implementation
selects beliefs that are important but uncertain and actively attempts to
disprove them with external evidence.

The implementation MUST:

1. Collect refutation candidates from all filtered topic graphs via
   `refutation_candidates(topic, 3)`.
2. Sort candidates by priority: `importance * (1 - confidence)`,
   descending. This prioritises beliefs that are both important to the
   graph structure and insufficiently tested.
3. Dispatch prompts for the top 2 candidates.
4. Log entries with `kind = "refutation"`.

Each refutation prompt MUST instruct the AI to perform three steps:

**Step 1 -- External search.** Use web search to find sources that
contradict or challenge the claim. Prioritise high-quality sources:
peer-reviewed papers, official reports, authoritative books. Fetch and
read the actual content. Save useful sources to `files/`.

**Step 2 -- Internal consistency check.** Look for inconsistencies with
other beliefs in the knowledge graph. Check whether the evidence trail is
one-sided (all confirmations, no challenges).

**Step 3 -- Honest evaluation.** Exactly one of three outcomes MUST be
recorded:

| Outcome | Evidence Type | Bayes Factor | Meaning |
|---------|--------------|--------------|---------|
| A -- Survived | `refutation_survived` | 2.5 | A specific external source could have disproved the belief but failed to. The belief is genuinely strengthened. |
| B -- Failed | `refutation_failed` | 0.1 | External evidence does disprove or seriously undermine the belief. The belief is sharply weakened. |
| C -- Inconclusive | `inconsequential_search` | 1.0 | No relevant evidence was found either way. The belief is unchanged. Absence of counter-evidence does NOT strengthen. |

The prompt MUST explicitly warn against using `refutation_survived` when
no specific external source was found. Self-agreement is confirmation
bias, not genuine refutation.

### 4.6 Contradiction Resolution

**Guard:** `contradiction_resolution` MUST be `true`.

When two edges between the same node pair assert incompatible
relationships, they form a contradiction. The implementation MUST:

1. Call `contradiction_pairs(topic)` for each filtered topic graph.
2. Dispatch a prompt for up to 1 contradiction per topic, with a maximum
   of 2 contradictions per cycle.
3. Log entries with `kind = "contradiction"`.

The prompt MUST present both beliefs with their confidence levels and
context, and instruct the AI to:

- Evaluate which belief is more likely correct based on evidence.
- Strengthen the winner with `"corroboration"` evidence.
- Weaken the loser with `"contradiction"` evidence.
- If both can coexist (apparent contradiction), add context explaining how.

### 4.7 Autonomous Initiative

**Guard:** `initiative_enabled` MUST be `true`.

This phase identifies the weakest area of the knowledge graph and
dispatches improvement work. The implementation MUST:

1. Compute `uncertainty_stats(topic)` for each filtered topic graph.
2. Select the topic with the highest ratio of uncertain edges to total
   edges (`uncertain_edge_count / edge_count`).
3. Dispatch a prompt instructing the AI to:
   a. Read the topic graph and identify gaps.
   b. Search the web for evidence supporting or refuting uncertain edges.
   c. Save useful sources to `files/` and add citations.
   d. Add new conjectures based on external evidence.
   e. Strengthen edges with supporting external sources
      (`"corroboration"`).
   f. Weaken edges with contradicting external sources
      (`"inconsistency"`).
   g. For edges based purely on AI inference, look for real sources.
4. Log an entry with `kind = "initiative"`.

The prompt MUST emphasise going beyond internal reasoning -- an idea
backed by external sources is stronger than one backed only by AI
inference.

### 4.8 Meta-Rumination

Meta-rumination is the ANT's capacity for self-modification. The ANT
reviews its own recent thinking patterns and evolves its methodology.

**Frequency guard:** Meta-rumination MUST only execute when the rumination
log contains at least 10 entries AND the entry count is divisible by 5.
This means it runs approximately every 5 cycles.

The implementation MUST:

1. Examine the 20 most recent rumination log entries.
2. Count the number of entries whose description contains
   `"inconsequential"` (indicating searches that found nothing).
3. Compute the inconsequential rate as a percentage.
4. Check whether `memory/thinking_process.md` exists.
5. Dispatch a prompt instructing the AI to:
   a. Review its refutation rigour -- are searches actually finding
      relevant evidence, or casting too wide a net?
   b. Evaluate belief selection strategy -- are the right beliefs being
      tested?
   c. Assess evidence evaluation honesty -- is confidence being inflated
      or deflated inappropriately?
   d. Record observations in the `meta-cognition` topic graph.
   e. If improvements are identified, update `thinking_process.md` with
      what changed, why, and the expected improvement.
   f. Treat every change to the thinking process as itself a conjecture,
      subject to future testing.
6. Log an entry with `kind = "meta"` and topic `"meta-cognition"`.

### 4.9 Stop Directive

Every rumination prompt -- whether automatic or manual -- MUST be
appended with the following stop directive:

> If you have questions that need human input (decisions, opinions,
> clarifications), write them to memory/questions.json -- the human will
> see them next time they come online. Format:
> `{"questions": [{"timestamp": "YYYY-MM-DD", "topic": "...", "question": "...", "context": "..."}]}`.
> Append to existing questions, don't overwrite.
>
> IMPORTANT: This is an autonomous rumination task. Complete the work
> above, update the graph files, then STOP. Do not ask follow-up
> questions. Do not ask what to do next. Do not wait for input. Output a
> brief summary of what you changed and stop.

This directive prevents the AI from entering an interactive loop during
autonomous operation.

### 4.10 CliRequest Fields

All rumination requests MUST be dispatched as `CliRequest` values with:

| Field         | Value                                              |
|---------------|----------------------------------------------------|
| `chat_id`     | `-1` (the `RUMINATION_CHAT_ID` constant)           |
| `new_session` | `true`                                              |
| `task_id`     | `0` (assigned by the worker on receipt)             |
| `source`      | `"rumination"`                                      |

---

## 5. Manual Rumination (`/ruminate` Command)

The `/ruminate` command triggers 4 focused, independent tasks dispatched
in parallel. Unlike the automatic cycle, manual rumination does not
require idle detection and does not follow the phased sequence. Each task
has a specific, achievable goal so the AI does not lose focus.

An implementation MUST dispatch these 4 tasks:

### 5.1 Refutation

Pick one important belief with moderate confidence (40--80%) and attempt
to refute it. The AI MUST:

1. State the belief clearly.
2. Formulate specific ways it could be wrong.
3. Search for evidence that would disprove it.
4. Record the outcome using the three-outcome model from Section 4.5.
5. Update the topic graph file.

### 5.2 Undetermined Connections

Find edges with relation `"?"` and resolve them. The AI MUST:

1. Pick one `"?"` connection.
2. Examine what other edges connect to those nodes.
3. Determine the actual relationship.
4. Replace the `"?"` edge with the determined relation, setting
   `basis = "inferred"`.
5. If the relationship cannot be determined, add a question to
   `memory/questions.json`.

### 5.3 Strengthen and Improve

Look for areas to improve across all topic graphs. The AI MUST:

1. Find nodes with few connections and identify missing relationships.
2. Set `beneficial_impact` on edges where relevant (positive for ideas
   that benefit people and the planet).
3. Look for edges that should exist based on available knowledge.
4. Add new conjectures with appropriate basis and confidence.

### 5.4 Citation Consolidation

Resolve unknown citation links and cross-reference with topic graphs. The
AI MUST:

**Step 1 -- Resolve unknown citation links:**

1. Find edges in the citations graph with relation `"?"` and orphaned
   citation nodes.
2. For each `"?"` edge, examine the citation's URL, title, and snippet.
3. If a URL is present:
   a. Check `files/` first for already-downloaded content (match by
      filename derived from URL or `cite_id`).
   b. If not cached, fetch the URL and save content to `files/` using a
      descriptive filename (e.g., `files/cite-a1b2c3d4.html`).
   c. Read the content to determine the core idea.
4. Replace the `"?"` relation with a description of the citation's subject.

**Step 2 -- Link citations to topic graph edges:**

1. For each citation, identify which topic graph edges it supports.
2. Check the citation's `cite_id` (format: `cite-<8hex>`).
3. If a topic graph edge is supported by the citation but lacks it in its
   citations list, add it with the structure:
   `{"cite_id": "<id>", "url": "...", "title": "...", "ref_type": "...", "quality": ...}`.
4. Do NOT fabricate citations -- only link citations that genuinely
   support the edge.

Each task MUST be dispatched with `new_session = true` and
`source = "rumination"`. Each prompt MUST end with the instruction:
complete the specific task, update the graph files, output a brief
summary, and STOP.

---

## 6. Citation Consolidation (`/citations` Command)

The `/citations` command dispatches a single, comprehensive citation
analysis task. It is more thorough than the citation sub-task in
`/ruminate` (Section 5.4) and includes verification, clustering, quality
upgrade, and cross-referencing.

An implementation MUST dispatch a single `CliRequest` with
`new_session = true` and `source = "rumination"`.

The prompt MUST instruct the AI to execute four steps:

### 6.1 Step 1 -- Verify and Analyse Each Citation Source

For each citation node in the citations graph:

1. If the node has a URL, fetch it. Check `files/` first for cached
   content.
2. If the fetch returns 404 or the page does not exist, the citation is
   broken. The implementation MUST remove it from the citations graph AND
   from any topic graph edges that reference it. Broken URLs are likely
   fabricated and MUST NOT be retained.
3. If the fetch succeeds, save the content to `files/`.
4. Read the actual content and extract the top 3 core ideas. Store as the
   node's summary: `"Core ideas: (1) ... (2) ... (3) ..."`.
5. Follow upstream references to find more authoritative sources
   (peer-reviewed papers, official reports). Add verified upstream
   references as new citation nodes.

### 6.2 Step 2 -- Find Citation Clusters and Core Sources

1. Compare core ideas across citations to identify which sources say
   similar things.
2. Add `"corroborates"` edges between citations that support the same
   ideas.
3. Add `"contradicts"` edges between citations that disagree.
4. Add `"cites"` edges when one source references another.
5. Identify core citations -- sources that many others reference or that
   originated key ideas. Tag these with `"core_source"` in their tags.
6. Core sources SHOULD have higher quality scores.

### 6.3 Step 3 -- Upgrade Low-Quality Citations

1. For edges with low-quality citations (blog, website, ai_inference),
   check if a better citation exists in the same family -- one connected
   by `"corroborates"` or `"cites"` edges that shares the same core ideas.
2. Replace with the higher-quality source, keeping the original as
   secondary.
3. Quality preference order: `peer_reviewed > official_report > book >
   news > blog > ai_inference`.

### 6.4 Step 4 -- Link Citations to Topic Graph Edges

1. Using core ideas extracted in Step 1, match citations to topic graph
   edges.
2. A citation supports an edge when its core ideas align with the claim.
3. Use `graph_add_citation` to attach citations to edges.
4. Prefer core citations over derivative ones.
5. For edges with only `ai_inference` citations, search for real sources.

---

## 7. Rumination Log

Every rumination activity -- whether automatic or manual -- MUST be
recorded in the rumination log.

### 7.1 Structure

The rumination log is a JSON file persisted at
`memory/rumination_log.json`. It contains:

```json
{
  "entries": [
    {
      "timestamp": "2026-03-30T14:30:00Z",
      "kind": "refutation",
      "topic": "philosophy",
      "description": "Challenging: 'Popper' refutes 'inductivism' (72%)",
      "edges_created": 0,
      "edges_updated": 1
    }
  ]
}
```

### 7.2 RuminationEntry Fields

| Field           | Type     | Description                                                   |
|-----------------|----------|---------------------------------------------------------------|
| `timestamp`     | `String` | ISO 8601 datetime of the entry.                               |
| `kind`          | `String` | Phase identifier (see table below).                           |
| `topic`         | `String` | Which topic graph was involved, or `"multiple"` / `"meta-cognition"`. |
| `description`   | `String` | Human-readable description of the activity.                   |
| `edges_created` | `u32`    | Number of edges created (if known at log time).               |
| `edges_updated` | `u32`    | Number of edges updated (if known at log time).               |

### 7.3 Kind Values

| Kind                | Phase                          |
|---------------------|--------------------------------|
| `"synthesis"`       | Synthesis (Section 4.2)        |
| `"undetermined"`    | Undetermined connections (4.3) |
| `"pattern_transfer"`| Cross-domain transfer (4.4)    |
| `"competition"`     | Darwinian competition (4.4*)   |
| `"refutation"`      | Active refutation (4.5)        |
| `"contradiction"`   | Contradiction resolution (4.6) |
| `"initiative"`      | Autonomous initiative (4.7)    |
| `"citations"`       | Citation consolidation         |
| `"meta"`            | Meta-rumination (4.8)          |

*Competition is a sub-mode of cross-domain analysis where competing
hypotheses about the same node pair are evaluated.

### 7.4 Size Limit

The log MUST retain at most 200 entries. When the limit is exceeded, the
implementation MUST drain the oldest entries to bring the count back to
200. This draining MUST occur on each append.

### 7.5 Persistence

The log MUST be saved to disk after every append. The implementation
MUST use atomic writes: write to a `.json.tmp` file, then rename to the
final path. This prevents corruption from interrupted writes.

---

## 8. Questions Queue

When rumination encounters knowledge gaps that cannot be resolved
autonomously, the AI MUST write questions to `memory/questions.json` for
human review.

### 8.1 Format

```json
{
  "questions": [
    {
      "timestamp": "2026-03-30",
      "topic": "philosophy",
      "question": "Is Lakatos's 'sophisticated falsificationism' a refinement or a replacement of Popper?",
      "context": "Found conflicting interpretations during refutation of 'Popper supersedes Lakatos' edge."
    }
  ]
}
```

### 8.2 Rules

- The AI MUST append to existing questions, never overwrite the file.
- Questions SHOULD be surfaced to the human via the `/questions` command.
- Questions are advisory: the human MAY answer, ignore, or delete them.

---

## 9. Self-Modification

The file `memory/thinking_process.md` is a first-class conjecture that
describes the ANT's current reasoning methodology. It is not a fixed
configuration -- the ANT MAY modify it during meta-rumination
(Section 4.8).

### 9.1 Principles

1. The thinking process file is itself subject to Popperian scrutiny.
   Every change MUST be treated as a conjecture, not as a proven
   improvement.
2. During meta-rumination, the ANT reviews its recent reasoning patterns,
   identifies systematic weaknesses (e.g., high inconsequential search
   rate), and updates its methodology.
3. Changes MUST be documented within the file: what was changed, why, and
   what improvement is expected.
4. Meta-rumination records are stored in the `meta-cognition` topic graph.
5. If the file does not yet exist, meta-rumination SHOULD create it with
   an initial methodology.

### 9.2 Scope of Modification

The ANT MAY modify:

- Refutation strategy (which beliefs to test, how to search).
- Evidence weighting heuristics.
- Topic prioritisation.
- Search depth and breadth.

The ANT MUST NOT modify:

- The Thurisaz engine parameters (those are structural, not conjectural).
- The rumination phase ordering (that is defined by this specification).
- The stop directive or CliRequest constants.

---

## 10. Consolidation

Consolidation is a maintenance operation that runs on schedule
independent of rumination. It also runs once at the end of every
automatic rumination cycle (Section 4). Consolidation does not consume AI
tokens -- it is purely structural graph maintenance.

### 10.1 File Housekeeping

Before graph consolidation, the implementation MUST perform file
housekeeping:

1. **Move stray graphs.** JSON files in `memory/` that contain `"nodes"`
   and `"edges"` keys (i.e., are knowledge graphs) MUST be moved to
   `memory/graphs/`, unless they are known root files. Known root files
   that MUST NOT be moved:
   - `knowledge.json`, `knowledge-archive.json`
   - `episodes.json`, `embeddings.json`
   - `reputation.json`, `questions.json`
   - `rumination_log.json`, `rumination.md`
   - `thinking_process.md`
   - Files starting with a digit or `-` (user memory files)
2. **Clean temporary files.** Files ending with `.corrupted`, `.json.tmp`,
   `.cbor.tmp`, or `.tmp` MUST be deleted, recursively through all
   subdirectories of `memory/`.

### 10.2 Meta-Graph Consolidation

1. Extract misplaced nodes from the meta-graph (nodes that are not topic
   references) and relocate them to the `"uncategorised"` graph.
2. Run `consolidate("meta")` -- deduplicate nodes, merge parallel edges,
   collapse chains.
3. Run `backfill_thurisaz("meta")` -- ensure all edges have valid
   Thurisaz confidence scores.
4. Run `link_orphans("meta")` -- connect orphaned nodes.
5. Detect and log contradictions.
6. Detect and log clusters of 3 or more nodes.
7. Broadcast a graph update event if any changes were made.

### 10.3 Topic Graph Consolidation

For each topic graph (excluding `"meta"`):

1. Run `consolidate(name)` -- deduplicate nodes, merge parallel edges.
2. Run `backfill_thurisaz(name)`.
3. Run `link_orphans(name)`.
4. Detect and log contradictions.
5. Broadcast a graph update event.

### 10.4 Citation Integrity Check

After graph consolidation, the implementation MUST check citation
integrity:

1. Load the citations graph visualisation.
2. Count core sources (nodes tagged `"core_source"`).
3. Count family connections (`"corroborates"`, `"cites"`, `"contradicts"`
   edges).
4. Log the counts for observability.

### 10.5 Cross-Linking

Cross-linking runs on a separate schedule (default: every 6 hours) and
is independent of rumination. It finds entities that appear in multiple
topic graphs and creates `"shares_entities"` edges in the meta-graph.

The implementation MUST:

1. Collect all entity labels per topic graph (excluding `"meta"`).
2. For each pair of topic graphs, find labels that appear in both
   (case-insensitive comparison).
3. For each pair with shared entities:
   a. Ensure both topics have nodes in the meta-graph.
   b. If no `"shares_entities"` edge exists between them, create one with
      context listing the shared entities.
4. Rebuild the meta-graph index and save.
5. Broadcast a graph update event if any edges were added.

---

## 11. Conformance

### 11.1 REQUIRED

An implementation claiming conformance to ANTHILL-RUMINATION MUST:

- R1. Respect the `enabled` flag: automatic rumination MUST NOT occur
  when disabled.
- R2. Enforce `min_idle_secs` and `interval_secs` timing constraints.
- R3. Detect idle state by checking whether the TaskMap is empty.
- R4. Execute automatic rumination phases in the order specified in
  Section 4 (4.1 through 4.8).
- R5. Append the stop directive (Section 4.9) to every rumination prompt.
- R6. Set `source = "rumination"` and `chat_id = -1` on all automatic
  rumination requests.
- R7. Set `new_session = true` on all rumination requests.
- R8. Maintain a rumination log with at most 200 entries, persisted via
  atomic writes to `memory/rumination_log.json`.
- R9. Run consolidation (Section 10) after each automatic rumination
  cycle and on its own schedule.
- R10. Create a Git commit of `memory/` after each automatic rumination
  cycle completes.
- R11. Dispatch exactly 4 parallel tasks for the `/ruminate` command
  as specified in Section 5.
- R12. Dispatch exactly 1 task for the `/citations` command as specified
  in Section 6.
- R13. Enforce the three-outcome refutation model (survived / failed /
  inconclusive) with the specified Bayes factors.
- R14. Exclude the `"meta"` graph from topic iteration in all rumination
  phases.
- R15. Remove broken citation URLs (404, unreachable) from both the
  citations graph and any topic graph edges that reference them.

### 11.2 RECOMMENDED

- S1. Implementations SHOULD limit refutation prompts to 2 per automatic
  cycle to control AI token consumption.
- S2. Implementations SHOULD limit contradiction resolution to 2
  prompts per cycle.
- S3. Implementations SHOULD batch undetermined connections (up to 8)
  into a single prompt.
- S4. Implementations SHOULD run meta-rumination approximately every 5
  cycles, not on every cycle.
- S5. Implementations SHOULD cache fetched citation content in `files/`
  to avoid redundant network requests.

### 11.3 OPTIONAL

- O1. Implementations MAY support the competition sub-phase (Darwinian
  competition between hypotheses about the same node pair).
- O2. Implementations MAY support cross-domain pattern transfer.
- O3. Implementations MAY extend the rumination log with additional
  fields beyond those specified in Section 7.

---

## 12. References

- Popper, K. R. *The Logic of Scientific Discovery*. Routledge, 1959.
- Popper, K. R. *Conjectures and Refutations*. Routledge, 1963.
- RFC 2119. Bradner, S. "Key words for use in RFCs to Indicate
  Requirement Levels." IETF, 1997.
- ANTHILL-KNOWLEDGE -- Knowledge graph schema and persistence.
- ANTHILL-THURISAZ -- Bayesian confidence engine, evidence diversity,
  fading foundations.
- ANTHILL-WORKER -- AI subprocess lifecycle and multi-backend dispatch.
- ANTHILL-COLONY -- Colony supervisor and rumination scheduling.
