# ANTHILL-MEMORY: Thurisaz Epistemic Knowledge Store

**Version:** 0.4.0
**Date:** 2026-03-22
**Status:** Draft
**Depends on:** ANTHILL-INTRO, ANTHILL-COLONY

---

## 1. Introduction

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT", "SHOULD", "SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this document are to be interpreted as described in RFC 2119.

This specification defines how ANTS store, retrieve, and maintain knowledge. Anthill uses a **Thurisaz-compliant epistemic knowledge store** where all relationships are conjectures with Bayesian confidence, combined with **episodic memory** for narrative context and **per-user memory** for individual preferences.

The knowledge store implements Popperian epistemology with structural anti-confirmation-bias enforcement, Darwinian competition between hypotheses, fading foundations, reputation-weighted evidence, and beneficial impact scoring.

### 1.1 Terminology

| Term | Definition |
|------|-----------|
| **Knowledge store** | The validated API boundary through which all graph mutations pass |
| **Knowledge graph** | A directed graph of entities (nodes) and conjectural relationships (edges) |
| **Conjecture** | An edge in the graph — a relationship believed to be true with some confidence |
| **Confidence** | A probability (0.0-1.0) derived from log-odds, representing strength of a conjecture |
| **Log-odds** | Internal representation: ln(p/(1-p)). Used for numerical stability in sequential Bayesian updates |
| **Bayes factor** | The likelihood ratio of evidence under H1 vs H0. BF>1 strengthens, BF<1 weakens |
| **Refutation** | Evidence that contradicts a conjecture, reducing its confidence |
| **Evidence type** | One of 12 predefined categories of evidence, each with a base Bayes factor |
| **Basis** | How a conjecture was formed: observed, told, inferred, or assumed |
| **Decay category** | Controls how quickly a belief fades without fresh evidence |
| **Beneficial impact** | Fitness modifier: positive values benefit people and planet |
| **Corroboration strength** | How strongly an edge is supported by its network neighbourhood |
| **Competition group** | Identifier linking competing hypotheses |
| **Importance** | How central a relationship is to the ANT's work (0.0-1.0) |
| **Episode** | A timestamped summary of a conversation or event |
| **Consolidation** | Structural maintenance: deduplication, merging, chain collapsing |
| **Rumination** | Autonomous thinking: refutation, synthesis, competition, pattern transfer |

### 1.2 Philosophy: Popperian Epistemology

All knowledge in the graph is **conjectural**. There are no facts — only conjectures with varying degrees of confidence. This follows Karl Popper's epistemology:

1. **Knowledge grows through conjecture and refutation**, not through accumulation of confirmed facts.
2. **A conjecture gains strength by surviving refutation**, not by being confirmed. An edge tested 50 times and never contradicted is stronger than one confirmed 50 times but never tested against alternatives.
3. **Nothing is certain.** Confidence approaches but never reaches 1.0.
4. **Contradictions are valuable.** They reveal where the graph's model of reality is wrong.
5. **Refuted conjectures are not deleted.** They are archived as a record of what was tried.
6. **Absence of evidence is NOT evidence of absence.** Searching for counter-evidence and finding nothing (InconsequentialSearch, BF=1.0) does not strengthen a belief. Only active, failed refutation (RefutationSurvived, BF=2.5) strengthens a claim.
7. **Diversity of refutation is strength.** An idea that survived 3 different kinds of challenges is stronger than one that "survived" 10 identical corroborations. This is enforced by the evidence diversity ceiling.
8. **The system questions itself.** Meta-rumination reviews the thinking process; `thinking_process.md` is a conjecture the ANT can modify.
9. **Beneficial ideas get an evolutionary advantage.** Not censorship, but fitness bias — ideas good for people and planet are favoured in the selection landscape.

---

## 2. Knowledge Store Architecture

### 2.1 Trait Boundary

All graph access MUST go through the `KnowledgeStore` trait. Implementations MUST:

