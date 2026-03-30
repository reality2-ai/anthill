# ANTHILL-KNOWLEDGE: Knowledge Store and Graph Operations

| Field       | Value                                                      |
|-------------|------------------------------------------------------------|
| Version     | 0.1 Draft                                                  |
| Date        | 2026-03-30                                                 |
| Status      | Draft                                                      |
| Depends on  | ANTHILL-SENTANT, R2-CBOR                                   |
| Related     | ANTHILL-THURISAZ, ANTHILL-RUMINATION, R2-KNOWLEDGE          |

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT",
"SHOULD", "SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this
document are to be interpreted as described in RFC 2119.


## 1. Introduction

The knowledge store is the persistence layer for ANT knowledge. It provides
validated writes, Bayesian integrity enforcement, and automatic git
journaling. The AI interacts through MCP tools -- direct file editing is
prohibited.

Two memory systems coexist:

1. **Knowledge graph** -- entities and conjectural relationships following
   Popperian epistemology: knowledge is conjectural and strengthened through
   surviving refutation, not through confirmation.
2. **Episodic memory** -- timestamped conversation summaries (outside the
   scope of this specification).

Both are persisted to CBOR (with JSON fallback). Context-aware retrieval
extracts relevant subgraphs for the AI prompt.


### 1.1 Terminology

| Term                   | Definition                                                              |
|------------------------|-------------------------------------------------------------------------|
| Knowledge Graph        | A directed graph of conjectural relationships between entities.         |
| Topic Graph            | A named subgraph scoped to a single topic (e.g. "anthill", "thurisaz").|
| Meta-graph             | The root graph (`knowledge.cbor`) linking topic graphs to each other.   |
| Node                   | An entity in the graph (`KnowledgeNode`).                               |
| Edge                   | A conjectural relationship with confidence (`KnowledgeEdge`).           |
| KnowledgeStore trait   | The primary API boundary; all consumers access graphs through it.       |
| LiveKnowledgeStore     | The production implementation of `KnowledgeStore`.                      |
| CborGitBackend         | The storage backend: CBOR serialisation with git auto-commit.           |
| MCP Server             | JSON-RPC stdio server exposing graph tools to Claude Code.              |
| Thought Branch         | A git branch for speculative graph exploration.                         |


### 1.2 Design Principles

1. **Validated writes only** -- the AI cannot write invalid data. All
   mutations pass through `Validated*` builder types that enforce
   constraints at construction time.
2. **CBOR binary encoding** -- approximately 46% smaller than JSON,
   serialised via `ciborium`.
3. **Git auto-commit on every mutation** -- the git history becomes a
   thinking journal. Commits can be batched inside a "thought" transaction.
4. **Trait boundary isolates storage from consumers** -- the
   `KnowledgeStore` trait decouples graph logic from file I/O and git.
5. **Popperian epistemology** -- edges are conjectures. They gain
   confidence by surviving refutation, not by accumulating confirmation.
6. **Bayesian updating** -- evidence is applied through sequential Bayesian
   updates in log-odds space, with reputation-weighted Bayes factors.


## 2. Architecture

```
Consumers (MCP tools, Web API, Rumination)
        |
KnowledgeStore trait (validated writes, anti-bias enforcement)
        |
GraphEngine (petgraph StableGraph, Bayesian updates, queries, consolidation)
        |
CborGitBackend (CBOR serialisation, atomic writes, auto-commit)
        |
Git repository (thought journal, thought branches)
```

### 2.1 Component Responsibilities

- **KnowledgeStore trait** (`store/mod.rs`): defines the complete API
  surface. All consumers -- MCP, web, maintenance, rumination -- go through
  this trait.
- **LiveKnowledgeStore** (`store/live.rs`): production implementation.
  Manages an in-memory cache of named graphs (`HashMap<String,
  KnowledgeGraph>`) behind a `RwLock`. Loads graphs lazily on first access.
  After every mutation, saves through the CBOR backend.
- **GraphEngine / KnowledgeGraph** (`knowledge.rs`, `store/engine.rs`):
  owns the `petgraph::StableGraph` and implements queries, consolidation,
  rendering, and Bayesian updates.
- **CborGitBackend** (`store/cbor_backend.rs`): handles CBOR
  serialisation, atomic file writes, and git operations (commit, branch,
  merge).
- **Validated types** (`store/validated.rs`): `ValidatedNode`,
  `ValidatedEdge`, `ValidatedEvidence` -- builder types that reject invalid
  input at construction time.
- **Epistemic engine** (`epistemic.rs`): pure functions for Bayesian
  updating, reputation adjustment, fading foundations, and chain confidence.


