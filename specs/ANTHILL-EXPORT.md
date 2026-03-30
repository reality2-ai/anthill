# ANTHILL-EXPORT: Self-Contained HTML Export Format

| Field      | Value                                                        |
|------------|--------------------------------------------------------------|
| Version    | 0.1 Draft                                                    |
| Date       | 2026-03-30                                                   |
| Status     | Draft                                                        |
| Depends on | ANTHILL-KNOWLEDGE, ANTHILL-THURISAZ                          |
| Related    | ANTHILL-REPORTS, ANTHILL-GRAPH-UX                            |

> The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT",
> "SHOULD", "SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this
> document are to be interpreted as described in RFC 2119.

---

## 1. Introduction

An ANT's knowledge graph is valuable only if it can be shared, reviewed, and
archived. ANTHILL-EXPORT defines a self-contained HTML export format that
captures the complete state of an ANT's knowledge at a point in time and
presents it in a form that any person can read in any modern web browser with
no server, no installation, and no network connection beyond first load.

Each export produces a single `.html` file containing:

1. An AI-written narrative with inline numbered citations.
2. An interactive 3D knowledge graph matching the dashboard rendering.
3. A numbered reference list with URLs, authors, dates, and quality metadata.
4. Graph data embedded as JSON for client-side exploration.

The file is a snapshot. It captures the state of knowledge at export time and
does not change. A UUID v4 identifies each snapshot immutably.

---

## 2. HTML Structure

### 2.1 Document Layout

The exported HTML document MUST contain the following structural elements in
order:

1. **Header bar** -- ANT name, export timestamp, tab buttons, graph selector
   dropdown, and node search input.
2. **Tab bar** -- two tabs: "Insights" and "Graph" (or "Graphs" when multiple
   topic graphs are present). The Insights tab MUST be active on load.
3. **Insights view** -- the AI-written narrative (or algorithmic fallback),
   displayed within a styled container at a maximum width of 900 pixels.
4. **Graph view** -- an interactive ForceGraph3D canvas occupying the full
   viewport width and 80% of the viewport height. Hidden by default; shown
   when the user selects the Graph tab.
5. **Node info panel** -- a fixed-position overlay (bottom-left) that appears
   when a user clicks a graph node, showing label, kind, summary, tags, and
   connected edges with confidence scores.
6. **Legend panel** -- a fixed-position overlay (top-right) showing node-type
   colour key and confidence colour bands. Visible only in graph view.
7. **Footer** -- Anthill version, snapshot ID, and link to the project
   repository.

### 2.2 Insights Tab

The insights tab MUST contain:

- **Title**: `{ant_name} -- Knowledge Summary`.
- **Subtitle**: snapshot ID and generation timestamp.
- **Narrative body**: AI-written prose (see Section 5) or algorithmic fallback
  (see Section 4). Formatted as flowing paragraphs with `<h3>` and `<h4>`
  section headings. Inline citations appear as superscript numbered links
  (`[1]`, `[2]`, ...) that anchor to the corresponding entry in the reference
  list.
- **Provenance note**: a closing paragraph explaining that the summary was
  generated from a Popperian knowledge graph where every idea must earn its
  confidence through surviving genuine challenges.

### 2.3 Graph Tab

The graph tab MUST render the same force-directed 3D visualisation used in the
web dashboard:

- **Node colour** is determined by node kind (`person`, `project`, `tool`,
  `concept`, `decision`, `server`, `event`, `fact`, `theory`, `mechanism`,
  `principle`, `constraint`), using the standard Anthill colour palette.
- **Node opacity** is proportional to confidence, with a minimum alpha of
  0.15.
- **Node size** is larger for hub nodes (6) than non-hub nodes (3).
- **Edge width** is proportional to confidence (minimum 0.5), with orphan
  links drawn thinner (0.3).
- **Edge colour** follows confidence bands: green (>=80%), amber (>=50%),
  orange (>=30%), red (<30%).
- **Text labels** on nodes and edges SHOULD be rendered using SpriteText when
  available.
- **Directional arrows** MUST be present on all edges.
- When WebGL is unavailable, the renderer MUST fall back to a 2D canvas
  implementation using `ForceGraph` (force-graph.min.js).

### 2.4 References Section

When citations are present, the insights view MUST be followed by a numbered
reference list inside its own styled container. Each entry MUST include:

1. **Sequential number** matching the inline citation (`[1]`, `[2]`, ...).
2. **Author** (if available), followed by a period.
3. **Date/year** in parentheses (if available).
4. **Title** as an italic hyperlink to the source URL (if available), or
   plain italic text if no URL exists.