- Validate all write inputs before applying mutations
- Apply Bayesian updates through the Thurisaz engine
- Detect and warn about confirmation bias patterns
- Auto-commit every mutation to git
- Support multiple named graphs (meta + topic graphs)

The AI interacts through MCP tools that call trait methods. Direct file editing MUST NOT be supported.

### 2.2 Storage Backend

Implementations MUST use CBOR (Concise Binary Object Representation) for persistence:

- `memory/knowledge.cbor` — the meta-graph
- `memory/graphs/<topic>.cbor` — topic-specific graphs

CBOR is approximately 46% smaller than equivalent JSON. Implementations SHOULD use [ciborium](https://crates.io/crates/ciborium) for serde-compatible serialisation.

Implementations MUST support reading legacy JSON files for backward compatibility during migration. New writes MUST go to CBOR.

### 2.3 Atomic Writes

All file writes MUST use the atomic pattern: write to a temporary file, fsync, then rename. This prevents corruption on power loss.

### 2.4 Git Auto-Commit

Every `save_graph()` call MUST:
1. Stage the `memory/` directory
2. Check for staged changes
3. If changes exist, commit with a descriptive message (e.g. "knowledge: update <graph_name>")

The git history serves as the ANT's **thinking journal**.

---

## 3. Knowledge Graph Structure

### 3.1 Nodes

A node represents an entity. Implementations MUST support these node kinds:

| Kind | Description | Examples |
|------|-------------|----------|
| `person` | A human or named agent | "Roy", "Alice" |
| `project` | A body of work | "Anthill", "R2" |
| `server` | A machine or infrastructure | "Alfred", "AWS us-east-1" |
| `tool` | Software, language, or framework | "Rust", "Claude Code" |
| `concept` | An idea or principle | "Popperian epistemology" |
| `decision` | A choice that was made | "Use petgraph over SurrealDB" |
| `event` | Something that happened | "Production deploy 2026-03-15" |
| `fact` | A standalone assertion | "Anthill compiles on FreeBSD" |
| `theory` | A theoretical framework | "Bayesian epistemology" |
| `mechanism` | A process or mechanism | "Fading foundations" |
| `principle` | A guiding principle | "Absence of evidence != evidence of absence" |
| `constraint` | A limitation or constraint | "256-byte event limit" |
| `problem` | A problem or challenge | "Confirmation bias in AI" |
| `claim` | A specific claim being evaluated | "CBOR is 46% smaller than JSON" |
| `open_question` | An unresolved question | "Does this scale to 10K nodes?" |
| `implementation` | A code-level implementation | "CborGitBackend" |
| `entity` | A generic entity | Catch-all |
| `spec` | A specification | "ANTHILL-MEMORY" |
| `repo` | A code repository | "reality2-ai/anthill" |
| `platform` | A platform or OS | "Linux", "FreeBSD" |
| `framework` | A framework | "Reality2" |

Node schema:

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `label` | string | REQUIRED | Human-readable identifier (max 200 chars) |
| `kind` | NodeKind | REQUIRED | One of the kinds above |
| `summary` | string | RECOMMENDED | One-line description |
| `created` | date string | RECOMMENDED | When this node was created (YYYY-MM-DD) |
| `updated` | date string | RECOMMENDED | When last modified |
| `tags` | string[] | OPTIONAL | Searchable keywords |

### 3.2 Edges (Conjectures)

An edge represents a conjectural relationship between two nodes. Every edge carries full epistemic metadata.

Edge schema:

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `relation` | string | REQUIRED | Relationship type (e.g. "works_on", "deployed_on") |
| `context` | string | "" | Brief description of the relationship |
| `since` | date string | "" | When this relationship began |
| `confidence` | f64 | basis-dependent | Current confidence (0.0-1.0), derived from log_odds |
| `log_odds` | f64 | from confidence | Log-odds representation for Bayesian updates |
| `tests` | u32 | 0 | How many times this conjecture was tested |
| `survived` | u32 | 0 | How many tests it survived |
| `basis` | Basis | "assumed" | How the conjecture was formed |
| `last_tested` | date string | "" | When last tested or reinforced |
| `importance` | f64 | 0.5 | How central this is (0.0-1.0) |
| `references` | u32 | 0 | How many times referenced in conversation |
| `valid_from` | date string | "" | When this relationship became true |
| `valid_until` | date string | "" | When this relationship ceased to be true (empty = still valid) |
| `view` | EdgeView | Entity | MAGMA-inspired perspective: Semantic, Temporal, Causal, or Entity |
| `source` | string | "" | Provenance: how this conjecture was formed |
| `source_id` | string | "" | Links to the reputation registry |
| `refutation_log` | RefutationEntry[] | [] | Legacy refutation audit trail |
| `evidence_log` | Evidence[] | [] | Typed evidence trail with Bayes factors |
| `justificatory_chain` | JustificationStep[] | [] | Provenance chain: "why do I believe this?" |
| `decay_category` | DecayCategory | from basis | Controls how quickly this belief fades |
| `beneficial_impact` | f64 | 0.0 | Fitness modifier (-1.0 to 1.0). Positive = good for people/planet |
| `corroboration_strength` | f64 | 0.0 | Network support from neighbouring edges |
| `competition_group` | string | "" | Links competing hypotheses |

### 3.3 Basis and Initial Confidence

The **basis** determines starting confidence when a conjecture is first formed:

| Basis | Initial Confidence | Decay Category | Description |
|-------|-------------------|----------------|-------------|
| `observed` | 0.7 | Observation (7-day half-life) | The AI directly observed evidence |
| `told` | 0.6 | Fact (30-day half-life) | A user stated this as fact |
| `inferred` | 0.4 | Inference (3-day half-life) | Derived from other knowledge |
| `assumed` | 0.3 | Assumption (1-day half-life) | Guessed without evidence |

### 3.4 Edge Views (MAGMA-inspired)

Edges carry a single perspective classification for richer graph queries:

| View | Description |
|------|-------------|
| `Semantic` | Semantic relationship (e.g. "ownership", "dependency", "preference") |
| `Temporal` | Temporal nature (e.g. "ongoing", "completed", "planned") |
| `Causal` | Causal role (e.g. "cause", "effect", "prerequisite") |
| `Entity` | Default — entity-level relationship |

Edge views enable queries such as "show all causal relationships" or "show all temporal relationships" without requiring the AI to re-classify edges at query time.

### 3.5 Temporal Validity (Zep-inspired)

Edges with `valid_from` and `valid_until` fields support time-scoped knowledge:

- **Current knowledge**: edges where `valid_until` is empty or in the future.
- **Historical knowledge**: edges where `valid_until` is in the past.
- **Scheduled knowledge**: edges where `valid_from` is in the future.

The prompt renderer SHOULD prefer current knowledge over historical knowledge. Historical edges MAY be included when the user asks about past states.

### 3.6 Provenance Tracking

The `source` field on edges records how a conjecture was formed, enabling "why do I believe this?" tracing:

- `"conversation"` — directly stated or observed in a conversation
- `"analysis"` — produced by `/analyse` or `/reflect`
- `"inference"` — derived from other graph relationships
- `"import"` — imported from external data
- `"consolidation"` — created by the consolidation process (e.g. merged edges)
- `"rumination"` — created during autonomous rumination

---

## 4. Bayesian Confidence Dynamics (Thurisaz Engine)

### 4.1 Log-Odds Representation

Confidence is stored as log-odds for numerical stability:

```
log_odds = ln(p / (1 - p))
probability = 1 / (1 + e^(-log_odds))
```

Sequential updates are additive in log-odds space:

```
log_odds' = log_odds + ln(BF_adjusted)
```

Log-odds MUST be clamped to [-6.9, 6.9], corresponding to probability [0.001, 0.999].

### 4.2 Evidence Types and Bayes Factors

Each evidence type has a predefined base Bayes factor. These are then reputation-adjusted.

| Evidence Type | Base BF | Category | Description |
|---|---|---|---|
| `refutation_survived` | 2.5 | Strengthening | Actively tried to disprove, claim held |
| `refutation_failed` | 0.1 | Weakening | Actively tried to disprove, claim failed |
| `competition_won` | 2.0 | Strengthening | Won head-to-head against rival hypothesis |
| `corroboration` | 2.0 | Strengthening | Supporting evidence in another source |
| `pattern_transfer` | 1.8 | Strengthening | Cross-domain insight strengthens this |
| `human_attestation` | 1.5 | Strengthening | User confirmed or corrected |
| `consistency` | 1.5 | Strengthening | Consistent with existing knowledge graph |
| `synthesis` | 1.2 | Strengthening | Transitive inference from two strong edges |
| `inconsequential_search` | 1.0 | **Neutral** | Searched but found nothing relevant |
| `inconsistency` | 0.4 | Weakening | Inconsistent with existing knowledge graph |
| `competition_lost` | 0.3 | Weakening | Lost head-to-head against rival |
| `contradiction` | 0.3 | Weakening | Contradicting evidence found |

**Key principle:** `InconsequentialSearch` (BF=1.0) produces NO CHANGE in confidence. Absence of evidence is NOT evidence of absence. Only `RefutationSurvived` — an active, genuine attempt to disprove that fails — strengthens a claim. This is a structural enforcement, not a prompting guideline.

### 4.3 Reputation-Weighted Evidence

Source reliability modulates evidence strength via the Thurisaz formula (TH-WEAVE 7.1):

```
w(r) = 0.5 + 0.5 * reputation
BF_adjusted = BF_base ^ w(r)
```

Where:
- r=0.0 (untrusted): BF is square-rooted (dampened)
- r=0.5 (neutral): BF^0.75
- r=1.0 (fully trusted): full BF

Reputation can only attenuate, never amplify beyond the base BF.

### 4.4 Evidence Log

Each edge maintains a full audit trail of evidence:

| Field | Type | Description |
|---|---|---|
| `date` | string | When this evidence was observed |
| `evidence_type` | EvidenceType | What type of evidence |
| `test` | string | What was tested or observed |
| `detail` | string | The evidence itself |
| `source_id` | string | Source identifier (links to reputation registry) |
| `source_reputation` | f64 | Source reputation at time of evidence (for audit) |
| `bayes_factor` | f64 | The effective Bayes factor applied |
| `log_odds_before` | f64 | Log-odds before this evidence |
| `log_odds_after` | f64 | Log-odds after this evidence |

### 4.5 Justificatory Chain

Each edge maintains a provenance chain — a sequence of steps answering "why do I believe this?":

| Field | Type | Description |
|---|---|---|
| `step` | u32 | Step number in the chain |
| `process` | string | What process produced this evidence |
| `confidence` | f64 | Confidence at this point in the chain |
| `source` | string | Source identifier |

---

## 5. Anti-Confirmation Bias Enforcement

These mechanisms are **structural** — enforced in the mathematics, not just through prompting. The AI's training pushes it toward confirmation; these limits push back.

### 5.1 Evidence Diversity Ceiling

An edge's confidence MUST be capped based on the number of distinct evidence types in its evidence log:

| Distinct evidence types | Maximum confidence |
|---|---|
| 0-1 | 0.70 (70%) |
| 2 | 0.85 (85%) |
| 3 | 0.92 (92%) |
| 4+ | 0.99 (99%) |

This means an idea supported only by repeated corroborations — even hundreds of them — can NEVER exceed 70% confidence. To reach high confidence, the idea must survive genuinely different kinds of scrutiny: refutation, competition, pattern transfer, human attestation, etc.

This ceiling MUST be applied after every evidence update.

### 5.2 Consecutive-Confirmation Dampening

If the last 5 or more evidence entries in the log all have BF > 1.0 (positive), and the current update is also positive, the update MUST be dampened. The implementation SHOULD pull the confidence back toward the pre-update value.

This enforces the principle that real knowledge encounters friction. If an idea is being confirmed repeatedly without any challenge, something is probably wrong — the system is likely confirming rather than testing.

### 5.3 Confirmation Bias Detection

After applying evidence, the system SHOULD inspect the evidence trail and generate a warning if:

1. There are 5+ consecutive positive updates with zero negative updates
2. The positive rate exceeds 85% with evidence type diversity of 2 or fewer, across 5+ entries

Warnings MUST be returned to the caller as part of the `EdgeUpdate` result, so the AI receives them through MCP and can adjust its behaviour.

---

## 6. Fading Foundations

### 6.1 Decay Formula

Beliefs decay toward uncertainty (p=0.5, log-odds=0) over time:

```
log_odds(t) = log_odds(t_last) * 2^(-elapsed / half_life)
```

This resolves Agrippa's trilemma: you don't need an absolute foundation if foundations fade. Epistemic chains converge without requiring certainty at any point (Peijnenburg & Atkinson, 2017).

### 6.2 Decay Categories

| Category | Half-life | Description |
|---|---|---|
| Fact | 30 days | Stable facts |
| Decision | 14 days | Choices that were made |
| Observation | 7 days | Things directly observed |
| Inference | 3 days | Derived from other knowledge |
| Assumption | 1 day | Guesses without evidence |
| Other | 30 days | Unknown category — treated as Fact |

The decay category is inferred from the edge's basis at creation time.

### 6.3 Decay Trigger

Confidence decay MUST be evaluated on a **time-based trigger** (24 hours idle), not just on a request-count basis. If the ANT has been idle for 24 hours or more, decay MUST be applied to all conjectures on the next interaction.

### 6.4 Confidence Bounds

- Confidence MUST be clamped to [0.01, 0.99]. No conjecture reaches certainty.
- Combined confidence from merging parallel edges is capped at 0.95.

---

## 7. Darwinian Competition

### 7.1 Beneficial Impact

Each edge carries a `beneficial_impact` score (-1.0 to 1.0):

- **Positive values**: the idea benefits people and the planet
- **Zero** (default): neutral
- **Negative values**: the idea is potentially harmful

This score acts as a fitness modifier in relevance scoring:

```
fitness = 1.0 + 0.2 * beneficial_impact   // range 0.8-1.2
```

Beneficial ideas get an evolutionary advantage in what appears in the prompt and what survives competition — not censorship, but a bias toward the constructive.

### 7.2 Competition Groups

Edges with the same `competition_group` string are competing hypotheses. The rumination engine:

1. Detects edges that could be competitors (multiple edges from the same node explaining the same phenomenon)
2. Groups them by assigning matching `competition_group` values
3. During rumination, pits them head-to-head: the AI evaluates which hypothesis better fits the evidence
4. Awards `CompetitionWon` (BF=2.0) to the winner and `CompetitionLost` (BF=0.3) to the loser

### 7.3 Corroboration Strength

The `corroboration_strength` field measures how strongly an edge is supported by its network neighbourhood — the confidence of other edges that connect to the same nodes.

This is computed during consolidation and rumination. Higher values indicate the edge is part of a well-connected cluster of knowledge.

The relevance score includes a network bonus:

```
network_bonus = 1.0 + 0.1 * corroboration_strength
relevance = confidence * importance * fitness * network_bonus
```

### 7.4 Relevance Score

The **relevance score** combines all factors:

```
relevance = confidence * importance * fitness * network_bonus
```

Where:
- `importance` grows logarithmically with reference count: `0.5 + 0.5 * (1 - 1/(1 + references/10))`
- `fitness = 1.0 + 0.2 * beneficial_impact`
- `network_bonus = 1.0 + 0.1 * corroboration_strength`

Used to prioritise which edges appear in the prompt and in what order.

---

## 8. Query API

### 8.1 Query Types

Implementations MUST support these query types:

| Query | Input | Behaviour |
|-------|-------|-----------|
| **About** | label, depth | BFS traversal from the named node, depth hops. Returns subgraph with confidence-weighted relevance. |
| **Path** | from, to | Shortest path(s) between two nodes. Cumulative confidence is the product along the path. |
| **ByKind** | NodeKind | All nodes of a given type, sorted by average edge confidence. |
| **Uncertain** | threshold | All edges below the given confidence, above MIN_PROMPT_CONFIDENCE. |
| **Justification** | from, to, relation | The justificatory chain for a specific edge — "why do I believe this?" |
| **Orphans** | graph | Nodes with only '?' connections — candidates for investigation. |

### 8.2 Traversal (About Query)

The About query performs a breadth-first search from a starting node:

1. Find the node by exact label match (case-insensitive).
2. If not found, try substring match (fuzzy).
3. BFS outward to the requested depth, following both incoming and outgoing edges.
4. Score each node: root = 1.0, others = average relevance_score of connecting edges.
5. Return nodes sorted by score descending, plus all edges in the subgraph.

### 8.3 Path Query

The Path query finds how two entities are connected:

1. BFS from source, tracking cumulative confidence (product of edge confidences).
2. Search both outgoing and incoming edges (undirected traversal).
3. Maximum path length: 10 hops.
4. Return up to `max_paths` results, sorted by cumulative confidence descending.

A path confidence of 0.61 (= 0.85 * 0.72) means: "if both edges are correct, the connection holds with 61% confidence."

### 8.4 Prompt Integration

For the AI system prompt:

- **Small graphs** (<=30 nodes): render the full graph.
- **Large graphs** (>30 nodes): extract entity names from the user's message, run About queries for each, render the combined traversal results.
- **Semantic retrieval**: when Ollama embeddings are available, nearest nodes by cosine similarity seed the traversal.
- **Fallback**: if no entity labels match, use keyword-based subgraph extraction (inverted index + 1-hop expansion).
- **Cap**: rendered graph context MUST NOT exceed 4096 characters.

Edges below `MIN_PROMPT_CONFIDENCE` (0.15) MUST NOT appear in the prompt.

High-confidence edges (>=0.8) are rendered without qualifiers. Lower-confidence edges include a visual confidence indicator.

---

## 9. Episodic Memory

### 9.1 Purpose

The knowledge graph captures structured facts. Episodic memory captures **narrative** — what happened, in what order, with what outcome. Humans remember stories, not databases.

### 9.2 Episode Schema

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `date` | date string | REQUIRED | When this episode occurred |
| `participants` | string[] | OPTIONAL | Who was involved |
| `summary` | string | REQUIRED | 2-3 sentence narrative |
| `outcomes` | string[] | OPTIONAL | Key decisions or results |
| `entities` | string[] | OPTIONAL | Labels of knowledge graph nodes referenced in this episode |
| `tags` | string[] | OPTIONAL | Searchable keywords |

### 9.3 Entity Linking

The `entities` field links episodes to knowledge graph nodes by label. This enables:

- Retrieving all episodes that mention a given entity (bidirectional cross-reference).
- Providing narrative context when an entity is queried from the graph.
- Identifying entities that frequently co-occur in episodes (relationship discovery).

The AI SHOULD populate `entities` with the labels of all knowledge graph nodes mentioned or relevant to the episode.

### 9.4 When to Write Episodes

The AI SHOULD write an episode after:
- A significant conversation (not trivial questions)
- A decision is made
- A problem is solved
- A deployment or event occurs

### 9.5 Retrieval

Episodes are retrieved by keyword search against summary, outcomes, and tags, weighted by **recency** (exponential decay with 30-day half-life). Recent episodes score higher than old ones with identical keyword matches. The most relevant 5 episodes are included in the prompt.

---

## 10. Graph Consolidation

### 10.1 Purpose

Over time, the graph accumulates duplicate nodes, redundant edges, and low-value intermediary nodes. Consolidation is structural maintenance — not summarisation.

### 10.2 Operations

| Operation | Trigger | Behaviour |
|-----------|---------|-----------|
| **Node dedup** | Labels match (case-insensitive, substring, or Levenshtein <=15% edit distance for labels >=6 chars) + same kind | Merge: keep longer summary, union tags, move all edges |
| **Edge merge** | Same source, target, and relation | Combine: confidence = max(c1, c2), cap 0.95. Sum tests/survived/references. Concatenate contexts. |
| **Chain collapse** | A->B->C where B is a Fact with degree 2 and low importance | Collapse to A->C: confidence = min(c1, c2), combined relation |
| **Contradiction detect** | Same node pair with high/low confidence divergence | Flag as warning for AI review |
| **Community detection** | During consolidation | GraphRAG-inspired connected component analysis identifies knowledge clusters |
| **Corroboration strength** | During consolidation and rumination | Compute network support from neighbouring edges |
| **Orphan linking** | Nodes with only '?' connections | Link to relevant hubs or flag for investigation |

### 10.3 Confidence Merging

When merging parallel edges (same source, target, and relation):

```
combined_confidence = max(c1, c2)   // capped at 0.95
combined_context = "ctx1; ctx2"     // preserve both sources
```

The MAX strategy is used because parallel edges typically come from the same source (the AI processing different conversations). Using probabilistic OR would incorrectly compound confidence for non-independent observations.

For chain collapsing (transitive inference):

```
chain_confidence = min(c1, c2)
```

A chain is only as strong as its weakest link.

### 10.4 Schedule

Consolidation runs:

- **Every 15 minutes** — automatic background maintenance (dedup, link orphans, backfill metadata)
- **Every 50 AI requests** — inline maintenance including orphan linking and backfill
- **After each rumination cycle** — full consolidation pass
- **On `/reflect` or `/reprocess-graphs`** — manual trigger

---

## 11. Rumination

### 11.1 Purpose

When the ANT is idle, it thinks autonomously. Rumination applies epistemic operations to the knowledge graph without human prompting.

### 11.2 Cycle

A full rumination cycle includes, in order:

1. **Corroboration strength computation** — recompute network support for all edges
2. **Synthesis** — find A->B->C paths where no A->C edge exists; create edges with Synthesis evidence (BF=1.2); no AI tokens required
3. **Undetermined connections** — investigate '?' edges; ask the AI to determine the relationship or write a question to `questions.json`
4. **Competition** — detect competing hypotheses; run head-to-head evaluations; award CompetitionWon/CompetitionLost evidence
5. **Cross-domain pattern transfer** — find structural similarities between topic graphs; award PatternTransfer evidence (BF=1.8)
6. **Active refutation** — select important but uncertain edges; actively try to disprove them; award RefutationSurvived (BF=2.5) or RefutationFailed (BF=0.1)
7. **Contradiction resolution** — find edge pairs where both cannot be true; send to AI for resolution
8. **Citation consolidation** — ensure every edge has proper source citations; build and maintain the citations graph; add `ai_inference` references for AI-reasoned edges (see §11.5)
9. **Autonomous initiative** — identify knowledge gaps; write questions to `questions.json`
10. **Meta-rumination** — review the thinking process; potentially modify `thinking_process.md`

After the cycle: consolidation, orphan linking, git commit.

### 11.5 Citation Consolidation

The citations graph is a **core graph** — automatically created and maintained during every rumination cycle. It tracks all sources used across topic graphs.

During citation consolidation:

1. All topic graph edges are scanned for their `citations` field
2. Each unique citation source gets a node in `memory/graphs/citations.cbor`
3. Unresolved `?` edges in the citations graph are investigated (URLs fetched and saved to `files/`)
4. Topic graph edges lacking citations receive them — web-sourced edges get URL citations, AI-inferred edges get `ref_type: ai_inference`
5. All updated graphs are written back

Citation consolidation runs automatically when more than one-third of edges lack citations, or when the citations graph does not yet exist. It can also be triggered manually via `/citations`.

#### Citation Schema

Each edge's `citations` field is an array of:

```json
{
  "cite_id": "cite-<8hex>",
  "url": "https://...",
  "title": "Source title",
  "author": "",
  "date": "",
  "accessed": "YYYY-MM-DD",
  "snippet": "Brief quote from source",
  "ref_type": "peer_reviewed|official_report|book|news|blog|website|personal|ant_knowledge|ai_inference",
  "quality": 0.0-1.0
}
```

Citation quality provides a confidence bonus: well-cited edges receive up to 15% boost to effective confidence.

### 11.3 Questions Queue

Rumination generates questions for the human when it encounters gaps it cannot fill autonomously. These are written to `memory/questions.json` and surfaced via the `/questions` command or the web dashboard.

### 11.4 Meta-Rumination

The ANT maintains `memory/thinking_process.md` — a document describing its evolved approach to reasoning. During meta-rumination:

1. The ANT reviews its recent rumination results
2. Evaluates whether its reasoning strategies were effective
3. Can modify `thinking_process.md` to evolve its methodology
4. The thinking process is itself a conjecture — subject to revision

This file is loaded into the system prompt (up to 2KB) for all subsequent interactions.

---

## 12. Embedding-Based Retrieval

Embedding-based semantic search is available as an additional retrieval strategy via Ollama's `nomic-embed-text` model.

### 12.1 How It Works

1. Each node's label+summary is encoded as a vector via `POST /api/embed` to the local Ollama instance.
2. The user's message is encoded as a vector.
3. Nearest nodes by cosine similarity seed the graph traversal.

This enables queries like "the project Roy is working on" to find "Anthill" even when the word doesn't appear in the message.

### 12.2 Fallback

If Ollama is not installed or `nomic-embed-text` is not available, retrieval falls back to keyword-based subgraph extraction (inverted index + 1-hop expansion) as described in 8.4. The system operates fully without embeddings; they are an enhancement, not a requirement.

### 12.3 Setup

```bash
ollama pull nomic-embed-text
```

No configuration is required in `ant.toml` — the system auto-detects the embedding model at startup.

---

## 13. Keyword Extraction

### 13.1 Language Agnostic

Keyword extraction MUST NOT depend on any specific natural language. The implementation:

1. Lowercase and split on non-alphanumeric characters.
2. Filter words shorter than 3 characters.
3. Filter high-frequency function words (a small multilingual list).
4. Generate suffix-stripped variants (remove last 1, 2, 3 characters) for fuzzy matching.

This approach works for English, French, German, Spanish, and other Latin-alphabet languages. CJK languages require character-level n-gram extraction (future work).

---

## 14. Security Considerations

1. **Knowledge graphs may contain sensitive information** (passwords, API keys mentioned in conversation). The AI SHOULD be instructed not to store credentials in the graph.
2. **Encrypted backups** (ANTHILL-COLONY 3.2) protect the knowledge graph at rest in git.
3. **Per-user memory files** contain user-specific information. These are accessible to the ANT but not to other ANTS in the colony.
4. **Validated writes** prevent the AI from writing malformed or invalid data to the knowledge store.
5. **CBOR binary format** is not human-readable, which provides a mild layer of obscurity (but MUST NOT be relied upon for security).