## 3. Graph Data Model

### 3.1 Node (`KnowledgeNode`)

| Field    | Type           | Constraints                          | Description                              |
|----------|----------------|--------------------------------------|------------------------------------------|
| label    | String         | Non-empty, max 200 chars, unique per graph | Human-readable identifier.          |
| kind     | NodeKind       | One of the 25 defined variants       | Classification of the entity.            |
| summary  | String         | --                                   | Brief description.                       |
| created  | String         | ISO 8601 date                        | When the node was created.               |
| updated  | String         | ISO 8601 date                        | When the node was last modified.         |
| tags     | Vec\<String\>  | --                                   | Free-form classification tags.           |

#### 3.1.1 NodeKind Enumeration (25 variants)

`person`, `project`, `server`, `tool`, `concept` (default), `decision`,
`event`, `fact`, `theory`, `mechanism`, `principle`, `constraint`,
`epistemology`, `problem`, `claim_type`, `claim`, `open_question`,
`implementation`, `entity`, `spec`, `repo`, `platform`, `framework`,
`r2_spec`, `other`.

The `other` variant is a serde catch-all for unknown kinds from external
graphs. Implementations MUST NOT reject nodes with unrecognised kinds;
they MUST map them to `other`.

The validated API (`ValidatedNode::new`) accepts 21 kinds and rejects
unknown values: `person`, `project`, `server`, `tool`, `concept`,
`decision`, `event`, `fact`, `theory`, `mechanism`, `principle`,
`constraint`, `problem`, `claim`, `open_question`, `implementation`,
`entity`, `spec`, `repo`, `platform`, `framework`.


### 3.2 Edge (`KnowledgeEdge`)

| Field                   | Type                     | Default    | Constraints / Range    | Description                                             |
|-------------------------|--------------------------|------------|------------------------|---------------------------------------------------------|
| relation                | String                   | --         | Non-empty              | Relationship type (e.g. "uses", "depends_on").          |
| context                 | String                   | ""         | --                     | When/where this relationship applies.                   |
| since                   | String                   | ""         | ISO 8601               | When the relationship was first recorded.               |
| confidence              | f64                      | 0.5        | [0.0, 1.0]             | Current confidence (derived from log_odds via sigmoid). |
| tests                   | u32                      | 0          | --                     | Number of times this conjecture has been tested.        |
| survived                | u32                      | 0          | --                     | Number of tests survived without contradiction.         |
| basis                   | Basis                    | assumed    | See 3.2.1              | How the conjecture was originally formed.               |
| last_tested             | String                   | ""         | ISO 8601               | When this conjecture was last tested.                   |
| importance              | f64                      | 0.5        | [0.0, 1.0]             | How central this relationship is.                       |
| references              | u32                      | 0          | --                     | How many times this edge has been referenced.           |
| valid_from              | String                   | ""         | ISO 8601               | Temporal validity start (empty = since creation).       |
| valid_until             | String                   | ""         | ISO 8601               | Temporal validity end (empty = still valid).             |
| view                    | EdgeView                 | entity     | See 3.2.2              | MAGMA-inspired orthogonal perspective.                  |
| source                  | String                   | ""         | --                     | Provenance: document, conversation, or "observation".   |
| refutation_log          | Vec\<RefutationEntry\>   | []         | --                     | Audit trail of the conjecture-and-refutation process.   |
| log_odds                | f64                      | 0.0        | [-6.9, 6.9]            | Internal belief state (source of truth for confidence). |
| evidence_log            | Vec\<Evidence\>          | []         | --                     | Typed evidence trail with Bayes factors.                |
| justificatory_chain     | Vec\<JustificationStep\> | []         | --                     | Provenance chain: why is this believed?                 |
| source_id               | String                   | ""         | --                     | Links to the reputation registry.                       |
| decay_category          | DecayCategory            | fact       | See 3.2.3              | Controls how quickly this belief fades.                 |
| beneficial_impact       | f64                      | 0.0        | [-1.0, 1.0]            | Fitness modifier: positive = beneficial.                |
| corroboration_strength  | f64                      | 0.0        | [0.0, 1.0]             | How strongly supported by neighbour edges.              |
| competition_group       | String                   | ""         | --                     | Group ID for competing hypotheses.                      |
| citations               | Vec\<Reference\>         | []         | --                     | Supporting sources (see 3.3).                           |

#### 3.2.1 Basis Enumeration

| Variant  | Initial Confidence | Description                                   |
|----------|--------------------|-----------------------------------------------|
| observed | 0.7                | Directly observed by the AI.                  |
| told     | 0.6                | Told by the user.                             |
| inferred | 0.4                | Inferred from other knowledge.                |
| assumed  | 0.3                | Assumed without evidence (default).           |
| other    | 0.3                | Catch-all for unknown values (serde fallback).|

