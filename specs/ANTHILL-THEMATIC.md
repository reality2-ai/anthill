# ANTHILL-THEMATIC: Thematic Analysis Pipeline

**Version:** 0.1 Draft
**Date:** 2026-03-20
**Status:** Draft
**Depends on:** ANTHILL-MEMORY, ANTHILL-WORKER

---

## 1. Introduction

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT", "SHOULD", "SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this document are to be interpreted as described in RFC 2119.

This specification defines a pipeline for converting documents into knowledge graph nodes and edges using **reflexive thematic analysis** (Braun & Clarke, 2006, 2022). The pipeline is AI-driven: the AI backend performs the actual analysis at each phase, while Anthill provides structure, prompts, and parsing.

### 1.1 Background: Thematic Analysis

Thematic Analysis (TA) is a qualitative research method for identifying patterns of meaning across datasets. Braun & Clarke's reflexive TA positions the researcher's interpretation as central — not incidental — to the analysis. This aligns with Anthill's Popperian approach: the AI's interpretation is a conjecture, not ground truth.

### 1.2 Why Thematic Analysis for Documents?

Simple entity extraction ("find all names and places") misses the meaning. Thematic analysis captures:

- **What entities exist** (codes) — people, projects, tools, concepts
- **How they relate** (relationships) — structural, causal, temporal
- **What patterns emerge** (themes) — higher-level concepts that group related entities
- **How well-supported each finding is** (confidence) — grounded in evidence from the source

The output is not a keyword index. It's a structured, confidence-weighted knowledge graph that reflects the document's meaning.

---

## 2. Pipeline Phases

The pipeline follows Braun & Clarke's six phases, adapted for AI-driven document analysis:

### Phase 1: Familiarisation

**Input:** Raw document text.
**Process:** Chunk the document into overlapping segments (default: 8000 chars, 500 char overlap). Break at paragraph or sentence boundaries where possible.
**Output:** Array of text chunks.

Chunking is necessary because AI backends have context limits. Overlap preserves cross-boundary context.

### Phase 2: Coding

**Input:** Each text chunk (processed sequentially).
**Process:** The AI extracts **codes** — entities, concepts, decisions, facts — from each chunk.
**Output:** Array of `Code` objects.

A code has:

| Field | Type | Description |
|-------|------|-------------|
| `label` | string | Short identifier (e.g. "Ed25519 signing") |
| `kind` | string | Node kind: person, project, server, tool, concept, decision, event, fact |
| `description` | string | One-sentence explanation |
| `evidence` | string | Source excerpt supporting this code |
| `tags` | string[] | Searchable keywords |

Codes from all chunks are collated. Duplicates (same label) are merged.

### Phase 3: Theme Generation

**Input:** All codes from Phase 2.
**Process:** The AI groups codes into **themes** — broader patterns of shared meaning.
**Output:** Array of `Theme` objects.

A theme has:

| Field | Type | Description |
|-------|------|-------------|
| `name` | string | Concise theme name |
| `concept` | string | Central idea unifying the member codes |
| `codes` | string[] | Labels of codes belonging to this theme |
| `support` | f64 | How well-supported (0.0–1.0), based on evidence count and clarity |

### Phase 4: Review

**Input:** Themes + codes + original chunks.
**Process:** Implicit in the AI's Phase 3 work — the prompt asks the AI to assess support levels. In a multi-pass implementation, this phase would re-read the source to verify themes.
**Output:** Updated support scores on themes.

### Phase 5: Refinement

**Input:** Codes + themes.
**Process:** The AI identifies **relationships** between entities.
**Output:** Array of `Relationship` objects.

A relationship has:

| Field | Type | Description |
|-------|------|-------------|
| `from` | string | Source entity label |
| `to` | string | Target entity label |
| `relation` | string | Relationship type (uses, deployed_on, depends_on, etc.) |
| `context` | string | Brief description |
| `basis` | string | How determined: "observed" (explicit), "inferred" (implied), "assumed" (interpretation) |

### Phase 6: Integration

**Input:** Codes, themes, relationships.
**Process:** The AI integrates results into the knowledge graph (`knowledge.json`):

1. Each code becomes a node (or updates an existing node).
2. Each theme becomes a concept node, with edges from member codes.
3. Each relationship becomes an edge with Popperian metadata:
   - `basis: "observed"` → initial confidence 0.7
   - `basis: "inferred"` → initial confidence 0.4
   - `basis: "assumed"` → initial confidence 0.3
