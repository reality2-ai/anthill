# ANTHILL-EXPORT — Knowledge Export

**Version:** 0.1.0
**Status:** Implemented
**Depends on:** ANTHILL-MEMORY, ANTHILL-WEB

---

## 1. Overview

Anthill can export an ANT's knowledge as a **self-contained HTML file** that opens in any browser without a server. The export includes an AI-written narrative summary with citations, interactive 3D graph visualisation, and all graph data embedded as JSON.

Each export is a snapshot — a frozen-in-time view of what the ANT knows, how confident it is, and what sources support its beliefs.

---

## 2. Export Modes

| Mode | Trigger | Scope |
|------|---------|-------|
| Single graph | "Export" button on Graph tab | Currently selected topic graph |
| All graphs | "Export All" button on Graph tab | Meta-graph + all topic graphs |
| CLI | `anthill --export-graph --ant <name>` | All graphs for the named ANT |

---

## 3. Guidance Text

When exporting from the web dashboard, the user MAY provide optional **guidance text** to shape the AI report writer's output. Examples:

- "Focus on practical applications"
- "Write for a beginner audience"
- "Emphasise the gaps and what needs further investigation"
- "Compare the Swedish vocabulary and grammar sections"

The guidance is injected into the AI prompt as an additional directive. When no guidance is provided, the AI uses its default style.

---

## 4. Export Structure

The exported HTML file contains two tabs:

### 4.1 Insights Tab (default)

The **Insights** tab is shown by default when the export is opened. It contains:

1. **AI-written narrative summary** — the raw graph data is sent to an AI (via `claude -p`) which rewrites it as flowing prose. The prompt instructs the AI to:
   - Write for a general reader
   - Structure by topic with `##` headings
   - Include specific facts from the data
   - Cite sources using `[cite-xxxx]` codes (mandatory when citations exist)
   - Follow any user-provided guidance

2. **"View graph →" links** — each topic heading includes a link that switches to the Graph tab and selects the corresponding graph. Links are matched by comparing heading text against topic graph names.

3. **References section** — numbered citations in order of first appearance in the text. Each entry includes title, author, date, URL (clickable), and reference type.

If the AI is unavailable, a fallback algorithmic summary is generated from the graph statistics.

### 4.2 Graph Tab

Interactive 3D force-directed graph using ForceGraph3D + Three.js + SpriteText (all embedded in the HTML — no external dependencies). Features:

- Graph selector dropdown for switching between topics
- Node search by label, summary, or tags
- Node click for details panel (connections, confidence, basis)
- Camera zoom to clicked node
- Confidence-based node opacity and edge colouring

---

## 5. Citation Pipeline

Citations flow through the export as follows:

1. **Collection** — `compute_insights()` scans all edges across all graphs for their `citations` field. Each unique citation (by `cite_id` or URL) is collected into a deduplicated list.

2. **AI prompt** — the citation list is appended to the raw data sent to the AI, with instructions to cite sources using `[cite-xxxx]` codes inline.

3. **Renumbering** — after the AI produces text, all `[cite-xxxx]` codes are renumbered to `[1]`, `[2]`, etc. in order of first appearance.

4. **Reference list** — the ordered citations are rendered as an HTML `<ol>` with linked titles and metadata.

### 5.1 Citation Requirement

When citations exist in the graph data, the AI prompt includes a **mandatory citation directive**: every topic section must include at least one citation reference. Citations that the AI does not reference are still included in the reference list.

---

## 6. Self-Contained HTML

The export embeds all dependencies at compile time:

| Asset | Source |
|-------|--------|
| Three.js | `src/vendor/three.min.js` |
| three-spritetext | `src/vendor/three-spritetext.min.js` |
| 3d-force-graph | `src/vendor/3d-force-graph.min.js` |
| Graph data | Serialised as JSON in a `<script>` block |
| Insights HTML | Generated and inlined |

No external CDN requests. No server needed. The file can be shared, archived, or hosted anywhere.

---

## 7. GitHub Gist Publishing

When the GitHub CLI (`gh`) is installed and authenticated, exports are automatically published as public GitHub Gists. The gist URL is returned in the `X-Gist-Url` response header.

---

## 8. Snapshot Identity

Each export carries a unique snapshot ID (8-hex timestamp) and generation timestamp. These appear in the header and footer, making each export a citable, immutable point-in-time reference.