The validated API accepts four values: `observed`, `told`, `inferred`,
`assumed`.

#### 3.2.2 EdgeView Enumeration (MAGMA-inspired)

| Variant  | Description                                          |
|----------|------------------------------------------------------|
| semantic | What things mean and how they relate conceptually.   |
| temporal | When things happened, temporal ordering.             |
| causal   | Why things happened, cause-and-effect chains.        |
| entity   | Which entities are involved, structural connections (default). |
| other    | Catch-all for unknown view types.                    |

#### 3.2.3 DecayCategory Enumeration

| Variant     | Half-life | Description                                        |
|-------------|-----------|----------------------------------------------------|
| fact        | 30 days   | Stable facts (default).                            |
| decision    | 14 days   | Design decisions, choices.                         |
| observation | 7 days    | Runtime observations, versions.                    |
| inference   | 3 days    | Inferred relationships.                            |
| assumed     | 1 day     | Assumptions without evidence.                      |
| other       | 30 days   | Catch-all (treated as fact).                       |


### 3.3 Reference (Citation)

Citations are first-class epistemic objects. A citation is not merely a
bibliographic footnote — it is a conjecture about the reliability and
relevance of a source. Citations MUST be real: fabricated citations are
worse than no citation at all. An ANT MUST NOT invent a citation it has
not verified.

Each citation SHOULD be condensed to its **core ideas** — the three or
four key claims the source makes — along with full bibliographic metadata.
This condensation serves the data reduction principle (§4.6): rather than
re-reading entire documents on every prompt, the ANT stores the essential
insights as graph edges linked to the citation by `cite_id`.

Citations are subject to the same conjecture-refutation process as all
other knowledge. A citation's `quality` score starts based on its
`ref_type` but evolves through evidence: if claims from a source
repeatedly survive refutation, its quality rises. If claims from a source
are contradicted, its quality falls. A peer-reviewed paper that fails
refutation is less valuable than a blog post that survives it.

| Field    | Type          | Default | Constraints                     | Description                              |
|----------|---------------|---------|---------------------------------|------------------------------------------|
| cite_id  | String        | ""      | Format: "cite-\<8hex\>"        | Unique citation identifier.              |
| url      | String        | ""      | --                              | URL of the source (if web-based).        |
| title    | String        | ""      | MUST be non-empty if known      | Title or short description.              |
| author   | String        | ""      | --                              | Author(s) if known.                      |
| date     | String        | ""      | --                              | Publication date or year.                |
| accessed | String        | ""      | ISO 8601 RECOMMENDED            | When this source was accessed or cited.  |
| snippet  | String        | ""      | --                              | Brief quote or core idea summary.        |
| ref_type | ReferenceType | website | See 3.3.1                       | Source classification.                   |
| quality  | f64           | 0.5     | [0.0, 1.0]                     | Quality score (evolves through evidence).|

#### 3.3.1 ReferenceType Enumeration

The source type determines the **initial** quality score. This is a
prior, not a ceiling — the quality evolves as claims from the source
are tested through the conjecture-refutation process.

| Variant         | Initial Quality | Description                                |
|-----------------|----------------:|--------------------------------------------|
| peer_reviewed   |             0.8 | Peer-reviewed scientific paper.            |
| official_report |             0.7 | Government or official publication.        |
| book            |             0.7 | Book or textbook.                          |
| ant_knowledge   |             0.6 | Another ANT's knowledge graph.             |
| news            |             0.5 | News article from a reputable source.      |
| personal        |             0.5 | Personal communication or user statement.  |
| website         |             0.4 | General web source.                        |
| blog            |             0.3 | Blog post or opinion piece.                |
| ai_inference    |             0.3 | AI's own reasoning with no external source.|
| other           |             0.3 | Catch-all for unknown types.               |

#### 3.3.2 Citation Lifecycle

1. **Acquisition** — when the ANT encounters a source (URL, PDF, file),
   it SHOULD download the content and cache it in `files/` for future
   reference. The content is read to extract core ideas.
2. **Condensation** — the ANT distils the source to its 3-4 key claims
   and records them as graph edges, each linked to the citation by
   `cite_id`. The `snippet` field captures the most important quote or
   idea.
3. **Linking** — each topic graph edge that is supported by a citation
   MUST include the citation in its `citations` vector. An edge with
   well-sourced citations receives a relevance boost (§3.2, citation
   bonus).
4. **Verification** — during rumination, the citation consolidation task
   revisits citations with '?' relations, fetches their content, and
   determines the actual relationship.
