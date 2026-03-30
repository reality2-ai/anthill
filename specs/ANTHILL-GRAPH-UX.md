# ANTHILL-GRAPH-UX: Knowledge Graph Visualisation and Interaction

| Field      | Value                                                        |
|------------|--------------------------------------------------------------|
| Version    | 0.1 Draft                                                    |
| Date       | 2026-03-30                                                   |
| Status     | Draft                                                        |
| Depends on | ANTHILL-KNOWLEDGE, ANTHILL-DASHBOARD                         |
| Related    | ANTHILL-REPORTS, ANTHILL-THURISAZ                            |

> The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT",
> "SHOULD", "SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this
> document are to be interpreted as described in RFC 2119.

---

## 1. Introduction

The knowledge graph is visualised as an interactive 3D force-directed graph
using ForceGraph3D (Three.js). Users can explore, query, and update
knowledge directly from the graph view.

The graph view is one of the primary tabs in the Anthill web dashboard. It
renders an ANT's topic graphs and meta-graph as a navigable 3D scene,
providing visual feedback on knowledge structure, confidence levels, and
inter-concept relationships. A 2D fallback renderer MUST be provided when
WebGL is unavailable.

---

## 2. Graph Selector

A `<select>` dropdown MUST be displayed in the toolbar at the top-left of
the graph panel, listing all available topic graphs for the active ANT plus
the meta-graph.

### 2.1 Population

When a graph is loaded, the API response includes an `available_graphs`
array and a `current_graph` string. The implementation MUST populate the
dropdown from `available_graphs`, marking the entry matching `current_graph`
as selected. The meta-graph entry MUST be labelled `"Meta-graph (index)"`.

### 2.2 Loading

Selecting a graph from the dropdown MUST trigger a load via:

```
GET /api/ants/{id}/graph?name=<topic>
```

where `{id}` is the active ANT identifier and `<topic>` is the selected
graph name. If the `name` parameter is empty or `"meta"`, the endpoint
MUST return the meta-graph.

### 2.3 Default Behaviour

If the meta-graph is empty but topic graphs exist, the implementation MUST
automatically load the first available topic graph (excluding `"meta"`).
If no graphs exist at all, a placeholder message MUST be displayed:
*"No knowledge graph yet. Chat with the ANT to build one."*

### 2.4 Tab Label

The Graph tab button MUST display `"Graphs"` (plural) when more than one
graph is available, and `"Graph"` (singular) otherwise.

---

## 3. Node Rendering

### 3.1 Colour by Kind

Each node MUST be coloured according to its `kind` field using the
following palette:

| Kind       | Hex Colour |
|------------|------------|
| `person`   | `#e94560`  |
| `project`  | `#4ade80`  |
| `tool`     | `#fbbf24`  |
| `concept`  | `#60a5fa`  |
| `decision` | `#c084fc`  |
| `server`   | `#f472b6`  |
| `event`    | `#fb923c`  |
| `fact`     | `#94a3b8`  |

Nodes with an unrecognised `kind` MUST use the fallback colour `#888888`.

### 3.2 Opacity by Confidence

Node opacity MUST reflect the node's `confidence` value. The alpha channel
MUST be set to `max(0.15, confidence)`, ensuring that even low-confidence
nodes remain visible. If `confidence` is undefined, the implementation MUST
default to `0.5`. The `nodeOpacity` property MUST be set to `1.0` so that
the per-node alpha applied via the RGBA colour string takes full effect.

In the 2D fallback, `globalAlpha` MUST be set to `max(0.3, confidence)`
(defaulting to `0.7` when confidence is undefined).

### 3.3 Size by Importance

Hub nodes (`is_hub === true`) MUST be rendered at a larger size than
non-hub nodes. In the 3D graph, hub nodes MUST use `nodeVal` of `6` and
non-hub nodes `3`. In the 2D fallback, hub nodes MUST use `nodeVal` of
`12` and non-hub nodes `5`. Node resolution in 3D MUST be set to `12`
segments.

### 3.4 Hover Tooltip

On hover, a tooltip MUST be displayed containing:

- **Label** -- the node's `label` field, rendered in bold.
- **Kind** -- the node's `kind` field, in parentheses after the label.
- **Summary** -- the node's `summary` field, if present.

The tooltip MUST use the theme's overlay background (`--overlay-strong`)
and text colour (`--text`) CSS custom properties.

### 3.5 Label

