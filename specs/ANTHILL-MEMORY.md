# ANTHILL-MEMORY: Knowledge Graph and Memory Architecture

**Version:** 0.3.0
**Date:** 2026-03-20
**Status:** Draft
**Depends on:** ANTHILL-INTRO, ANTHILL-COLONY

---

## 1. Introduction

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT", "SHOULD", "SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this document are to be interpreted as described in RFC 2119.

This specification defines how ANTS store, retrieve, and maintain knowledge. Anthill uses a **Popperian knowledge graph** where all relationships are conjectures with probabilistic confidence, combined with **episodic memory** for narrative context and **per-user memory** for individual preferences.

### 1.1 Terminology

| Term | Definition |
|------|-----------|
| **Knowledge graph** | A directed graph of entities (nodes) and conjectural relationships (edges) |
| **Conjecture** | An edge in the graph — a relationship believed to be true with some confidence |
| **Confidence** | A value 0.0–1.0 representing the strength of a conjecture |
| **Refutation** | Evidence that contradicts a conjecture, reducing its confidence |
| **Basis** | How a conjecture was formed: observed, told, inferred, or assumed |
| **Importance** | How central a relationship is to the ANT's work (0.0–1.0) |
| **Episode** | A timestamped summary of a conversation or event |
| **Consolidation** | Structural maintenance: deduplication, merging, chain collapsing |

### 1.2 Philosophy: Popperian Epistemology

All knowledge in the graph is **conjectural**. There are no facts — only conjectures with varying degrees of confidence. This follows Karl Popper's epistemology:

1. **Knowledge grows through conjecture and refutation**, not through accumulation of confirmed facts.
2. **A conjecture gains strength by surviving refutation**, not by being confirmed. An edge tested 50 times and never contradicted is stronger than one confirmed 50 times but never tested against alternatives.
3. **Nothing is certain.** Confidence approaches but never reaches 1.0.
4. **Contradictions are valuable.** They reveal where the graph's model of reality is wrong.
5. **Refuted conjectures are not deleted.** They are archived as a record of what was tried.

---

## 2. Knowledge Graph Structure

### 2.1 Nodes

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

Node schema:

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `label` | string | REQUIRED | Human-readable identifier |
| `kind` | NodeKind | REQUIRED | One of the kinds above |
| `summary` | string | RECOMMENDED | One-line description |
| `created` | date string | RECOMMENDED | When this node was created (YYYY-MM-DD) |
| `updated` | date string | RECOMMENDED | When last modified |
| `tags` | string[] | OPTIONAL | Searchable keywords |

### 2.2 Edges (Conjectures)

An edge represents a conjectural relationship between two nodes. Every edge carries Popperian metadata.

Edge schema:

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `relation` | string | REQUIRED | Relationship type (e.g. "works_on", "deployed_on") |
| `context` | string | "" | Brief description of the relationship |
| `since` | date string | "" | When this relationship began |
| `confidence` | f64 | basis-dependent | Current confidence (0.0–1.0) |
| `tests` | u32 | 0 | How many times this conjecture was tested |
| `survived` | u32 | 0 | How many tests it survived |
| `basis` | Basis | "assumed" | How the conjecture was formed |
| `last_tested` | date string | "" | When last tested or reinforced |
| `importance` | f64 | 0.5 | How central this is (0.0–1.0) |
| `references` | u32 | 0 | How many times referenced in conversation |
| `valid_from` | date string | "" | When this relationship became true (Zep-inspired temporal validity) |
| `valid_until` | date string | "" | When this relationship ceased to be true (empty = still valid) |
| `source` | string | "" | Provenance: how this conjecture was formed ("conversation", "analysis", "inference", etc.) |
| `views` | EdgeViews | {} | MAGMA-inspired multi-perspective metadata (see §2.4) |

### 2.3 Basis and Initial Confidence

The **basis** determines starting confidence when a conjecture is first formed:

