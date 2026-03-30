# ANTHILL-THEMATIC: Analysis Pipelines

| Field      | Value                                                        |
|------------|--------------------------------------------------------------|
| Version    | 0.1 Draft                                                    |
| Date       | 2026-03-30                                                   |
| Status     | Draft                                                        |
| Depends on | ANTHILL-KNOWLEDGE, ANTHILL-WORKER                            |
| Related    | ANTHILL-EXPORT, ANTHILL-CHAT                                 |

> The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT",
> "SHOULD", "SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this
> document are to be interpreted as described in RFC 2119.

---

## 1. Introduction

Anthill provides three analysis pipelines that transform source material into
structured artefacts:

1. **Thematic Analysis** (`/analyse`) -- converts documents into knowledge
   graph nodes and edges using a six-phase methodology derived from Braun &
   Clarke's reflexive thematic analysis (2006, revised 2022).

2. **Specification Generation** (`/specify`) -- extracts behaviours,
   invariants, and contracts from source code and produces a formal
   specification document using RFC 2119 requirement levels.

3. **Test Vector Generation** (`/test-vectors`) -- derives concrete test
   cases from source code or specification documents, covering normal, edge,
   error, and security scenarios.

All three pipelines share a common architecture: a file is read, its content
is passed to the AI worker as a structured prompt, and the AI backend
performs the analysis. The pipelines reuse the same document-reading
infrastructure and security restrictions.

### 1.1 Terminology

| Term              | Definition                                                                                          |
|-------------------|-----------------------------------------------------------------------------------------------------|
| Code              | A labelled extract from Phase 2: an entity, concept, decision, tool, person, or fact identified in the source material. |
| Theme             | A pattern of shared meaning underpinned by a central concept, grouping multiple codes (Phase 3).    |
| Relationship      | A directed edge between two codes, with a relation type, basis, and confidence (Phase 5).           |
| Behaviour         | A contract or invariant extracted from source code during specification generation.                  |
| Test Vector       | A concrete test case specifying input, expected output, setup, and category.                        |
| Basis             | How a relationship was determined: `observed` (explicit in text), `inferred` (implied), or `assumed` (interpretation). |
| Chunk             | A segment of a document produced by the chunking algorithm for AI processing.                       |

---

## 2. Thematic Analysis (`/analyse`)

The `/analyse` command performs Braun & Clarke's reflexive thematic analysis
on a document or source file and integrates the results into the ANT's
knowledge graph.

An implementation MUST execute all six phases in order. Each phase MUST
complete before the next begins.

### 2.1 Phase 1: Familiarisation

The implementation MUST read the entire source document. For documents
exceeding the chunk size threshold, the implementation MUST chunk the
document into overlapping segments (see Section 5.1).

The AI backend MUST produce a 2--3 sentence overview of the document's
subject matter, structure, and key topics.

### 2.2 Phase 2: Coding

The AI backend MUST extract every significant entity, concept, decision,
tool, person, and fact from the source material. Each extracted item is a
**code** with the following fields:

| Field       | Type       | Description                                                |
|-------------|------------|------------------------------------------------------------|
| label       | string     | Short identifier (e.g. "Rust", "Ed25519 signing")          |
| kind        | string     | One of: `person`, `project`, `server`, `tool`, `concept`, `decision`, `event`, `fact` |
| description | string     | One-sentence explanation                                   |
| evidence    | string     | Exact quote or paraphrase from the source supporting this code |
| tags        | string[]   | Searchable keywords (MAY be empty)                         |

The AI backend MUST return codes as a JSON array. The implementation MUST
tolerate markdown code fences (```` ```json ... ``` ````) wrapping the output
and MUST strip them before parsing.

If JSON parsing fails on the complete array, the implementation SHOULD
attempt fallback strategies (e.g. line-by-line parsing) and MAY return an
empty result.

### 2.3 Phase 3: Theme Generation

The AI backend MUST group codes into higher-level themes. A theme is defined
as "a pattern of shared meaning underpinned by a central concept or idea"
(Braun & Clarke, 2022). The implementation SHOULD produce 3--8 themes per
analysis.

Each theme has the following fields:

| Field   | Type     | Description                                             |
|---------|----------|---------------------------------------------------------|
| name    | string   | Concise theme name                                      |
| concept | string   | The central idea unifying the member codes              |
| codes   | string[] | Labels of codes belonging to this theme                 |
| support | float    | How well-evidenced this theme is (0.0--1.0)             |

### 2.4 Phase 4: Review

The AI backend MUST re-read the source document and validate each theme
against the original material. The review MUST check:

- Whether the evidence genuinely supports each theme.
- Whether any codes were missed during Phase 2.
- Whether any themes are too broad or too narrow.