5. **Reference type badge** in square brackets: Peer-reviewed, Book, Official
   report, News, Website, Blog, Personal communication, ANT knowledge, or
   Reference.
6. **Direct URL** displayed as small text below the title when the entry has
   both a title and a URL.
7. **Search link indicator** when no URL is available but a Google Scholar or
   Google search link has been constructed from the title and author.

Each `<li>` element MUST have an `id` attribute of `ref-{N}` (where N is the
1-based citation number) so that inline citation links can anchor to it.

### 2.5 Metadata

The following metadata MUST be present in the exported document:

- ANT name (displayed in header and title).
- Export timestamp (displayed in header subtitle and insights subtitle).
- Snapshot ID (a hex-encoded UNIX timestamp, displayed in title and footer).
- Graph statistics: number of topic graphs, total nodes, total edges.
- Anthill version (from `CARGO_PKG_VERSION`, displayed in footer).

### 2.6 Graph Selector and Search

- A `<select>` element MUST list all embedded graphs by name and node count.
  The default selection MUST be the first non-empty, non-meta graph.
- A text input MUST provide client-side search across node labels, summaries,
  and tags. Matching nodes retain full colour; non-matching nodes dim to
  `rgba(100,100,100,0.1)`. Clearing the search restores all node colours.

---

## 3. Graph Data Embedding

All graph data MUST be embedded in the HTML document as a JSON array assigned
to a JavaScript constant `ALL_GRAPHS` within a `<script>` tag.

### 3.1 Data Shape

Each element of `ALL_GRAPHS` is an object with the following fields:

| Field        | Type   | Description                                            |
|--------------|--------|--------------------------------------------------------|
| `name`       | string | Topic graph name (e.g. `"climate-science"`)            |
| `node_count` | number | Number of nodes in this graph                          |
| `edge_count` | number | Number of edges in this graph                          |
| `data`       | object | Visualisation payload with `nodes` and `links` arrays  |

The `data.nodes` array contains objects with at least: `id`, `label`, `kind`,
`summary`, `tags`, `confidence`, and `is_hub`.

The `data.links` array contains objects with at least: `source`, `target`,
`relation`, `confidence`, `basis`, `is_orphan_link`, and `citations`.

### 3.2 Vendor Libraries

The following JavaScript libraries MUST be embedded directly in the HTML file
via `include_str!` at compile time, making the export fully self-contained:

- `three.min.js` -- Three.js 3D rendering engine.
- `three-spritetext.min.js` -- SpriteText for 3D text labels.
- `3d-force-graph.min.js` -- ForceGraph3D force-directed layout.
- `force-graph.min.js` -- ForceGraph 2D canvas fallback.

An implementation MUST NOT require network access to render the graph. All
dependencies are bundled at compile time from `src/vendor/`.

---

## 4. Insight Computation

The `compute_insights` function analyses all graph data and produces a
`GraphInsights` structure. The computation MUST extract the following:

### 4.1 Aggregate Statistics

- **Total nodes** and **total edges** across all topic graphs.
- **Average confidence** across all edges (edges with undetermined relations
  are excluded).

### 4.2 Strongest Beliefs

The top 10 edges sorted by descending confidence. Each entry records source
label, target label, relation name, and confidence score. These represent
relationships that have earned high confidence through surviving diverse
refutation.

### 4.3 Knowledge Gaps (Weakest Beliefs)

The bottom 10 edges sorted by ascending confidence, excluding edges with
confidence below 0.05 (orphan links). These represent ideas still at an early
stage of investigation.

### 4.4 Most Connected Concepts

The top 10 nodes by connection count (sum of inbound and outbound edges).
These act as hubs that tie the knowledge together.

### 4.5 Topic Summaries

For each topic graph: name, node count, edge count, and average confidence.
Empty topic graphs (zero nodes) are excluded from the narrative but retained
in the list.

### 4.6 Node Summaries

A map from node label to summary text, collected from all nodes that have a
non-empty summary longer than 10 characters. The first summary encountered
for a given label is kept.

### 4.7 Topic Descriptions

A map from topic graph name to a description derived from the first node
tagged `hub` or `topic` that has a summary.

### 4.8 Citation Collection

All citations are collected from edge `citations` arrays across all topic
graphs, deduplicated by URL (or by `cite_id` when no URL is present). Each
collected citation records:

| Field      | Type   | Description                                              |
|------------|--------|----------------------------------------------------------|
| `cite_id`  | string | Unique code, e.g. `cite-0001`. Auto-generated if absent |
| `url`      | string | Source URL                                               |
| `title`    | string | Source title                                             |
| `author`   | string | Author name(s)                                           |
| `date`     | string | Publication date                                         |
| `ref_type` | string | Reference type (website, book, peer_reviewed, etc.)      |
| `quality`  | f64    | Source quality score (0.0 to 1.0)                        |
| `supports` | string | The relationship this citation supports (`A rel B`)      |