5. **Evolution** — the citation's `quality` score rises or falls based on
   whether claims from this source survive refutation. A citation that
   supported a refuted claim has its quality reduced.

#### 3.3.3 Citation Integrity Rules

1. An ANT MUST NOT fabricate a citation. If the source cannot be verified,
   the ANT MUST NOT include it.
2. If knowledge comes from AI inference (the ANT's own reasoning), the
   `ref_type` MUST be `ai_inference` — NOT a fake external source.
3. If a URL cannot be fetched or a source cannot be found, the ANT MUST
   add a question to `questions.json` asking the human to confirm it.
4. Downloaded citation content MUST be cached in `files/` so it does not
   need to be re-fetched on future rumination cycles.
5. A citation that survives refutation is more valuable than ten that
   were never tested.


### 3.4 Evidence Entry

| Field             | Type         | Description                                        |
|-------------------|--------------|----------------------------------------------------|
| date              | String       | When this evidence was observed.                   |
| evidence_type     | EvidenceType | Classification of the evidence (see 3.4.1).        |
| test              | String       | What was tested or observed.                       |
| detail            | String       | The evidence itself.                               |
| source_id         | String       | Source identifier (links to reputation registry).  |
| source_reputation | f64          | Source reputation at time of evidence (for audit). |
| bayes_factor      | f64          | The effective Bayes factor applied.                |
| log_odds_before   | f64          | Log-odds before this evidence.                     |
| log_odds_after    | f64          | Log-odds after this evidence.                      |

#### 3.4.1 EvidenceType Enumeration (12 types + catch-all)

| Type                   | Base BF | Rep-dependent | Description                                       |
|------------------------|--------:|:-------------:|---------------------------------------------------|
| corroboration          |     2.0 | Yes           | Supporting evidence found in another source.      |
| contradiction          |     0.3 | Yes           | Contradicting evidence found.                     |
| refutation_survived    |     2.5 | No            | Claim survived active disproof attempt.           |
| refutation_failed      |     0.1 | No            | Claim failed active disproof attempt.             |
| human_attestation      |     1.5 | Yes           | User confirmed or corrected.                      |
| consistency            |     1.5 | No            | Consistent with existing knowledge graph.         |
| inconsistency          |     0.4 | No            | Inconsistent with existing knowledge graph.       |
| synthesis              |     1.2 | No            | Transitive inference from two strong edges.       |
| competition_won        |     2.0 | No            | Won competition against a rival hypothesis.       |
| competition_lost       |     0.3 | No            | Lost competition against a rival hypothesis.      |
| pattern_transfer       |     1.8 | No            | Cross-domain pattern transfer.                    |
| inconsequential_search |     1.0 | No            | Searched for counter-evidence, found nothing.     |
| unknown                |     1.0 | No            | Catch-all (serde fallback). No update applied.    |

Reputation-dependent types have their effective Bayes factor computed as:

    BF_adj = BF_base ^ (0.5 + 0.5 * r)

where `r` is the source reputation in [0.0, 1.0]. This means:
- r=0.0 (untrusted): BF is square-rooted (dampened).
- r=0.5 (neutral): BF^0.75.
- r=1.0 (fully trusted): full BF applied.

Reputation can only attenuate, never amplify beyond the base BF.


### 3.5 Justification Step

| Field      | Type   | Description                                          |
|------------|--------|------------------------------------------------------|
| step       | u32    | Step number in the justification chain.              |
| process    | String | What process produced this evidence.                 |
| confidence | f64    | Confidence at this point in the chain.               |
| source     | String | Source identifier (e.g. "document:README.md").       |


### 3.6 Refutation Entry

| Field             | Type   | Description                                        |
|-------------------|--------|----------------------------------------------------|
| date              | String | When this test occurred.                           |
| test              | String | What was tested.                                   |
| evidence          | String | What evidence was considered.                      |
| outcome           | String | "survived", "weakened", or "contradicted".         |
| confidence_before | f64    | Confidence before this test.                       |
| confidence_after  | f64    | Confidence after this test.                        |


## 4. KnowledgeStore Trait

The `KnowledgeStore` trait is the primary interface to the knowledge
system. All consumers go through this trait. The trait requires `Send +
Sync` for concurrent access.


### 4.1 Graph Management

```
fn list_graphs(&self) -> StoreResult<Vec<GraphInfo>>
```

Returns summary information (name, node count, edge count) for all
available graphs (meta + topic graphs).

```
fn graph_stats(&self, graph: &str) -> StoreResult<GraphStats>
```

Returns detailed statistics for a named graph: node count, edge count,
average confidence, count of uncertain edges, count of orphan nodes.


### 4.2 Node Operations

```
fn add_node(&self, graph: &str, node: ValidatedNode) -> StoreResult<NodeId>
```

Adds a validated node to the named graph. Returns an opaque `NodeId`
wrapping a `petgraph::NodeIndex`. Fails with `StoreError::Duplicate` if a
node with the same label already exists.

```
fn get_node(&self, graph: &str, label: &str) -> StoreResult<KnowledgeNode>
```

Retrieves a node by its label. Fails with `StoreError::NotFound` if the
label does not exist.

```
fn list_nodes(&self, graph: &str) -> StoreResult<Vec<String>>
```

Returns the labels of all nodes in the named graph.


### 4.3 Edge Operations (Validated)

```
fn add_edge(&self, graph: &str, edge: ValidatedEdge) -> StoreResult<EdgeId>
```

Adds a validated edge between two existing nodes. The `ValidatedEdge`
carries the from-label, to-label, and the edge data. Confidence is
initialised from the basis (see 3.2.1) and log-odds is computed
accordingly. Returns an opaque `EdgeId`.

```
fn update_evidence(
    &self, graph: &str, from: &str, to: &str, relation: &str,
    evidence: ValidatedEvidence,
) -> StoreResult<EdgeUpdate>
```

The primary Thurisaz update path. Applies typed evidence to an existing
edge using sequential Bayesian updating in log-odds space. Returns an
`EdgeUpdate` containing confidence before/after, log-odds before/after,
the evidence type, the effective Bayes factor, and an optional
confirmation bias warning.

```
fn strengthen(
    &self, graph: &str, from: &str, to: &str, relation: &str,
    test: &str, evidence: &str,
) -> StoreResult<EdgeUpdate>
```

Strengthens an edge (the conjecture survived a refutation attempt). Uses
`EvidenceType::RefutationSurvived` (BF = 2.5).

```
fn weaken(
    &self, graph: &str, from: &str, to: &str, relation: &str,
    test: &str, evidence: &str,
) -> StoreResult<EdgeUpdate>
```

Weakens an edge (an inconsistency was found). Uses
`EvidenceType::Inconsistency` (BF = 0.4).

```
fn contradict(
    &self, graph: &str, from: &str, to: &str, relation: &str,
    test: &str, evidence: &str,
) -> StoreResult<EdgeUpdate>
```

Contradicts an edge (refutation failed -- sharp penalty). Uses
`EvidenceType::RefutationFailed` (BF = 0.1).

```
fn add_citation(
    &self, graph: &str, from: &str, to: &str, relation: &str,
    citation: Reference,
) -> StoreResult<()>
```

Adds a citation to an existing edge. The citation is appended to the
edge's `citations` vector.


### 4.4 Queries (Read-only)

```
fn query_about(
    &self, graph: &str, entity: &str, depth: usize,
) -> StoreResult<QueryResult>
```

Returns the subgraph within `depth` hops of the named entity. The
`QueryResult` contains nodes with relevance scores, weighted edges, and
any paths found.

```
fn query_path(
    &self, graph: &str, from: &str, to: &str, max_paths: usize,
) -> StoreResult<QueryResult>
```

Finds up to `max_paths` paths between two entities.

```
fn query_by_kind(
    &self, graph: &str, kind: &str,
) -> StoreResult<QueryResult>
```

Returns all nodes of a given kind and their connecting edges.

```
fn query_uncertain(
    &self, graph: &str, threshold: f64,
) -> StoreResult<QueryResult>
```

Returns all edges with confidence below `threshold`.

```
fn query_justification(
    &self, graph: &str, from: &str, to: &str, relation: &str,
) -> StoreResult<String>
```

Returns a human-readable justification chain for an edge, describing the
evidence trail and reasoning.

```
fn list_orphans(&self, graph: &str) -> StoreResult<Vec<String>>
```

Returns labels of orphan nodes -- nodes with only undetermined ('?')
connections or no connections at all.


### 4.5 Maintenance

```
fn consolidate(&self, graph: &str) -> StoreResult<ConsolidationReport>
```

Runs the consolidation pipeline (see Section 6). Returns a report of
nodes merged, edges merged, chains collapsed, contradictions detected, and
community clusters.

```
fn apply_decay(&self, graph: &str, days: u32) -> StoreResult<u32>
```

Applies time-based fading foundations to all edges. Each edge decays
toward p=0.5 (log-odds=0) according to its decay category's half-life.
Returns the number of edges affected.

```
fn compute_corroboration_strength(&self, graph: &str) -> StoreResult<()>
```

Recomputes the `corroboration_strength` field for all edges based on the
confidence of neighbouring edges.

```
fn link_orphans(&self, graph: &str) -> StoreResult<u32>
```

Links orphan nodes to a hub node. Returns the number of nodes linked.

```
fn backfill_thurisaz(&self, graph: &str) -> StoreResult<u32>
```

Backfills Thurisaz-format fields (log_odds, evidence_log,
justificatory_chain, decay_category) on legacy edges that predate the
epistemic engine. Returns the number of edges updated.


### 4.6 Rendering

```
fn render_for_prompt(&self, message: &str, max_chars: usize) -> String
```

Renders relevant knowledge for the AI system prompt. This is the primary
data reduction mechanism: an ANT's knowledge graph may contain thousands
of nodes and edges (megabytes of data), but the AI's context window is
finite and expensive. This method MUST distil the graph down to the most
relevant subset — typically ~4KB from potentially megabytes of raw graph
data.

The reduction pipeline:

1. **Semantic search** — if Ollama embeddings are available, encode the
   user's message as a vector and find the nearest nodes by cosine
   similarity. This identifies which parts of the graph are topically
   relevant.
2. **Keyword fallback** — if embeddings are unavailable, use TF-IDF
   keyword matching against node labels, edge relations, and context
   fields.
3. **Subgraph extraction** — from the matched nodes, extract their
   immediate neighbourhood (1-2 hops) to provide relationship context.
4. **Confidence filtering** — edges below MIN_PROMPT_CONFIDENCE (0.15)
   MUST be excluded. They are too uncertain to be useful.
5. **Confidence qualification** — high-confidence edges (>=0.8) are
   rendered without qualifiers. Lower-confidence edges include a visual
   indicator so the AI knows what is well-established vs uncertain.
6. **Budget enforcement** — the rendered text MUST fit within `max_chars`
   (typically ~4000). Lowest-relevance content is truncated first.

This intelligent graph building is a core design principle: the knowledge
graph is a compression of the ANT's understanding. Rather than sending
entire conversation histories or document contents to the AI, the graph
captures the essential relationships, evidence, and confidence — enabling
the AI to reason with a fraction of the data that would otherwise be
required.

```
fn to_visualization(&self, graph: &str) -> StoreResult<serde_json::Value>
```

Renders the graph as a JSON structure suitable for 3D visualisation (nodes
with positions, edges with weights).


### 4.7 Rumination Support

```
fn refutation_candidates(&self, graph: &str, limit: usize)
    -> StoreResult<Vec<(String, String, String, f64, f64)>>
```

Finds edges suitable for refutation: important but uncertain. Returns
tuples of (from, to, relation, confidence, importance).

```
fn synthesis_candidates(&self, graph: &str, limit: usize)
    -> StoreResult<Vec<(NodeId, NodeId, String, String, String)>>
```

Finds synthesis opportunities: A->B->C paths where no direct A->C edge
exists.

```
fn undetermined_connections(&self, graph: &str, limit: usize)
    -> StoreResult<Vec<(String, String)>>
```

Finds edges with '?' relations (undetermined connections).

```
fn find_competitors(&self, graph: &str)
    -> StoreResult<Vec<CompetitorGroup>>
```

Identifies groups of competing hypotheses (edges sharing a competition
group).

```
fn contradiction_pairs(&self, graph: &str)
    -> StoreResult<Vec<ContradictionPair>>
```

Finds pairs of edges between the same nodes that contradict each other.

```
fn uncertainty_stats(&self, graph: &str)
    -> StoreResult<UncertaintyStats>
```

Returns aggregate uncertainty statistics for the graph.

```
fn cross_domain_patterns(&self, graph_a: &str, graph_b: &str, limit: usize)
    -> StoreResult<Vec<PatternMatch>>
```

Finds structural patterns shared between two different topic graphs.


### 4.8 Git Integration

```
fn commit(&self, message: &str) -> StoreResult<String>
```

Explicitly commits current state with a message. Returns the commit hash.
Note: the CBOR backend auto-commits after every mutation, so explicit
commits are typically used for checkpointing.

```
fn history(&self, graph: &str, limit: usize) -> StoreResult<Vec<CommitInfo>>
```

Returns recent commit history for a graph. Each `CommitInfo` contains the
hash, message, and timestamp.


### 4.9 Thought Branches

```
fn begin_thought(&self)
```

Begins a thought transaction. Subsequent saves are batched -- no
individual commits until `end_thought` is called.

```
fn end_thought(&self, message: &str) -> StoreResult<String>
```

Ends the thought transaction and commits all batched changes atomically.
The message SHOULD describe what was reasoned about, not what files
changed.

```
fn create_thought_branch(&self, name: &str) -> StoreResult<String>
```

Creates a git branch named `thought/<name>` for speculative exploration.
Commits any pending changes first. Returns the branch name.

```
fn merge_thought_branch(&self, branch: &str) -> StoreResult<bool>
```

Merges a thought branch into main using `--no-ff`. Returns true if the
merge succeeded. On conflict, aborts and returns false. Deletes the branch
after successful merge.

```
fn abandon_thought_branch(&self, branch: &str) -> StoreResult<()>
```

Switches back to main and deletes the thought branch. The exploration was
a dead end.

```
fn list_thought_branches(&self) -> StoreResult<Vec<String>>
```

Lists all branches matching `thought/*`.

```
fn current_branch(&self) -> StoreResult<String>
```

Returns the name of the current git branch.


## 5. CBOR+Git Backend

### 5.1 File Layout

```
<ant-working-dir>/
  memory/
    knowledge.cbor             -- meta-graph
    knowledge.json             -- legacy JSON (read for migration)
    graphs/
      <topic>.cbor             -- topic graphs
      <topic>.json             -- legacy JSON fallback
      <topic>-archive.json     -- archived low-confidence edges
```

The meta-graph is stored at `memory/knowledge.cbor`. Topic graphs are
stored at `memory/graphs/<topic>.cbor`.


### 5.2 CBOR Encoding

Graphs are serialised using `ciborium` (a pure-Rust CBOR implementation).
The serialisation format is `GraphData`:

```
GraphData {
    nodes: Vec<Option<KnowledgeNode>>,   // None slots for removed nodes
    edges: Vec<(usize, usize, KnowledgeEdge)>,  // (source_idx, target_idx, edge)
}
```

CBOR encoding produces files approximately 46% smaller than equivalent
JSON.


### 5.3 Atomic Writes

All saves follow the write-rename pattern:

1. Serialise to a `.cbor.tmp` file.
2. Call `fsync` on the temporary file.
3. Rename `.cbor.tmp` to `.cbor` (atomic on POSIX).

This ensures no partial writes corrupt the graph.


### 5.4 Auto-commit

Every `save_graph` call triggers a git commit unless:
- Auto-commit is disabled (batch/migration mode), OR
- A thought transaction is active (`begin_thought` was called).

The commit sequence:
1. `git add memory/`
2. Check for staged changes (`git diff --cached --quiet`).
3. If changes exist: `git commit -m "<descriptive message>"`.
4. Return the short commit hash.


### 5.5 Legacy JSON Migration

When loading a graph, the backend checks both CBOR and JSON paths:
- If both exist, the NEWER file by mtime is preferred.
- If JSON has citations that CBOR lacks, JSON is preferred.
- After loading from JSON, the backend auto-converts to CBOR.
- If CBOR parsing fails, the backend falls back to JSON.


## 6. Consolidation

The consolidation pipeline runs the following steps in order:

### 6.1 Deduplicate Nodes

Nodes with similar labels (fuzzy Levenshtein matching) and the same
`kind` are merged. All edges from the removed node are redirected to the
surviving node.

### 6.2 Merge Parallel Edges

Edges sharing the same source node, target node, and relation are merged
into a single edge. The edge with the higher confidence is kept.

### 6.3 Collapse Chains

Chains of the form A->B->C where B has degree 2 (one incoming, one
outgoing) and B is a `Fact` node are collapsed into a direct A->C edge.
The intermediate node B is removed.

### 6.4 Clean Relation Names

Two cleaning passes:
1. Strip '?' prefixes: if a relation starts with '?' followed by a
   non-empty title (e.g. "? some_relationship"), the '?' is removed.
   Pure '?' relations (undetermined) are left as-is.
2. Strip arrow characters: formatting noise like "->" or "-->" is removed
   from relation names.

### 6.5 Detect Contradictions

Identifies pairs of edges between the same nodes that assert contradictory
relationships. These are reported in the `ConsolidationReport` for human
or AI review.

### 6.6 Community Detection

GraphRAG-inspired connected-component analysis groups nodes into clusters.
Disconnected subgraphs are identified and reported.

### 6.7 Backfill Citation IDs

Citations missing a `cite_id` are assigned one by hashing the URL and
title: `cite-<8hex>`.

### 6.8 Rebuild Index

The keyword index is rebuilt from the current graph state to ensure
queries reflect the post-consolidation structure.


## 7. MCP Server

### 7.1 Protocol

The MCP server communicates over JSON-RPC 2.0 on stdio (stdin/stdout).
It is launched as:

```
anthill --mcp-server --memory-dir <path>
```

The server is auto-configured in `.claude/settings.json` for each ANT
working directory.

### 7.2 Tools Exposed

The following MCP tools are exposed, each mapping to a `KnowledgeStore`
trait method:

| MCP Tool                    | Trait Method            | Category   |
|-----------------------------|-------------------------|------------|
| graph_add_node              | add_node                | Write      |
| graph_add_edge              | add_edge                | Write      |
| graph_update_evidence       | update_evidence         | Write      |
| graph_strengthen            | strengthen              | Write      |
| graph_weaken                | weaken                  | Write      |
| graph_contradict            | contradict              | Write      |
| graph_add_citation          | add_citation            | Write      |
| graph_query_about           | query_about             | Read       |
| graph_query_path            | query_path              | Read       |
| graph_query_by_kind         | query_by_kind           | Read       |
| graph_query_uncertain       | query_uncertain         | Read       |
| graph_query_justification   | query_justification     | Read       |
| graph_query_reputation      | (reputation registry)   | Read       |
| graph_list_graphs           | list_graphs             | Read       |
| graph_list_orphans          | list_orphans            | Read       |

All write tools accept validated input and return structured results. On
validation failure, the MCP server returns a JSON-RPC error with a
descriptive message.


## 8. Validation Rules

Implementations MUST enforce the following validation rules at the API
boundary (in the `Validated*` constructors):

### 8.1 Node Validation

1. Node labels MUST be non-empty after trimming.
2. Node labels MUST NOT exceed 200 characters.
3. Node kind MUST be one of the 21 accepted values (see 3.1.1).

### 8.2 Edge Validation

1. The `from` and `to` labels MUST be non-empty after trimming.
2. The `relation` MUST be non-empty after trimming.
3. The `basis` MUST be one of: `observed`, `told`, `inferred`, `assumed`.
4. The `view` MUST be one of: `semantic`, `temporal`, `causal`, `entity`
   (or empty, which maps to `entity`).
5. The `beneficial_impact` MUST be clamped to [-1.0, 1.0].

### 8.3 Evidence Validation

1. The `evidence_type` MUST be one of the 12 defined types:
   `corroboration`, `contradiction`, `refutation_survived`,
   `refutation_failed`, `human_attestation`, `consistency`,
   `inconsistency`, `synthesis`, `competition_won`, `competition_lost`,
   `pattern_transfer`, `inconsequential_search`.
2. The `test` field MUST be non-empty after trimming.
3. The `source_reputation` MUST be clamped to [0.0, 1.0].

### 8.4 Bayesian Update Rules

1. Log-odds MUST be clamped to [-6.9, 6.9] after every update (equivalent
   to probability [0.001, 0.999]).
2. Bayes factors MUST match the predefined base values per evidence type
   (see 3.4.1), modulated by the reputation adjustment formula.
3. Invalid Bayes factors (BF <= 0) MUST be ignored (no update applied).
4. Confidence MUST be derived from log-odds via the sigmoid function:
   `confidence = 1 / (1 + exp(-log_odds))`.

### 8.5 Citation Validation

1. The `cite_id` format MUST be "cite-\<8hex\>" (8 lowercase hexadecimal
   characters). If empty on input, implementations MUST auto-generate a
   `cite_id` by hashing the URL and title.
2. The `quality` field MUST be in [0.0, 1.0].


## 9. Error Types

The store defines the following error variants:

| Error               | Meaning                                          |
|---------------------|--------------------------------------------------|
| StoreError::Validation | A field value failed validation.              |
| StoreError::NotFound   | The requested entity was not found.           |
| StoreError::Duplicate  | A duplicate entity already exists.            |
| StoreError::Storage    | I/O or serialisation error.                   |
| StoreError::Git        | Git operation failed.                         |


## 10. Conformance

A conforming implementation of the Knowledge Store:

1. MUST implement all methods of the `KnowledgeStore` trait.
2. MUST enforce all validation rules specified in Section 8.
3. MUST use sequential Bayesian updating in log-odds space for evidence
   application.
4. MUST clamp log-odds to [-6.9, 6.9] after every update.
5. MUST persist graphs atomically (no partial writes observable).
6. SHOULD auto-commit to git after every mutation (MAY defer during
   thought transactions).
7. MUST support both CBOR and legacy JSON loading for backward
   compatibility.
8. MUST run the full consolidation pipeline (Section 6) when
   `consolidate` is called.
9. MUST return descriptive error messages on validation failure, including
   the invalid value and the list of valid alternatives.
10. SHOULD implement the `StorageBackend` trait for pluggable storage.
11. MUST support concurrent access via `Send + Sync`.
12. SHOULD support thought branches for speculative exploration.