When the SpriteText library is available, the implementation MUST render a
SpriteText label for each node. The label text MUST be the node's `label`
field. The label colour MUST match the node kind colour from the palette in
Section 3.1, falling back to `#333` in light theme or `#ccc` in dark
theme. The text height MUST be `2.5` units. The label position MUST be
offset to `(0, 5, 0)` relative to the node centre.

In the 2D fallback, labels MUST be drawn via canvas when `globalScale`
exceeds `0.8`, positioned below the node circle at an offset of
`size + 2` pixels.

---

## 4. Edge Rendering

### 4.1 Directional Arrows

All edges MUST display directional arrows. The arrow length MUST be `6`
units and the arrow relative position MUST be `0.95` (near the target
node).

### 4.2 Colour by Confidence

Edge colour MUST be determined by the edge's `confidence` value using the
following thresholds:

| Condition               | Colour    | Meaning |
|-------------------------|-----------|---------|
| `confidence >= 0.8`     | `#4ade80` | Green -- high confidence |
| `confidence >= 0.5`     | `#fbbf24` | Yellow -- medium confidence |
| `confidence >= 0.3`     | `#fb923c` | Orange -- low confidence |
| `confidence < 0.3`      | `#f87171` | Red -- very low confidence |

Orphan links (`is_orphan_link === true`) MUST use the colour `#888888`
(grey) regardless of confidence.

Arrow colour MUST match the edge colour derived from the same confidence
thresholds.

### 4.3 Width

Edge width MUST be proportional to confidence:
`max(0.5, confidence * 1.5)`. Orphan links MUST use a fixed width of
`0.3`. Edge opacity in the 3D graph MUST be set to `0.6`.

### 4.4 Label

When the SpriteText library is available, each edge MUST display a label
showing the `relation` field. The label colour MUST be `#555` in light
theme or `#999` in dark theme. Text height MUST be `1.5` units. The label
MUST be positioned at the midpoint of the edge (the arithmetic mean of
the start and end coordinates on all three axes).

### 4.5 Multi-Edge Handling

When multiple edges connect the same pair of nodes, the implementation
SHOULD visually separate them to prevent overlap. Self-loops MUST be
filtered out: the implementation MUST exclude any link where
`source === target`. Links referencing node IDs not present in the node
set MUST also be excluded.

---

## 5. Node Click (Left-Click)

### 5.1 Info Panel

Left-clicking a node MUST open an info panel at the bottom of the graph
view (above the query bar). The panel MUST display:

- **Label** -- the node's `label`, rendered in bold.
- **Kind** -- the node's `kind`, in parentheses.
- **Summary** -- the node's `summary`, if present.
- **Tags** -- the node's `tags` array joined by commas, rendered in
  subdued text (`--text-dim`), if the array is non-empty.
- **Connections** -- a list of all edges connected to this node. Each
  connection MUST show: the relation name, the label of the other node,
  and the confidence as a percentage. The percentage MUST be colour-coded:
  green (`--green` CSS variable) for >= 80%, yellow (`--yellow`) for
  >= 50%, red (`--red`) for < 50%.

The panel MUST include a close button (rendered as a "X" character) in
the top-right corner.

### 5.2 Camera Transition

On left-click, the camera MUST smoothly transition to centre on the
clicked node over `1500` milliseconds. The camera MUST be positioned at a
distance of `60` units from the node, calculated as:

```
distRatio = 1 + 60 / hypot(node.x, node.y, node.z)
cameraPosition = (node.x * distRatio, node.y * distRatio, node.z * distRatio)
lookAt = (node.x, node.y, node.z)
```

---

## 6. Node Right-Click -- Update Dialog

### 6.1 Modal

Right-clicking a node MUST open a modal dialog titled "Update Node". The
modal MUST display:

- The node's **label** in bold and its **kind** in parentheses.
- A summary of the node's current **connections** (relation and other-node
  label for each edge), rendered in subdued text.