4. The source document is added as an event node.
5. Existing nodes are updated, not duplicated.

---

## 3. Confidence Mapping

Thematic analysis naturally produces confidence signals:

| TA Signal | Popperian Mapping |
|-----------|-------------------|
| Explicit statement in text | `basis: "observed"`, confidence 0.7 |
| Implied by multiple codes | `basis: "inferred"`, confidence 0.4 |
| Analyst's interpretation | `basis: "assumed"`, confidence 0.3 |
| Theme support level | Mapped to importance of theme edges |
| Evidence quote provided | Higher confidence (corroborated) |
| Single mention, no corroboration | Lower confidence |

The reflexive nature of TA acknowledges that all coding is interpretive. This aligns perfectly with the Popperian model: every extraction is a conjecture, subject to future refutation.

---

## 4. Document Types

The pipeline SHOULD handle:

| Format | Handling |
|--------|----------|
| Plain text (.txt) | Direct chunking |
| Markdown (.md) | Direct chunking (structure preserved) |
| PDF | Extract text first (future: pdf-to-text utility) |
| Web pages | Fetch and convert to markdown first |
| Source code | Treat as text; coding phase extracts architecture, dependencies, patterns |

---

## 5. Multi-Document Analysis

When multiple documents are analysed sequentially, later documents benefit from the existing knowledge graph:

1. The AI sees the current graph context in its prompt.
2. New codes are checked against existing nodes — matches update rather than duplicate.
3. New relationships may strengthen or contradict existing conjectures.
4. Themes may evolve as more documents are processed.

This is the recursive nature of reflexive TA — each new document is interpreted in the context of everything learned so far.

---

## 6. Implementation

### 6.1 Chunking

Documents are split into overlapping chunks:
- Default chunk size: 8000 characters
- Overlap: 500 characters
- Break points: paragraph (`\n\n`), sentence (`.\n`, `. `), or hard limit
- Progress guaranteed: advance at least half a chunk per step

### 6.2 AI Prompts

Each phase has a specific prompt template (see `src/thematic.rs`):
- Phase 2 prompt: asks for JSON array of codes
- Phase 3 prompt: asks for JSON array of themes from codes
- Phase 5 prompt: asks for JSON array of relationships from codes + themes
- Phase 6 prompt: instructs the AI to integrate into knowledge.json

### 6.3 Output Parsing

AI output is parsed tolerantly:
- Markdown code fences (` ```json `) are stripped
- JSON arrays are parsed; malformed output results in an empty array (not a crash)
- Individual parse errors are logged but don't stop the pipeline

---

## 7. Spec Generation Pipeline

### 7.1 `/specify <file>` — Formal Specification

Generates an RFC 2119-style specification from source code. The pipeline follows the same six-phase thematic analysis pattern:

1. **Familiarise** — read and chunk the source file
2. **Code** — extract behaviours, invariants, data structures, and interfaces
3. **Theme** — group codes into specification sections (lifecycle, protocol, error handling, etc.)
4. **Review** — validate extracted behaviours against the source
5. **Refine** — produce formal MUST/SHOULD/MAY statements with rationale
6. **Integrate** — write the spec file to `specs/` and update the knowledge graph

Output is a Markdown specification with normative language (RFC 2119 keywords).

### 7.2 `/test-vectors <file>` — Test Case Generation

Generates test cases from source code or an existing specification. Uses the same six-phase pipeline:

1. **Familiarise** — read and chunk the input file
2. **Code** — extract testable behaviours, edge cases, invariants
3. **Theme** — group into test categories (happy path, error handling, boundary conditions, etc.)
4. **Review** — validate coverage against the source
5. **Refine** — produce concrete test vectors with inputs, expected outputs, and rationale
6. **Integrate** — output test cases and Rust `#[test]` stubs

Both commands update the knowledge graph with extracted entities and relationships, providing the same Popperian confidence metadata as `/analyse`.

---

## 8. Security Considerations

1. **Document content is sent to external AI backends.** Users SHOULD be aware that analysed documents are processed by Claude Code, Codex, etc.
2. **Extracted knowledge persists** in the knowledge graph and is included in future prompts. Sensitive content in documents will appear in the graph.
3. **Evidence fields** may contain verbatim quotes from the source. These are stored in the codes (in memory during analysis) but not persisted to the knowledge graph — only the distilled nodes and edges are.
