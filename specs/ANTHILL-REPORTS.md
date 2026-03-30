# ANTHILL-REPORTS: Report Generation and Export Workflow

| Field      | Value                                                        |
|------------|--------------------------------------------------------------|
| Version    | 0.1 Draft                                                    |
| Date       | 2026-03-30                                                   |
| Status     | Draft                                                        |
| Depends on | ANTHILL-KNOWLEDGE, ANTHILL-DASHBOARD                         |
| Related    | ANTHILL-EXPORT, ANTHILL-THURISAZ                             |

> The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT",
> "SHOULD", "SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this
> document are to be interpreted as described in RFC 2119.

---

## 1. Introduction

Reports are AI-written narratives derived from an ANT's knowledge graphs.
They transform the structured data of conjectures, evidence, and
relationships into flowing prose suitable for human consumption. Each report
is a self-contained HTML document that includes:

- An **Insights tab** containing an AI-generated narrative summary with
  inline citations and confidence assessments.
- A **Graph tab** embedding an interactive 3D force-directed visualisation
  of the underlying knowledge graph (with 2D Canvas fallback).
- A **References section** listing all cited sources in order of first
  appearance, with clickable links.
- All graph data embedded as JSON, making the document fully self-contained
  and viewable offline in any browser.

Reports embody Anthill's Popperian epistemology: every claim is a
conjecture, confidence is earned through surviving refutation, and the
narrative distinguishes well-established ideas from those still under
investigation. The AI writer is instructed to produce academic-quality
prose with inline citations, not bullet-point summaries.

There are two paths to report generation:

1. **Background report task** -- initiated via the Export dialog in the
   dashboard or the `/report` chat command. Runs asynchronously as a
   worker task with progress events, stores the result persistently in
   the ANT's `reports/` directory, and posts a download link to chat on
   completion.

2. **Synchronous export** -- initiated via the `GET /api/ants/:id/export`
   endpoint or the `/export` chat command. Returns the HTML directly as a
   download. The `/export` chat command itself does not generate a file;
   it directs the user to the Export button or the `/report` command.

---

## 2. Export Dialog UX

The Export dialog is a modal overlay in the web dashboard, triggered by the
**Export** button in the Graph tab header.

### 2.1 Dialog Layout

The dialog MUST present the following controls:

1. **Scope selector** -- two toggle buttons:
   - **Current graph** -- exports only the graph currently selected in the
     graph-selector dropdown. The hint text MUST display the graph name
     (e.g. `Export "climate-policy" only.`).
   - **All knowledge** -- exports all topic graphs as a unified report.
     The hint text MUST read: "Export all topic graphs as a unified report
     -- not individual summaries, but an integrated whole."
   The default scope MUST be "All knowledge".

2. **Guidance prompt** -- a multi-line textarea (`<textarea rows="5">`)
   where the user provides instructions to the AI report writer. If empty
   on dialog open, it MUST be pre-filled with the default prompt:

   > Summarise the knowledge in this area, looking for practical insights
   > and solutions that would work in the real world. Highlight what is
   > well-established, what needs further investigation, and any surprising
   > connections between ideas.

   A helper line below the textarea MUST read: "This prompt is sent
   directly to the AI report writer. Be as specific or general as you
   like."

3. **Citations checkbox** -- a labelled checkbox, checked by default,
   reading "Include citations and references". When unchecked, the report
   omits citation collection and the References section.

4. **Action buttons** -- Cancel (closes dialog) and Export (submits).

### 2.2 Submission Behaviour

When the user clicks Export, the `submitExport()` function MUST:

1. Read the scope, guidance text, and citations checkbox state.
2. Resolve the graph name: if scope is "current" and a graph-selector
   exists, use its value; otherwise send an empty graph name (meaning all).
3. POST to `/api/ants/{id}/report` with a JSON body containing:
   - `graph` (string, optional) -- the graph name, omitted for all.
   - `guidance` (string, optional) -- the user's prompt.
   - `citations` (boolean) -- whether to include citations.
4. On success (HTTP 202), close the dialog and switch to the Workers tab
   so the user can observe task progress.