The AI backend MUST revise themes and codes as needed before proceeding.

### 2.5 Phase 5: Refinement

The AI backend MUST identify relationships between entities. Each
relationship has the following fields:

| Field    | Type   | Description                                                     |
|----------|--------|-----------------------------------------------------------------|
| from     | string | Source entity label (MUST match a code label)                   |
| to       | string | Target entity label (MUST match a code label)                   |
| relation | string | Relationship type (e.g. `uses`, `deployed_on`, `depends_on`, `part_of`, `decided`) |
| context  | string | Brief description of the relationship                           |
| basis    | string | How determined: `observed`, `inferred`, or `assumed`            |

The AI backend MUST also identify a **view** for each relationship: one of
`semantic`, `temporal`, `causal`, or `entity`.

### 2.6 Phase 6: Integration

The implementation MUST write analysis results into the knowledge graph
following these rules:

1. **Topic graph selection.** The AI backend MUST determine a topic name
   (lowercase-hyphenated, e.g. `anthill-architecture`) and write results to
   `memory/graphs/<topic>.json`.

2. **Read before write.** The implementation MUST read the existing topic
   graph (if any) before writing, to avoid duplicating nodes or edges.

3. **Node creation.** For each code, add or update a node in the topic
   graph. If a node with a matching label already exists, the implementation
   MUST NOT duplicate it but SHOULD update its summary if the new version is
   richer.

4. **Theme nodes.** For each theme, add a concept node and link its member
   codes with `part_of` edges.

5. **Edge creation.** For each relationship, add an edge as a
   Thurisaz-compliant conjecture with the following initial confidence
   values based on basis:

   | Basis      | Initial confidence | Log-odds             |
   |------------|--------------------|----------------------|
   | `observed` | 0.7                | ln(0.7 / 0.3) ~ 0.85 |
   | `inferred` | 0.4                | ln(0.4 / 0.6) ~ -0.41 |
   | `assumed`  | 0.3                | ln(0.3 / 0.7) ~ -0.85 |

6. **Evidence typing.** When an edge corroborates an existing edge, the
   implementation MUST use evidence type `corroboration`. When it contradicts
   an existing edge, the implementation MUST use `contradiction`. For new
   edges, the implementation MUST use `consistency`.

7. **Corroboration and contradiction.** If a relationship already exists,
   the implementation MUST strengthen it (increment `survived` and `tests`).
   If a relationship contradicts an existing edge, the implementation MUST
   weaken the existing edge (increment `tests` only).

8. **Justificatory chain.** Each new edge MUST include a justificatory chain
   entry: `step=1`, `process="Thematic analysis of <source_name>"`,
   `confidence=<initial>`, `source="document:<source_name>"`.

9. **Source event node.** The implementation MUST add the source document as
   an event node with today's date.

10. **Meta-graph update.** The implementation MUST update
    `memory/knowledge.json` to include a node for the topic graph (kind:
    `concept`, tags: `["graph", "topic"]`) and edges to related existing
    topics.

---

## 3. Specification Generation (`/specify`)

The `/specify` command reads a source file and generates a formal
specification document in the Anthill specification style.

### 3.1 Process

The implementation MUST perform the following steps:

1. **Read source file.** Read the file using the document reader
   (Section 5). If the content exceeds 30,000 characters, the
   implementation MUST truncate to the first 30,000 characters at a safe
   boundary.

2. **Derive spec name.** The implementation MUST derive the specification
   name from the source filename by replacing periods with hyphens and
   converting to uppercase (e.g. `web.rs` becomes `WEB-RS`).

3. **Extract behaviours.** The AI backend MUST identify all behaviours,
   invariants, and contracts in the source code. Each behaviour has the
   following fields:

   | Field       | Type     | Description                                          |
   |-------------|----------|------------------------------------------------------|
   | name        | string   | Short identifier (e.g. `auth_rejects_empty_credential`) |
   | description | string   | What the code does (descriptive)                     |
   | invariant   | string   | The contract in RFC 2119 language                    |
   | source      | string   | Function name or code reference                      |
   | level       | string   | One of: `must`, `should`, `may`                      |
   | related     | string[] | Names of related behaviours (MAY be empty)           |

   The extraction MUST focus on: input validation and error handling, state
   transitions and lifecycle, security boundaries and access control, data
   persistence and consistency, concurrency and ordering guarantees, and
   configuration defaults and overrides.

4. **Group into sections.** The AI backend MUST group behaviours into 4--8
   logical specification sections, ordered from setup through normal
   operation to error handling and security. Each section has:

   | Field       | Type     | Description                                      |
   |-------------|----------|--------------------------------------------------|
   | title       | string   | Section heading                                  |
   | number      | string   | Section number (e.g. `3.2`)                      |
   | behaviours  | string[] | Behaviour names belonging to this section        |
   | description | string   | 2--3 sentence overview                           |