| Basis | Initial Confidence | Description |
|-------|-------------------|-------------|
| `observed` | 0.7 | The AI directly observed evidence |
| `told` | 0.6 | A user stated this as fact |
| `inferred` | 0.4 | Derived from other knowledge |
| `assumed` | 0.3 | Guessed without evidence |

### 2.4 Edge Views (MAGMA-inspired)

Edges carry multi-perspective metadata for richer graph queries. Each view is OPTIONAL and may be populated by the AI or analysis pipelines.

| View | Type | Description |
|------|------|-------------|
| `semantic` | string | Semantic category of the relationship (e.g. "ownership", "dependency", "preference") |
| `temporal` | string | Temporal nature (e.g. "ongoing", "completed", "planned", "historical") |
| `causal` | string | Causal role (e.g. "cause", "effect", "correlation", "prerequisite") |
| `entity_class` | string | Classification of the relationship endpoints (e.g. "person-project", "tool-concept") |

Edge views enable queries such as "show all causal relationships" or "show all historical relationships" without requiring the AI to re-classify edges at query time.

### 2.5 Temporal Validity (Zep-inspired)

Edges with `valid_from` and `valid_until` fields support time-scoped knowledge:

- **Current knowledge**: edges where `valid_until` is empty or in the future.
- **Historical knowledge**: edges where `valid_until` is in the past.
- **Scheduled knowledge**: edges where `valid_from` is in the future.

The prompt renderer SHOULD prefer current knowledge over historical knowledge. Historical edges MAY be included when the user asks about past states.

### 2.6 Provenance Tracking

The `source` field on edges records how a conjecture was formed, enabling "why do I believe this?" tracing:

- `"conversation"` — directly stated or observed in a conversation
- `"analysis"` — produced by `/analyse` or `/reflect`
- `"inference"` — derived from other graph relationships
- `"import"` — imported from external data
- `"consolidation"` — created by the consolidation process (e.g. merged edges)

---

## 3. Confidence Dynamics

### 3.1 Strengthening (Surviving Refutation)

When a conjecture is encountered in context and no contradiction is found:

```
tests += 1
survived += 1
confidence = blend(prior, survived/tests, tests)
```

The blend function weights the basis prior against the observed survival rate, with more tests shifting weight toward the observed rate:

```
weight = tests / (tests + 3)   // 3 pseudo-observations as Bayesian prior
confidence = prior * (1 - weight) + (survived/tests) * weight
```

### 3.2 Weakening (Failing a Test)

When evidence is encountered that weakens but doesn't directly contradict:

```
tests += 1
// survived stays the same
confidence = blend(prior, survived/tests, tests)
```

### 3.3 Contradiction

When strong evidence directly contradicts the conjecture:

```
confidence *= 0.3
```

A single strong contradiction drops confidence by 70%.

### 3.4 Time Decay

Conjectures that are not tested drift toward uncertainty:

```
factor = 0.95 ^ (days_since_tested / 30)
confidence *= factor
```

Approximately 5% loss per month of inactivity.

**Decay trigger:** Confidence decay is evaluated on a **time-based trigger** (24 hours idle), not just on a request-count basis. If the ANT has been idle for 24 hours or more, decay is applied to all conjectures on the next interaction.

### 3.5 Confidence Bounds

- Confidence MUST be clamped to [0.01, 0.99]. No conjecture reaches certainty.
- Combined confidence from merging parallel edges is capped at 0.95.

### 3.6 Relevance Score

The **relevance score** combines confidence and importance:

```
relevance = confidence × importance
```

Used to prioritise which edges appear in the prompt and in what order.

### 3.7 Importance

Importance grows logarithmically with reference count:

```
importance = 0.5 + 0.5 × (1 - 1/(1 + references/10))
```

At 0 references: 0.5. At 10: ~0.8. At 50: ~0.9. Importance is clamped to [0.0, 1.0].

---

## 4. Query API

### 4.1 Query Types

Implementations MUST support these query types:

| Query | Input | Behaviour |
|-------|-------|-----------|
| **About** | label, depth | BFS traversal from the named node, depth hops. Returns subgraph with confidence-weighted relevance. |
| **Path** | from, to | Shortest path(s) between two nodes. Cumulative confidence is the product along the path (weakest link chain). |
| **ByKind** | NodeKind | All nodes of a given type, sorted by average edge confidence. |
| **Uncertain** | threshold | All edges below the given confidence, above MIN_PROMPT_CONFIDENCE. |

### 4.2 Traversal (About Query)

The About query performs a breadth-first search from a starting node:

1. Find the node by exact label match (case-insensitive).
2. If not found, try substring match (fuzzy).
3. BFS outward to the requested depth, following both incoming and outgoing edges.
4. Score each node: root = 1.0, others = average relevance_score of connecting edges.
5. Return nodes sorted by score descending, plus all edges in the subgraph.

### 4.3 Path Query

The Path query finds how two entities are connected:

1. BFS from source, tracking cumulative confidence (product of edge confidences).
2. Search both outgoing and incoming edges (undirected traversal).
3. Maximum path length: 10 hops.
4. Return up to `max_paths` results, sorted by cumulative confidence descending.

A path confidence of 0.61 (= 0.85 × 0.72) means: "if both edges are correct, the connection holds with 61% confidence."

### 4.4 Prompt Integration

For the AI system prompt:

- **Small graphs** (≤30 nodes): render the full graph.
- **Large graphs** (>30 nodes): extract entity names from the user's message, run About queries for each, render the combined traversal results.
- **Fallback**: if no entity labels match, use keyword-based subgraph extraction (inverted index + 1-hop expansion).
- **Cap**: rendered graph context MUST NOT exceed 4096 characters.

Edges below `MIN_PROMPT_CONFIDENCE` (0.15) MUST NOT appear in the prompt.

High-confidence edges (≥0.8) are rendered without qualifiers. Lower-confidence edges include a visual confidence indicator: `[●●●○○ 60%]`.

---

## 5. Episodic Memory

### 5.1 Purpose

The knowledge graph captures structured facts. Episodic memory captures **narrative** — what happened, in what order, with what outcome. Humans remember stories, not databases.

### 5.2 Episode Schema

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `date` | date string | REQUIRED | When this episode occurred |
| `participants` | string[] | OPTIONAL | Who was involved |
| `summary` | string | REQUIRED | 2-3 sentence narrative |
| `outcomes` | string[] | OPTIONAL | Key decisions or results |
| `entities` | string[] | OPTIONAL | Labels of knowledge graph nodes referenced in this episode |
| `tags` | string[] | OPTIONAL | Searchable keywords |

### 5.3 Entity Linking

The `entities` field links episodes to knowledge graph nodes by label. This enables:

- Retrieving all episodes that mention a given entity (bidirectional cross-reference).
- Providing narrative context when an entity is queried from the graph.
- Identifying entities that frequently co-occur in episodes (relationship discovery).

The AI SHOULD populate `entities` with the labels of all knowledge graph nodes mentioned or relevant to the episode.

### 5.4 When to Write Episodes

The AI SHOULD write an episode after:
- A significant conversation (not trivial questions)
- A decision is made
- A problem is solved
- A deployment or event occurs

### 5.5 Retrieval

Episodes are retrieved by keyword search against summary, outcomes, and tags, weighted by **recency** (exponential decay with 30-day half-life). Recent episodes score higher than old ones with identical keyword matches. The most relevant 5 episodes are included in the prompt.

---

## 6. Graph Consolidation

### 6.1 Purpose

Over time, the graph accumulates duplicate nodes, redundant edges, and low-value intermediary nodes. Consolidation is structural maintenance — not summarisation.

### 6.2 Operations