5. On failure, display an alert with the HTTP status and error text.

The Export button text MUST change to "Starting..." and become disabled
during the request.

---

## 3. Background Report Task

### 3.1 Task Lifecycle

The `spawn_report_task` function creates a background report generation
task. It accepts:

| Parameter          | Type                              | Description                                |
|--------------------|-----------------------------------|--------------------------------------------|
| `ant_id`           | String                            | ANT identifier                             |
| `memory_dir`       | PathBuf                           | Path to the ANT's `memory/` directory      |
| `reports_dir`      | PathBuf                           | Path to the ANT's `reports/` directory      |
| `display_name`     | String                            | Human-readable ANT name                    |
| `graph_filter`     | Option\<String\>                  | If set, export only this named graph       |
| `guidance`         | Option\<String\>                  | User's guidance prompt for the AI writer   |
| `include_citations`| bool                              | Whether to collect and render citations     |
| `event_tx`         | broadcast::Sender\<WsEvent\>     | Channel for task lifecycle events          |
| `chat_id`          | i64                               | Chat session to post completion message to |

The function MUST return a `task_id` (u32) derived from
`SystemTime::now().subsec_nanos()`.

### 3.2 Task Events

The task MUST emit the following WebSocket events via the broadcast
channel:

1. **TaskStarted** -- emitted immediately with a preview string:
   `"Generating report"` or `"Generating report: <graph_name>"`.

2. **TaskProgress** (kind: `"thinking"`) -- emitted twice:
   - `"Computing graph insights..."` -- before insight computation.
   - `"AI writing narrative summary..."` -- before AI invocation.

3. On success:
   - **Message** -- posted to the chat session with a markdown body:
     ```
     **Report ready** (<duration>s, <size>)

     [Download report](/api/ants/{id}/reports/{filename})

     _Self-contained HTML with interactive 3D graph -- open in any browser._
     ```
     File size MUST be formatted as MB (one decimal) if over 1,000,000
     bytes, otherwise as KB (zero decimals).
   - **TaskCompleted** -- with `duration_secs`.

4. On failure:
   - **TaskError** -- with `error: "Report failed: <message>"`.
   - **TaskCompleted** -- with `duration_secs`.

### 3.3 Execution Model

The task MUST run on `tokio::task::spawn_blocking` since it performs
synchronous file I/O and subprocess invocation (Claude CLI). The
`reports/` directory MUST be created with `create_dir_all` if it does not
exist.

---

## 4. Insight Computation

The `compute_insights` function analyses an array of graph data objects
(one per topic graph) and produces a `GraphInsights` structure.

### 4.1 Input Format

Each element in the input array is a JSON object with:

```json
{
  "name": "topic-name",
  "node_count": 42,
  "edge_count": 87,
  "data": {
    "nodes": [ { "id": 1, "label": "...", "kind": "concept", "summary": "...", "tags": [...], "confidence": 0.8 } ],
    "links": [ { "source": 1, "target": 2, "relation": "supports", "confidence": 0.75, "citations": [...] } ]
  }
}
```

### 4.2 Computed Metrics

The function MUST compute:

| Metric               | Description                                                                 |
|----------------------|-----------------------------------------------------------------------------|
| `total_nodes`        | Sum of node counts across all graphs.                                       |
| `total_edges`        | Sum of edge counts across all graphs.                                       |
| `avg_confidence`     | Mean confidence across all edges (excluding edges with relation `"?"`).     |
| `strongest_beliefs`  | Top 10 edges sorted by confidence (descending). Each entry is a tuple of (from_label, to_label, relation, confidence). |
| `weakest_beliefs`    | Bottom 10 edges sorted by confidence (ascending), filtered to confidence > 0.05 to exclude orphan links. |
| `most_connected`     | Top 10 nodes by connection count (both inbound and outbound).               |
| `topic_summaries`    | Per-graph summary: (name, node_count, edge_count, avg_confidence).          |
| `node_summaries`     | Map from node label to summary text (only nodes with summaries > 10 chars). |
| `topic_descriptions` | Map from graph name to the summary of the first hub/topic-tagged node.      |
| `all_citations`      | Deduplicated list of citations collected from edges (see Section 6).        |