5. **Generate specification document.** The AI backend MUST produce a
   complete Markdown document with:
   - Header with title, version (`0.1 Draft`), date, status, and dependencies
   - RFC 2119 keyword notice
   - Terminology table
   - Numbered sections matching the grouped structure
   - Each behaviour written as a normative statement in its section
   - Examples where helpful
   - A security considerations section

6. **Save.** The specification MUST be saved to `specs/<SPEC-NAME>.md`.

### 3.2 Output Format

The generated specification MUST follow the R2 specification style:

- Metadata table at the top (Version, Date, Status, Depends on, Related).
- RFC 2119 block quote.
- Numbered sections with prose and normative statements.
- Requirement levels: `MUST`, `SHOULD`, `MAY` (and their negations).

---

## 4. Test Vector Generation (`/test-vectors`)

The `/test-vectors` command generates concrete test cases from source code
or specification documents.

### 4.1 Source Type Detection

The implementation MUST detect the source type:

- **Specification:** files ending in `.md` that contain the keyword `MUST`.
- **Source code:** files ending in `.rs`, `.py`, `.ts`, or `.go`.
- **Document:** all other files.

The detected type MUST be communicated to the AI backend to guide the
style of test generation.

### 4.2 Test Vector Format

For each behaviour or requirement found, the AI backend MUST generate 2--3
test vectors covering:

- **Normal cases** -- expected happy-path behaviour.
- **Edge cases** -- boundary conditions and unusual but valid inputs.
- **Error cases** -- invalid input handling, missing data, malformed requests.
- **Security cases** -- injection attacks, overflow, privilege escalation
  (where relevant).

Each test vector has the following fields:

| Field       | Type   | Description                                              |
|-------------|--------|----------------------------------------------------------|
| behavior    | string | Which behaviour or requirement this tests                |
| test_name   | string | `snake_case` name suitable for a Rust `#[test]` function |
| description | string | Human-readable explanation of the test                   |
| setup       | string | Preconditions (empty string if none)                     |
| input       | string | What to do or feed in                                    |
| expected    | string | What should happen                                       |
| category    | string | One of: `normal`, `edge`, `error`, `security`            |

### 4.3 Output Formats

The implementation MUST support two output formats:

1. **Rust test stubs.** When the source is code (especially `.rs` files),
   the AI backend SHOULD generate runnable Rust `#[test]` function stubs
   within a `#[cfg(test)] mod tests { ... }` block, including necessary
   imports and `TODO` comments where implementation details are needed.

2. **JSON vectors.** The AI backend MUST return test vectors as a JSON
   array conforming to the schema in Section 4.2. The implementation MUST
   strip markdown fences before parsing.

When the source is a specification, the AI backend MUST generate tests that
verify the specification requirements. When the source is code, the AI
backend MUST generate tests that verify the code's actual behaviour.

---

## 5. File Reading

All three analysis pipelines share a common document-reading function that
handles multiple file formats.

### 5.1 Supported Formats

The implementation MUST support the following file formats:

| Extension       | Method                              | Dependency        |
|-----------------|-------------------------------------|-------------------|
| `.pdf`          | `pdftotext -layout <file> -`        | poppler-utils     |
| `.docx`, `.doc` | `pandoc -t plain <file>`            | pandoc            |
| All others      | Direct filesystem read (UTF-8)      | None              |

**PDF handling.** The implementation MUST invoke `pdftotext` with the
`-layout` flag and read from stdout. If `pdftotext` is not installed, the
implementation MUST report a clear error message referencing `poppler-utils`.
If the extracted text is empty (image-only PDF), the implementation MUST
report that OCR is needed.

**DOCX handling.** The implementation MUST invoke `pandoc` with `-t plain`
to convert Word documents to plain text. If `pandoc` is not installed, the
implementation MUST report a clear error message.

**Text files.** All other files (plain text, Markdown, source code) MUST
be read directly via filesystem read.

### 5.2 Document Chunking

For thematic analysis of long documents, the implementation MUST chunk the
content into overlapping segments for AI processing.

- **Chunk size:** 8,000 characters maximum.
- **Overlap:** 500 characters between consecutive chunks to preserve context
  at boundaries.
- **Boundary snapping:** Chunk boundaries MUST be snapped to the nearest
  paragraph break (`\n\n`), sentence boundary (`.\n` or `. `), or word
  boundary. The implementation MUST NOT split in the middle of a UTF-8
  character.