- A **textarea** with placeholder text describing example updates (e.g.,
  *"Change location to Auckland, or Add that this person works at
  University of Auckland"*).
- A **Cancel** button that closes the modal without action.
- An **Update** button that submits the update.

### 6.2 Submission

On submit, the implementation MUST construct a prompt containing:

- The graph name (from the selector).
- The node label and kind.
- The user's update description from the textarea.
- Instructions directing the AI to apply changes using MCP graph tools
  (`graph_add_edge`, `graph_update_evidence`, `graph_strengthen`, etc.)
  with `human_attestation` as the evidence type.

The prompt MUST be sent as a regular chat message via the WebSocket
connection. The implementation MUST:

1. Close the update modal.
2. Switch to the Chat tab.
3. Display the update as a user message in the chat history.
4. Render the chat view so the user can observe the AI's response.

---

## 7. Graph Query Bar

### 7.1 Layout

An input bar MUST be positioned at the bottom of the graph panel,
containing:

- A text input with placeholder *"Ask about this graph..."*.
- An **Ask** button.

### 7.2 Submission

Pressing Enter in the input field or clicking Ask MUST submit a graph
query. The implementation MUST construct a prompt scoped to the currently
selected graph:

```
GRAPH QUERY -- answer using ONLY the "<graphName>" knowledge graph:

<user question>

Read the "<graphName>" topic graph in memory/graphs/ and answer the
question based on what is in that graph. Reference specific nodes and
edges with their confidence levels. If the answer isn't in this graph,
say so -- do not use general knowledge. Keep your answer concise.
```

The query MUST be sent via the WebSocket as a `chat` message. The
implementation MUST:

1. Record the query in the chat history as a user message, prefixed with
   a search indicator and the graph name (e.g., `[graphName] question`).
2. Show the typing indicator for the active ANT.
3. Clear the input field.
4. Switch to the Chat tab.
5. Re-render the chat view.

---

## 8. Export Dialog

### 8.1 Trigger

An **Export** button MUST be displayed in the graph toolbar. Clicking it
MUST open the export modal dialog. The dialog MUST NOT open if no ANT is
active.

### 8.2 Scope

The dialog MUST provide two scope options:

- **Current graph** -- export only the currently selected graph.
- **All knowledge** -- export all topic graphs as a unified report.

Each option MUST be a button. The selected option MUST be visually
distinguished by an accent-coloured border and a contrasting background.
A hint line below the buttons MUST describe the selected scope. The
default scope MUST be `"all"`.

### 8.3 Guidance Prompt

A textarea MUST be provided for the user to enter report guidance. If the
textarea is empty when the dialog opens, it MUST be pre-populated with
the default guidance:

> Summarise the knowledge in this area, looking for practical insights
> and solutions that would work in the real world. Highlight what is
> well-established, what needs further investigation, and any surprising
> connections between ideas.

The textarea MUST receive focus when the dialog opens.

### 8.4 Citations

A checkbox labelled *"Include citations and references"* MUST be provided.
It MUST default to checked.

### 8.5 Submission

On submit, the implementation MUST send a `POST` request to:

```
POST /api/ants/{id}/report
```

with a JSON body containing:

- `graph` -- the graph name (only if scope is "current graph").
- `guidance` -- the contents of the guidance textarea.
- `citations` -- boolean from the citations checkbox.

While the request is in flight, the Export button text MUST change to
*"Starting..."* and the button MUST be disabled.

On success, the implementation MUST:

1. Close the export modal.
2. Switch to the **Workers** tab so the user can track report progress.

On failure, the implementation MUST display an alert with the HTTP status
code and response body.

### 8.6 Dismissal

The export dialog MUST be closable by:

- Clicking the Cancel button.
- Pressing the Escape key.

---

## 9. Live Graph Refresh

When a `graph_updated` WebSocket event arrives, the implementation MUST
automatically reload the graph if all of the following conditions are met:

1. The current tab is the Graph tab.
2. The active ANT matches the `bot` field of the event.
3. The event's `graph` field matches the currently viewed graph, OR the
   event's `graph` field is `"all"`, OR the selector value is empty.

The `graph_updated` event MUST include a `source` field indicating what
triggered the update (e.g., `"consolidation"`, `"rumination"`,
`"user edit"`). The implementation SHOULD log the graph name and source to
the browser console.

---

## 10. Visual Effects

### 10.1 Tumble Animation

Connected nodes (nodes that appear as source or target in at least one
link) MUST exhibit a subtle tumble animation. The animation MUST be
driven by sine functions applied as velocity nudges on each simulation
tick:

```
vx += sin(t * fx + px) * 0.003
vy += sin(t * fy + py) * 0.003
vz += sin(t * fz + pz) * 0.002
```

where `t` is the current time in seconds (`Date.now() * 0.001`).

Per-node phase offsets (`px`, `py`, `pz`) and frequencies (`fx`, `fy`,
`fz`) MUST be derived deterministically from a hash of the node ID using
the Knuth multiplicative hash (`id * 2654435761`, unsigned 32-bit). Phase
values MUST span the range `[0, 2pi]`. Frequencies MUST be:

- `fx = 0.07 + ((hash >> 4) & 0xF) / 150`
- `fy = 0.09 + ((hash >> 12) & 0xF) / 150`
- `fz = 0.06 + ((hash >> 20) & 0xF) / 150`

If a node has a Three.js object (`__threeObj`), its `rotation.y` MUST be
set to `sin(t * fy * 0.3 + py) * 0.5` on each tick to create a gentle
label wobble.

### 10.2 Simulation Parameters

The force simulation MUST be configured with warm parameters to allow
gentle nudges to propagate through the graph:

- `d3AlphaDecay`: `0.005` (low -- keeps simulation warm).
- `d3VelocityDecay`: `0.4` (moderate -- damps oscillation).
- `warmupTicks`: `100`.
- `cooldownTime`: `3000` ms.

### 10.3 Force Configuration

The charge force MUST use adaptive strength based on edge count:

- Nodes with at least one edge: `strength = -40 - (edgeCount * 10)`.
- Nodes with no edges: `strength = -5`.

The link force MUST distinguish orphan links from regular links:

- Regular links: distance `60`, strength `0.8`.
- Orphan links: distance `120`, strength `0.05`.

### 10.4 Node Dragging

Node dragging MUST be enabled (`enableNodeDrag(true)`).

### 10.5 Background Colour

The graph background colour MUST match the current theme's `--bg` CSS
custom property.

### 10.6 Initial Zoom

After warmup and a `500` ms settle period, the implementation MUST
smoothly zoom to fit the entire graph within the viewport over `800` ms
with a `60`-unit padding.

---

## 11. Legend

A colour legend MUST be displayed in the graph toolbar. The legend MUST
contain:

- A coloured dot and label for each node kind, using the colours from
  Section 3.1: person, project, tool, concept, decision, server, event,
  fact.
- A separator (`|`).
- An edge confidence key: `green` = high, `yellow` = medium, `red` = low.

---

## 12. Context Menu Suppression

The browser's default right-click context menu MUST be suppressed on the
graph container element. This MUST be achieved by calling
`preventDefault()` on the `contextmenu` event. This suppression is
REQUIRED to allow the node update dialog (Section 6) to function without
interference from the browser context menu.

---

## 13. Conformance

### 13.1 REQUIRED

An implementation claiming conformance to this specification MUST:

1. Render the 3D force-directed graph using ForceGraph3D (Three.js) when
   WebGL is available.
2. Provide a 2D fallback renderer when WebGL is unavailable.
3. Apply node colours by kind as specified in Section 3.1.
4. Map node opacity to confidence as specified in Section 3.2.
5. Apply edge colours by confidence as specified in Section 4.2.
6. Display directional arrows on all edges (Section 4.1).
7. Show the node info panel on left-click (Section 5.1).
8. Perform smooth camera transitions on left-click (Section 5.2).
9. Open the update dialog on right-click (Section 6).
10. Provide the graph query bar (Section 7).
11. Provide the export dialog (Section 8).
12. Perform live graph refresh on `graph_updated` events (Section 9).
13. Suppress the browser context menu on the graph canvas (Section 12).
14. Display the legend (Section 11).
15. Populate and operate the graph selector dropdown (Section 2).
16. Filter out self-loops and links to missing nodes (Section 4.5).

### 13.2 RECOMMENDED

An implementation SHOULD:

1. Apply the tumble animation to connected nodes (Section 10.1).
2. Use the warm simulation parameters specified in Section 10.2.
3. Configure adaptive charge and link forces (Section 10.3).

### 13.3 OPTIONAL

An implementation MAY:

1. Provide additional visual effects beyond those specified.
2. Support alternative 3D rendering libraries provided they satisfy all
   REQUIRED rendering behaviours.

---

## 14. References

- ForceGraph3D -- https://github.com/vasturiano/3d-force-graph
- Three.js -- https://threejs.org/
- SpriteText -- https://github.com/vasturiano/three-spritetext
- RFC 2119. Bradner, S. "Key words for use in RFCs to Indicate Requirement
  Levels." IETF, 1997.
- ANTHILL-KNOWLEDGE -- Knowledge Graph specification.
- ANTHILL-DASHBOARD -- Web Dashboard specification.
- ANTHILL-THURISAZ -- Thurisaz Engine specification.