### 4.3 Edge Filtering

Edges with relation `"?"` (undetermined) MUST be skipped entirely -- they
do not contribute to confidence calculations or belief rankings.

---

## 5. AI Summary Generation

### 5.1 Overview

The `ai_polish_summary` function takes raw insight text and rewrites it as
polished academic-quality prose using the Claude CLI. If the AI is
unavailable or produces insufficient output, it falls back to returning
the raw insights unchanged.

### 5.2 Prompt Construction

The prompt MUST be structured as follows:

1. **Role**: "You are writing an academic-quality report based on
   knowledge graph data for '{ant_name}'."

2. **Citation instructions** (always included):
   - If the raw insights contain `[cite-` codes: the AI MUST use those
     codes inline after every factual claim. The prompt specifies the
     exact format (`[cite-a1b2c3d4]`), requires every claim to have at
     least one citation, and states that a report without inline citations
     is "UNACCEPTABLE". The AI MUST NOT invent citation codes.
   - If no citation codes are present: the AI MUST distinguish between
     data-supported claims and its own synthesis using language like "the
     data indicates" versus "this appears likely".

3. **User task**: the guidance prompt provided by the user. If no guidance
   was provided, the default prompt (Section 2.1) is used.

4. **Formatting rules**: flowing prose (no bullet points or tables),
   markdown `##` headings for structure, specific facts from the data,
   integrated narrative (not per-topic summaries), third person referring
   to knowledge as belonging to the ANT.

5. **Knowledge data**: the raw text containing citations (if any),
   statistics, topic summaries, strongest/weakest beliefs, most connected
   concepts, and key entity descriptions.

### 5.3 Prompt Size Cap

The total prompt MUST NOT exceed 100,000 characters (~25,000 tokens). If
it exceeds this limit, the prompt MUST be truncated at the last newline
before the cap, with an appended note:

> [... data truncated for length -- focus on the topics and beliefs shown
> above ...]

### 5.4 Citation Budget

When citations are included, they are placed at the start of the knowledge
data section (before statistics and beliefs). Citations are sorted by
quality (highest first) and MUST NOT exceed 50,000 characters (half of
the 100K prompt budget). If the citation block exceeds this budget,
remaining citations are omitted with a count note:

> [N lower-quality sources omitted]

### 5.5 Claude CLI Invocation

The function MUST invoke the Claude CLI as follows:

```
claude -p --max-turns 1 --output-format text
```

The prompt MUST be piped via stdin (not as a command-line argument) to
support large inputs. The stdin pipe is closed after writing to signal
end of input.

### 5.6 Timeout and Fallback

The function MUST poll `child.try_wait()` in a loop with 500ms sleep
intervals. If the process does not complete within **5 minutes** (300
seconds), the function MUST:

1. Log a warning: "Claude export summary timed out after 300s -- killing".
2. Kill the child process.
3. Wait for the process to exit.
4. Fall back to the raw insights text.

The function MUST also fall back to raw insights if:

- The Claude CLI fails to spawn (logged as warning).
- The process exits with a non-success status.
- The output is shorter than 100 characters (indicating a trivial or
  error response).

---

## 6. Citation Handling

### 6.1 Citation Collection

During `compute_insights`, citations are extracted from the `citations`
array on each edge. Each citation object in the graph has the following
fields:

| Field      | Type   | Description                                      |
|------------|--------|--------------------------------------------------|
| `cite_id`  | String | Unique citation code (e.g. `cite-a1b2c3d4`)      |
| `url`      | String | Source URL                                        |
| `title`    | String | Source title                                      |
| `author`   | String | Author name(s)                                   |
| `date`     | String | Publication date                                 |
| `ref_type` | String | Reference type (e.g. `website`, `peer_reviewed`)  |
| `quality`  | f64    | Quality score (0.0--1.0)                          |

Additionally, each collected citation records:

- `supports` -- a string describing the relationship it supports, in the
  format `"<from> <relation> <to>"`.

### 6.2 Filtering Rules

The following citations MUST be excluded:

1. **AI-generated references** -- citations where `ref_type` (after
   lowercasing and removing underscores) is `"aiinference"` or
   `"aireference"`. These are internal AI annotations, not real sources.

2. **Duplicate URLs** -- deduplication is keyed by URL if non-empty,
   otherwise by `cite_id`. Only the first occurrence is kept.

3. **Broken URLs** -- each URL is verified via a HEAD request using curl
   with a 5-second timeout and redirect following (`-L`). URLs returning
   HTTP 4xx, 5xx, or timing out are skipped. A log message MUST be
   emitted for each skipped URL: `"[export] Skipping broken URL: <url>"`.

### 6.3 Citation ID Assignment

If a citation lacks a `cite_id`, one MUST be generated in the format
`cite-NNNN` where NNNN is a zero-padded hex counter based on the current
citation count (e.g. `cite-0001`, `cite-0002`).

### 6.4 Citation Renumbering

The `renumber_citations` function post-processes AI-generated text to
replace `[cite-xxxx]` codes with sequential numeric references `[1]`,
`[2]`, etc., in order of first appearance.

**Algorithm:**

1. Build a lookup map from `cite_id` to `CollectedCitation`.
2. Scan the text left-to-right for `[cite-` patterns.
3. For each match, extract the cite_id (text between `[` and `]`).
4. If this is the first occurrence, assign the next sequential number and
   record the corresponding citation in an ordered list.
5. Replace all instances of `[cite-xxxx]` with an HTML anchor:
   ```html
   <a href='#ref-N' style='color:#60a5fa;text-decoration:none;font-size:12px;vertical-align:super'>[N]</a>
   ```
   where N is the assigned sequential number.
6. Return the renumbered text and the ordered list of citations.

The ordered list drives the References section (Section 7.4), ensuring
references appear in the same order they are first cited in the narrative.

---

## 7. HTML Export Format

### 7.1 Self-Contained Document

The exported HTML MUST be fully self-contained. All JavaScript
dependencies are embedded at compile time via `include_str!`:

| Library                | Constant          | Purpose                         |
|------------------------|-------------------|---------------------------------|
| three.min.js           | `THREE_JS`        | 3D rendering engine             |
| three-spritetext.min.js| `SPRITETEXT_JS`   | Text labels on 3D nodes/links   |
| 3d-force-graph.min.js  | `FORCEGRAPH_JS`   | 3D force-directed graph layout  |
| force-graph.min.js     | `FORCEGRAPH_2D_JS`| 2D Canvas fallback              |

The document requires no network access and opens in any modern browser.

### 7.2 Document Structure

The HTML document contains:

1. **Header** -- title (ANT name), subtitle ("Knowledge Snapshot --
   {timestamp}"), tab buttons (Insights / Graph), a graph selector
   dropdown, and a node search input.

2. **Insights view** (`#insights-view`) -- the AI-written narrative (or
   algorithmic fallback), rendered as styled HTML.

3. **Graph view** (`#graph-view`) -- the interactive 3D force-directed
   graph (or 2D Canvas fallback when WebGL is unavailable). Hidden by
   default; shown when the Graph tab is selected.

4. **Info panel** (`#info`) -- a fixed overlay that appears on node click,
   showing the node's label, kind, summary, tags, and connections with
   confidence levels.

5. **Legend** (`#legend`) -- colour key for node kinds (person, project,
   tool, concept, decision, server, event, fact) and confidence levels
   (>=80% green, >=50% yellow, >=30% orange, <30% red).

6. **Footer** -- Anthill version, snapshot ID, and tagline.

### 7.3 Insights Tab Content

When AI polishing succeeds (output differs from raw input and exceeds 100
chars), the insights tab contains:

- A heading: "{ant_name} -- Knowledge Summary".
- A subtitle with snapshot ID and generation timestamp.
- The AI narrative converted from markdown to HTML: `#`/`##` become
  `<h3>`, `###` becomes `<h4>`, `**bold headings**` become `<h4>`, and
  all other lines become `<p>` elements. Inline `**bold**` is converted
  to `<b>` tags.
- Topic headings include a "View graph -->" link that switches to the
  Graph tab at the corresponding graph index.
- A closing note identifying the document as AI-generated from a
  Popperian knowledge graph.

When AI polishing fails, the algorithmic fallback (`render_insights_html`)
generates a structured narrative covering:

- An explanatory preamble about Popperian epistemology.
- Per-topic overviews with node/edge counts, confidence assessments, and
  density characterisations.
- Key concept descriptions drawn from node summaries.
- "What Is Well Established" -- the strongest beliefs with confidence
  percentages and node descriptions.
- "Areas Needing Further Investigation" -- the weakest beliefs.
- "Central Themes" -- the most connected concepts acting as knowledge
  hubs.

Confidence levels are described in natural language:

| Range       | Word         | Explanation                                              |
|-------------|--------------|----------------------------------------------------------|
| >= 70%      | strong       | most ideas have been rigorously tested and well-supported|
| >= 50%      | moderate     | core ideas supported but benefit from deeper investigation|
| >= 30%      | developing   | many ideas still being explored and tested               |
| < 30%       | early        | early exploration of the subject                         |

### 7.4 References Section

When citations are enabled and at least one citation exists, a References
section MUST be appended after the narrative. It consists of an ordered
list (`<ol>`) where each entry has an `id` attribute `ref-N` (matching the
inline `[N]` anchors).

Each reference entry is formatted as:

```
Author. (Year). *Title*. [Type badge] URL
```

Specifically:

- **Author** -- if present, followed by a period.
- **Date** -- if present, in parentheses followed by a period.
- **Title** -- rendered as `<em>`, linked to the URL if available.
  If no title exists, the raw URL is shown as the link.
- **Type badge** -- a normalised label in grey:

  | Normalised ref_type | Display label          |
  |---------------------|------------------------|
  | peerreviewed        | Peer-reviewed          |
  | book                | Book                   |
  | officialreport      | Official report        |
  | news                | News                   |
  | website             | Website                |
  | blog                | Blog                   |
  | personal            | Personal communication |
  | antknowledge        | ANT knowledge          |
  | other               | Reference              |

- **URL** -- if both title and URL exist, the URL is shown below in
  small grey text. If the citation lacks a URL, a search fallback is
  generated: Google Scholar for books and peer-reviewed papers, Google
  Search for everything else. Search-fallback links are annotated with
  "(search link)".

### 7.5 Embedded Graph Data

All graph data MUST be embedded in the HTML as a JavaScript constant:

```javascript
const ALL_GRAPHS = <json>;
```

This enables the client-side graph viewer to render any topic graph
without server access. The graph selector dropdown is populated from this
array, displaying each graph's name and node count. The first non-empty,
non-meta graph is selected by default.

### 7.6 Snapshot Identification

Each export MUST have a unique snapshot ID, computed as:

```rust
format!("{:08x}", SystemTime::now().duration_since(UNIX_EPOCH).as_secs())
```

This 8-character hex timestamp appears in the document title, subtitle,
and footer, providing an immutable permalink identifier.

---

## 8. Report Storage

### 8.1 Directory Layout

Reports are stored in the ANT's working directory under `reports/`:

```
<ant_working_dir>/
  reports/
    report-<uuid-prefix>.html
    report-<uuid-prefix>.html
    ...
```

The directory MUST be created with `create_dir_all` if it does not exist
when a report task starts.

### 8.2 Filename Format

Report filenames MUST follow the pattern:

```
report-<first-8-chars-of-uuid>.html
```

where the UUID is generated via `uuid::Uuid::new_v4()`. Example:
`report-a1b2c3d4.html`.

### 8.3 Persistence

Reports are persistent across server restarts. They are plain HTML files
stored on disk and are not managed by the knowledge graph or Git version
control.

### 8.4 Download Endpoint

Reports are served via:

```
GET /api/ants/{id}/reports/{filename}
```

The `download_report` handler MUST:

1. **Sanitize the filename** -- reject any filename containing `..`, `/`,
   or `\` with HTTP 400 Bad Request.
2. Look up the ANT in the registry; return 404 if not found.
3. Resolve the file path as `<working_dir>/reports/<filename>`.
4. Return 404 if the file does not exist.
5. Serve the file with:
   - `Content-Type: text/html; charset=utf-8`
   - `Content-Disposition: attachment; filename="<filename>"`

### 8.5 Synchronous Export Endpoint

The synchronous export endpoint is:

```
GET /api/ants/{id}/export
```

It accepts optional query parameters:

| Parameter   | Type   | Default | Description                           |
|-------------|--------|---------|---------------------------------------|
| `graph`     | String | (all)   | Export only the named graph            |
| `guidance`  | String | (none)  | Guidance prompt for the AI writer      |
| `citations` | String | `true`  | Set to `"false"` to omit citations     |

This endpoint generates the HTML in a temporary file, reads it into
memory, deletes the temporary file, and returns the content directly as
an HTTP response with `Content-Disposition: attachment`. The filename
follows the pattern `<ant_id>-<graph>-<uuid>.html` (with graph) or
`<ant_id>-<uuid>.html` (all graphs).

Unlike the background report task, this endpoint does NOT store the
result persistently and does NOT emit task lifecycle events.

---

## 9. /export and /report Chat Commands

### 9.1 /export Command

The `/export` chat command is informational only. It MUST return a
message directing the user to the Export button or the `/report` command:

> Click the **Export** button in the Graph tab to download a shareable
> HTML snapshot of this ANT's knowledge graph. The file opens in any
> browser -- no server needed.
>
> Or use `/report [guidance]` to generate a report as a background task.

### 9.2 /report Command

The `/report [guidance]` chat command triggers a background report task.

**Behaviour:**

1. Parse optional guidance text from the command (everything after
   `/report`, trimmed, empty strings filtered out).
2. Resolve `memory_dir` and `reports_dir` from the ANT handle.
3. Call `spawn_report_task` with:
   - `graph_filter`: `None` (always exports all graphs).
   - `guidance`: the parsed guidance text, or `None`.
   - `include_citations`: `true` (always enabled via chat command).
4. Return an immediate acknowledgement:

   > Generating report in the background...
   >
   > The download link will appear in chat when ready. You can leave
   > this page.

The download link is delivered asynchronously via the TaskStarted /
TaskProgress / Message event sequence described in Section 3.2.

---

## 10. Conformance

An implementation claiming conformance to ANTHILL-REPORTS:

1. MUST implement the background report task lifecycle (Section 3) with
   all specified WebSocket events.

2. MUST implement insight computation (Section 4) with all specified
   metrics and edge filtering rules.

3. MUST implement AI summary generation (Section 5) with the specified
   prompt structure, 100K character cap, 50K citation budget, 5-minute
   timeout, and fallback behaviour.

4. MUST implement citation collection (Section 6) with AI-reference
   filtering, URL deduplication, broken-URL verification, and sequential
   renumbering.

5. MUST produce self-contained HTML exports (Section 7) with embedded
   JavaScript, dual Insights/Graph tabs, and all graph data as JSON.

6. MUST implement report storage (Section 8) with UUID-based filenames,
   path-traversal sanitization on download, and persistent storage.

7. MUST implement the `/export` and `/report` chat commands as specified
   in Section 9.

8. The export dialog (Section 2) is REQUIRED for implementations that
   include the web dashboard (ANTHILL-DASHBOARD).

9. Support for the synchronous export endpoint (Section 8.5) is
   RECOMMENDED but OPTIONAL.

10. Implementations MAY substitute a different AI backend for the Claude
    CLI, provided the prompt structure, timeout, and fallback behaviour
    are preserved.

---

## 11. References

- ANTHILL-KNOWLEDGE -- Knowledge Graph specification (graph schema, node
  and edge types).
- ANTHILL-DASHBOARD -- Web Dashboard specification (real-time channels,
  graph visualisation).
- ANTHILL-THURISAZ -- Thurisaz Engine specification (Bayesian confidence,
  evidence diversity).
- ANTHILL-EXPORT -- Export and Reporting overview (specification suite
  entry in ANTHILL-INTRO).
- RFC 2119 -- Key words for use in RFCs to Indicate Requirement Levels.