- **Progress guarantee:** Each iteration MUST advance by at least half a
  chunk size to prevent infinite loops.
- **Short documents:** Documents that fit within a single chunk (8,000
  characters or fewer) MUST NOT be chunked.

For specification generation and test vector generation, the implementation
MUST truncate content exceeding 30,000 characters at a safe character
boundary rather than chunking.

### 5.3 Path Resolution

File paths MUST be resolved relative to the ANT's working directory. If a
path begins with `/`, it MUST be treated as absolute.

### 5.4 Security Restrictions

The `/analyse`, `/specify`, and `/test-vectors` commands read files from the
filesystem. To prevent unauthorised file access:

- These commands MUST only be permitted from the `web` dashboard or
  `system` sources.
- Requests from Telegram or Slack MUST be rejected with a message directing
  the user to the web dashboard.
- The implementation MUST check the source via `is_sensitive_allowed(source)`
  where only `"web"` and `"system"` return `true`.

---

## 6. Knowledge Graph Integration

This section describes how analysis results are written to the knowledge
graph, applicable to all three pipelines.

### 6.1 Node Types

Thematic analysis creates the following node kinds:

| Kind     | Created by           | Description                              |
|----------|----------------------|------------------------------------------|
| person   | Phase 2 (coding)     | A person mentioned in the source         |
| project  | Phase 2 (coding)     | A project or product                     |
| server   | Phase 2 (coding)     | A server or infrastructure component     |
| tool     | Phase 2 (coding)     | A tool, language, or technology          |
| concept  | Phases 2 and 3       | An abstract concept or theme             |
| decision | Phase 2 (coding)     | A decision or choice made                |
| event    | Phase 6 (integration)| Source document or temporal marker        |
| fact     | Phase 2 (coding)     | A factual statement                      |

### 6.2 Edge Metadata

Every edge created by thematic analysis MUST carry the following metadata,
as required by ANTHILL-THURISAZ:

- **basis** -- how the relationship was determined (`observed`, `inferred`,
  `assumed`, `told`).
- **confidence** -- initial probability derived from basis (see Section 2.6,
  item 5).
- **log_odds** -- `ln(p / (1 - p))` computed from confidence.
- **source_id** -- `"document:<source_name>"` for file analysis, or
  `"conversation <chat_id>"` for conversation compaction.
- **decay_category** -- based on content type: `fact`, `decision`,
  `observation`, `inference`, or `assumed`.
- **valid_from** -- today's date on all new edges.
- **survived** and **tests** -- refutation tracking counters.
- **justificatory_chain** -- at least one entry documenting the analysis
  process.

### 6.3 Topic Graph Structure

Each analysis run writes to a topic-specific graph file at
`memory/graphs/<topic>.json`. The topic name MUST be lowercase-hyphenated
and derived from the document's subject matter.

The implementation MUST also update the meta-graph at
`memory/knowledge.json` to register the topic graph as a concept node with
tags `["graph", "topic"]` and create edges to related existing topics.

### 6.4 Citation Linking

When the source material is a document (as opposed to a conversation), the
implementation SHOULD add a citation linking the source to the knowledge
graph. The citation MUST use the `graph_add_citation` mechanism defined in
ANTHILL-KNOWLEDGE.

---

## 7. Conformance

An implementation claiming conformance to ANTHILL-THEMATIC:

1. MUST implement the `/analyse` command with all six phases as described
   in Section 2.
2. MUST implement the `/specify` command as described in Section 3.
3. MUST implement the `/test-vectors` command as described in Section 4.
4. MUST support all file formats listed in Section 5.1.
5. MUST enforce the security restrictions in Section 5.4.
6. MUST produce knowledge graph output conforming to ANTHILL-KNOWLEDGE and
   ANTHILL-THURISAZ.
7. MUST chunk documents according to the parameters in Section 5.2.
8. MUST strip markdown code fences from AI output before JSON parsing.
9. SHOULD gracefully degrade when optional dependencies (`pdftotext`,
   `pandoc`) are unavailable, reporting clear error messages.

---

## 8. References

- Braun, V. & Clarke, V. "Thematic analysis: A practical guide." SAGE, 2022.
- Braun, V. & Clarke, V. "Using thematic analysis in psychology."
  *Qualitative Research in Psychology*, 3(2), 77--101, 2006.
- RFC 2119. Bradner, S. "Key words for use in RFCs to Indicate Requirement
  Levels." IETF, 1997.
- ANTHILL-KNOWLEDGE -- Knowledge Graph specification.
- ANTHILL-THURISAZ -- Thurisaz Engine specification.
- ANTHILL-WORKER -- AI Worker specification.
- ANTHILL-EXPORT -- Export & Reporting specification.