| Operation | Trigger | Behaviour |
|-----------|---------|-----------|
| **Node dedup** | Labels match (case-insensitive, substring, or Levenshtein ≤15% edit distance for labels ≥6 chars) + same kind | Merge: keep longer summary, union tags, move all edges |
| **Edge merge** | Same source, target, and relation | Combine: confidence = max(c1, c2), cap 0.95. Sum tests/survived/references. Concatenate contexts. |
| **Chain collapse** | A→B→C where B is a Fact with degree 2 and low importance | Collapse to A→C: confidence = min(c1, c2), combined relation |
| **Contradiction detect** | Same node pair with high/low confidence divergence | Flag as warning for AI review |
| **Community detection** | During consolidation | GraphRAG-inspired connected component analysis identifies knowledge clusters. Communities are labelled and can seed focused graph queries |

### 6.3 Confidence Merging

When merging parallel edges (same source, target, and relation):

```
combined_confidence = max(c1, c2)   // capped at 0.95
combined_context = "ctx1; ctx2"     // preserve both sources
```

The MAX strategy is used because parallel edges typically come from the same source (the AI processing different conversations). Using probabilistic OR (`1-(1-c1)(1-c2)`) would incorrectly compound confidence for non-independent observations.

This models independent confirmation: if two sources both say the same thing, the combined confidence exceeds either individual. Capped at 0.95.

For chain collapsing (transitive inference):

```
chain_confidence = min(c1, c2)
```

A chain is only as strong as its weakest link.

### 6.4 Schedule

Consolidation SHOULD run periodically (e.g. every 50 requests). Archiving (moving edges below 0.10 confidence to `knowledge-archive.json`) SHOULD run less frequently (every 100 requests).

---

## 7. Caching and Persistence

### 7.1 Write-Through Model

The knowledge graph file (`knowledge.json`) is the source of truth. The AI backend writes directly to this file. The in-memory cache detects file changes via mtime and reloads.

This is a **read-through cache**: the cache never writes to the file (except during consolidation and archiving). The AI is the sole writer during normal operation.

### 7.2 Atomic Writes

All file writes MUST use the atomic pattern: write to a temporary file, then rename. This prevents corruption on power loss.

### 7.3 Archive Safety

When archiving, the archive file MUST be written first, then the active graph. If power is lost between the two writes, duplicates exist in both files (safe — the next archive pass cleans up).

---

## 8. Keyword Extraction

### 8.1 Language Agnostic

Keyword extraction MUST NOT depend on any specific natural language. The implementation:

1. Lowercase and split on non-alphanumeric characters.
2. Filter words shorter than 3 characters.
3. Filter high-frequency function words (a small multilingual list).
4. Generate suffix-stripped variants (remove last 1, 2, 3 characters) for fuzzy matching.

This approach works for English, French, German, Spanish, and other Latin-alphabet languages. CJK languages require character-level n-gram extraction (future work).

---

## 9. Embedding-Based Retrieval

Embedding-based semantic search is available as an additional retrieval strategy via Ollama's `nomic-embed-text` model.

### 9.1 How It Works

1. Each node's label+summary is encoded as a vector via `POST /api/embed` to the local Ollama instance.
2. The user's message is encoded as a vector.
3. Nearest nodes by cosine similarity seed the graph traversal.

This enables queries like "the project Roy is working on" to find "Anthill" even when the word doesn't appear in the message.

### 9.2 Fallback

If Ollama is not installed or `nomic-embed-text` is not available, retrieval falls back to keyword-based subgraph extraction (inverted index + 1-hop expansion) as described in §4.4. The system operates fully without embeddings; they are an enhancement, not a requirement.

### 9.3 Setup

```bash
ollama pull nomic-embed-text
```

No configuration is required in `ant.toml` — the system auto-detects the embedding model at startup.

---

## 10. Security Considerations

1. **Knowledge graphs may contain sensitive information** (passwords, API keys mentioned in conversation). The AI SHOULD be instructed not to store credentials in the graph.
2. **Encrypted backups** (ANTHILL-COLONY §3.2) protect the knowledge graph at rest in git.
3. **Per-user memory files** contain user-specific information. These are accessible to the ANT but not to other ANTS in the colony.