Citations with `ref_type` of `ai_inference` or `ai_reference` (normalised,
case-insensitive) MUST be excluded -- these are internal AI-generated
references, not real citations.

### 4.9 URL Verification

Before including a citation, the implementation SHOULD verify that the URL is
reachable by issuing an HTTP HEAD request with a 5-second timeout and
following redirects. URLs that respond with 4xx, 5xx, or timeout SHOULD be
skipped with a log message. Empty URLs bypass verification.

---

## 5. AI Narrative Generation

When an AI backend is available, the implementation SHOULD invoke it to
rewrite the raw algorithmic insights as polished, flowing prose. The function
`ai_polish_summary` handles this process.

### 5.1 Invocation

The Claude CLI MUST be invoked as a subprocess:

```
claude -p --max-turns 1 --output-format text
```

The prompt MUST be piped via stdin (not passed as a command-line argument) to
support large inputs.

### 5.2 Prompt Construction

The prompt sent to the AI MUST contain the following sections in order:

1. **Role statement**: "You are writing an academic-quality report based on
   knowledge graph data for '{ant_name}'."
2. **Citation instructions** (see Section 5.3).
3. **User guidance**: the user's custom prompt if provided, otherwise a
   default asking for practical insights, well-established findings, areas
   needing investigation, and surprising connections.
4. **Formatting rules**: flowing prose (no bullet points, no tables, no graph
   jargon), markdown `##` headings, specific facts, unified narrative in third
   person.
5. **Raw insights data**: citations listed first (highest quality first),
   followed by aggregate statistics, topic summaries, strongest beliefs,
   weakest beliefs, most connected concepts, and key entity descriptions.

### 5.3 Citation Instructions

When the raw data contains `[cite-xxxx]` codes, the prompt MUST instruct the
AI to:

- Place citation codes inline after every factual claim.
- Use the format `[cite-a1b2c3d4]` exactly as provided.
- Mark unsupported claims explicitly as "(no source available)".
- Use as many of the provided citations as are relevant.
- Never invent citation codes not in the provided list.

When no citation codes are present, the prompt MUST instruct the AI to
distinguish between data-supported claims and its own synthesis using
qualifying language.

### 5.4 Prompt Size Cap

The prompt MUST be capped at 100,000 characters (approximately 25,000
tokens). If the prompt exceeds this limit, it MUST be truncated at the last
newline before the cap and a notice appended:
`[... data truncated for length -- focus on the topics and beliefs shown above ...]`

Citations are placed first in the raw data so they are never truncated. The
citation section itself is capped at 50,000 characters (half the prompt
budget), with lower-quality sources omitted if necessary.

### 5.5 Timeout and Failure

The AI subprocess MUST be given a maximum of 5 minutes (300 seconds). The
implementation MUST poll using `try_wait` at 500ms intervals. If the timeout
elapses, the process MUST be killed and waited on.

The AI result is accepted only if the process exits successfully and the
output exceeds 100 characters.

### 5.6 Fallback

If the AI is unavailable, times out, or produces insufficient output, the
implementation MUST fall back to the algorithmic `render_insights_html`
output. This fallback produces structured prose covering:

- An explanatory preamble about Popperian epistemology.
- An overview with total concepts, relationships, and confidence assessment.
- Per-topic narratives with concept counts, density analysis, and key entity
  descriptions.
- "What Is Well Established" section with the strongest beliefs.
- "Areas Needing Further Investigation" section with the weakest beliefs.
- "Central Themes" section with the most connected concepts.

### 5.7 Markdown to HTML Conversion

AI output is treated as markdown-like text and converted to HTML:

- Lines starting with `# `, `## `, or `### ` become `<h3>` or `<h4>` elements.
- Lines consisting entirely of `**bold text**` become `<h4>` elements.
- All other non-empty lines become `<p>` elements.
- Inline `**bold**` markers are converted to `<b>` tags.
- Headings that match a topic graph name receive an appended "View graph"
  link that switches to the graph tab and selects the corresponding graph.

---

## 6. Citation Renumbering

After the AI produces its narrative, the `renumber_citations` function MUST
post-process the text to convert internal citation codes to sequential
reader-friendly numbers.

### 6.1 Algorithm

1. **Scan** the AI text from start to end for all occurrences of the pattern
   `[cite-XXXX]` (where XXXX is any string).
2. **Assign** a sequential number to each unique cite code in order of first
   appearance: the first code encountered becomes `[1]`, the second `[2]`,
   and so on.
3. **Replace** each `[cite-XXXX]` in the text with a clickable superscript
   HTML anchor: `<a href='#ref-N' ...>[N]</a>`.
4. **Build** an ordered reference list where the Nth entry corresponds to the
   citation whose code was assigned number N.

### 6.2 Unmatched Codes

If a `[cite-XXXX]` code appears in the AI text but does not match any
citation in the collected citations list, it is still assigned a sequential
number and replaced in the text. However, no reference entry is generated for
it. This ensures the narrative is never broken by missing data.

### 6.3 Reference Ordering

When the renumbering produces an ordered list, the reference section MUST use
that list (in order of first appearance in the narrative). When no
renumbering occurs (fallback mode), the reference section MUST use the
original `all_citations` list in collection order.

---

## 7. Single Graph vs All Graphs

The export system provides two entry points:

### 7.1 `export_single_graph`

Exports one named topic graph. The implementation MUST:

1. Load the specified graph from the `LiveKnowledgeStore`.
2. Build a data array containing exactly one graph entry.
3. Set the document title to `{ant_name} -- {graph_name}` (with hyphens
   replaced by spaces).
4. Pass the data to `generate_export_html`.

### 7.2 `export_ant_graphs`

Exports all topic graphs for an ANT as a unified document. The implementation
MUST:

1. List all graphs from the `LiveKnowledgeStore`.
2. Exclude internal graphs: `citations` and `uncategorised`.
3. Build a data array containing all remaining non-empty graphs.
4. Set the document title to `{ant_name}`.
5. Pass the data to `generate_export_html`.

In both modes, the AI narrative covers all included graphs as an integrated
whole -- not as separate per-graph summaries.

### 7.3 Parameters

Both entry points accept the following parameters:

| Parameter           | Type          | Description                                    |
|---------------------|---------------|------------------------------------------------|
| `memory_dir`        | path          | Path to the ANT's memory directory             |
| `ant_name`          | string        | Display name of the ANT                        |
| `output_path`       | path          | Destination file path for the HTML output      |
| `guidance`          | Option<&str>  | User's custom prompt to guide the AI narrative |
| `include_citations` | bool          | Whether to collect and display citations        |

---

## 8. UUID and Immutability

Each export MUST be assigned a unique identifier derived from the UNIX
timestamp at generation time, formatted as an 8-character zero-padded
hexadecimal string (e.g. `67e8a1b0`).

The snapshot ID MUST appear in:

- The HTML `<title>` element.
- The header subtitle.
- The insights subtitle.
- The footer.

The exported HTML file is a snapshot of the ANT's knowledge at the moment of
export. It MUST NOT contain any mechanism to update itself, fetch new data, or
connect to the Anthill server. The file is fully self-contained and immutable
once written.

Upon successful export, the implementation MUST print to stdout:

- The ANT name, output path, file size in KB, and snapshot ID.
- The number of graphs, total nodes, and total edges.
- A notice that the file opens in any browser with no server needed.
- The available tabs (Graph and Insights).

---

## 9. Conformance

An implementation claiming conformance to ANTHILL-EXPORT MUST:

1. Produce a single self-contained HTML file that renders in any modern
   browser without network access (after first load of vendor JS, which is
   embedded at compile time).
2. Embed all graph data as JSON in a `<script>` tag.
3. Provide both an Insights tab and a Graph tab with the structure defined in
   Section 2.
4. Compute insights as specified in Section 4.
5. Attempt AI narrative generation as specified in Section 5, falling back to
   algorithmic output on failure.
6. Renumber citation codes to sequential numbers as specified in Section 6.
7. Support both single-graph and all-graphs export modes as specified in
   Section 7.
8. Assign a unique snapshot identifier as specified in Section 8.

AI narrative generation (Section 5) is RECOMMENDED but not REQUIRED. An
implementation that omits AI polishing MUST still produce the algorithmic
fallback narrative.

Citation collection and URL verification (Section 4.8, 4.9) are RECOMMENDED.
An implementation MAY omit citations entirely by setting `include_citations`
to false.

---

## 10. References

- ANTHILL-KNOWLEDGE -- Knowledge graph schema, node/edge types, CBOR encoding.
- ANTHILL-THURISAZ -- Bayesian confidence engine, evidence diversity scoring.
- ANTHILL-GRAPH-UX -- Graph visualisation conventions and colour palette.
- ForceGraph3D -- <https://github.com/vasturiano/3d-force-graph>
- Three.js -- <https://threejs.org/>
- RFC 2119. Bradner, S. "Key words for use in RFCs to Indicate Requirement
  Levels." IETF, 1997.
