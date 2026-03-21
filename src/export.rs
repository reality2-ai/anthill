//! Export an ANT's knowledge graph as a self-contained HTML file.
//!
//! The exported file includes:
//! - 3D force-directed graph visualisation (Three.js + 3d-force-graph)
//! - All graph data embedded as JSON
//! - Click-to-explore nodes with evidence trails
//! - Search capability (client-side)
//! - No server needed — opens in any browser
//!
//! Usage: anthill --export-graph --ant <name> --output graph.html

use std::path::Path;
use crate::store::KnowledgeStore;
use crate::store::live::LiveKnowledgeStore;

/// Export all graphs for an ANT as a single self-contained HTML file.
pub fn export_ant_graphs(memory_dir: &Path, ant_name: &str, output_path: &Path) -> anyhow::Result<()> {
    let store = LiveKnowledgeStore::new(memory_dir.to_path_buf());

    // Collect all graph data.
    let graphs = store.list_graphs()?;
    let mut all_data = Vec::new();

    for g in &graphs {
        if let Ok(viz) = store.to_visualization(&g.name) {
            all_data.push(serde_json::json!({
                "name": g.name,
                "node_count": g.node_count,
                "edge_count": g.edge_count,
                "data": viz,
            }));
        }
    }

    let graphs_json = serde_json::to_string(&all_data)?;

    let html = format!(r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{ant_name} — Knowledge Graph (Anthill)</title>
<script src="https://unpkg.com/three@0.175.0/build/three.min.js"></script>
<script src="https://unpkg.com/three-spritetext@1.9.7/dist/three-spritetext.min.js"></script>
<script src="https://unpkg.com/3d-force-graph@1.78.6/dist/3d-force-graph.min.js"></script>
<style>
* {{ margin: 0; padding: 0; box-sizing: border-box; }}
body {{ background: #0f172a; color: #e2e8f0; font-family: -apple-system, system-ui, sans-serif; overflow: hidden; }}
#header {{ position: fixed; top: 0; left: 0; right: 0; z-index: 100; background: rgba(15,23,42,0.95);
  padding: 12px 20px; display: flex; align-items: center; gap: 16px; border-bottom: 1px solid #1e293b; }}
#header h1 {{ font-size: 18px; font-weight: 600; }}
#header .subtitle {{ color: #94a3b8; font-size: 13px; }}
#selector {{ background: #1e293b; color: #e2e8f0; border: 1px solid #334155; border-radius: 6px;
  padding: 6px 10px; font-size: 13px; }}
#search {{ background: #1e293b; color: #e2e8f0; border: 1px solid #334155; border-radius: 6px;
  padding: 6px 12px; font-size: 13px; width: 200px; }}
#search::placeholder {{ color: #64748b; }}
#graph-container {{ width: 100vw; height: 100vh; padding-top: 50px; }}
#info {{ position: fixed; bottom: 20px; left: 20px; background: rgba(15,23,42,0.95);
  border: 1px solid #334155; border-radius: 8px; padding: 14px 18px; max-width: 500px;
  max-height: 40vh; overflow-y: auto; font-size: 13px; display: none; z-index: 100; }}
#info b {{ color: #f1f5f9; }}
.conf-high {{ color: #4ade80; }}
.conf-mid {{ color: #fbbf24; }}
.conf-low {{ color: #fb923c; }}
.conf-weak {{ color: #f87171; }}
#legend {{ position: fixed; top: 60px; right: 20px; background: rgba(15,23,42,0.9);
  border: 1px solid #334155; border-radius: 8px; padding: 10px 14px; font-size: 12px; z-index: 100; }}
#legend div {{ margin: 3px 0; }}
.dot {{ display: inline-block; width: 10px; height: 10px; border-radius: 50%; margin-right: 6px; }}
#stats {{ position: fixed; bottom: 20px; right: 20px; background: rgba(15,23,42,0.9);
  border: 1px solid #334155; border-radius: 8px; padding: 10px 14px; font-size: 12px;
  color: #94a3b8; z-index: 100; }}
a {{ color: #60a5fa; }}
</style>
</head>
<body>

<div id="header">
  <h1>{ant_name}</h1>
  <span class="subtitle">Knowledge Graph — <a href="https://github.com/reality2-ai/anthill">Anthill</a></span>
  <select id="selector" onchange="loadGraph(this.value)"></select>
  <input id="search" type="text" placeholder="Search nodes..." oninput="searchNodes(this.value)">
</div>

<div id="graph-container"></div>

<div id="info"></div>

<div id="legend">
  <div><span class="dot" style="background:#e94560"></span>person</div>
  <div><span class="dot" style="background:#4ade80"></span>project</div>
  <div><span class="dot" style="background:#fbbf24"></span>tool</div>
  <div><span class="dot" style="background:#60a5fa"></span>concept</div>
  <div><span class="dot" style="background:#c084fc"></span>decision</div>
  <div><span class="dot" style="background:#f472b6"></span>server</div>
  <div><span class="dot" style="background:#fb923c"></span>event</div>
  <div><span class="dot" style="background:#94a3b8"></span>fact</div>
  <div style="margin-top:8px; border-top:1px solid #334155; padding-top:6px">
    <span class="conf-high">■</span> ≥80% &nbsp;
    <span class="conf-mid">■</span> ≥50% &nbsp;
    <span class="conf-low">■</span> ≥30% &nbsp;
    <span class="conf-weak">■</span> &lt;30%
  </div>
</div>

<div id="stats"></div>

<script>
const ALL_GRAPHS = {graphs_json};

const NODE_COLORS = {{
  person: '#e94560', project: '#4ade80', tool: '#fbbf24',
  concept: '#60a5fa', decision: '#c084fc', server: '#f472b6',
  event: '#fb923c', fact: '#94a3b8',
}};

let graphInstance = null;
let currentData = null;

// Populate selector.
const selector = document.getElementById('selector');
ALL_GRAPHS.forEach((g, i) => {{
  const opt = document.createElement('option');
  opt.value = i;
  opt.textContent = g.name === 'meta' ? 'Meta-graph (index)' : g.name + ' (' + g.node_count + ' nodes)';
  selector.appendChild(opt);
}});

// Auto-select first non-empty graph.
let firstNonEmpty = ALL_GRAPHS.findIndex(g => g.node_count > 0 && g.name !== 'meta');
if (firstNonEmpty < 0) firstNonEmpty = 0;
selector.value = firstNonEmpty;

function loadGraph(idx) {{
  const g = ALL_GRAPHS[idx];
  if (!g) return;
  const data = g.data;
  currentData = data;

  if (!data.nodes || data.nodes.length === 0) {{
    document.getElementById('graph-container').innerHTML =
      '<div style="color:#64748b;text-align:center;padding:100px 40px">This graph is empty.</div>';
    return;
  }}

  // Filter self-loops and invalid links.
  const nodeIds = new Set(data.nodes.map(n => n.id));
  data.links = (data.links || []).filter(l =>
    l.source !== l.target && nodeIds.has(l.source) && nodeIds.has(l.target));

  const container = document.getElementById('graph-container');

  if (graphInstance) {{
    graphInstance._destructor && graphInstance._destructor();
    graphInstance = null;
  }}
  container.innerHTML = '';

  graphInstance = ForceGraph3D()(container)
    .graphData(data)
    .nodeLabel(n => {{
      return '<div style="background:rgba(15,23,42,0.95);padding:6px 10px;border-radius:6px;font-size:13px;color:#e2e8f0">' +
        '<b>' + n.label + '</b> (' + n.kind + ')<br>' + (n.summary || '') + '</div>';
    }})
    .nodeColor(n => {{
      const c = n.confidence !== undefined ? n.confidence : 0.5;
      const alpha = Math.max(0.15, c);
      const hex = NODE_COLORS[n.kind] || '#888888';
      const r = parseInt(hex.slice(1,3), 16) || 136;
      const g = parseInt(hex.slice(3,5), 16) || 136;
      const b = parseInt(hex.slice(5,7), 16) || 136;
      return 'rgba(' + r + ',' + g + ',' + b + ',' + alpha + ')';
    }})
    .nodeOpacity(1.0)
    .nodeVal(n => n.is_hub ? 6 : 3)
    .nodeResolution(12);

  if (typeof SpriteText !== 'undefined') {{
    graphInstance
      .nodeThreeObjectExtend(true)
      .nodeThreeObject(n => {{
        const sprite = new SpriteText(n.label);
        sprite.color = NODE_COLORS[n.kind] || '#ccc';
        sprite.textHeight = 2.5;
        sprite.position.set(0, 5, 0);
        return sprite;
      }})
      .linkThreeObjectExtend(true)
      .linkThreeObject(l => {{
        const sprite = new SpriteText(l.relation);
        sprite.color = '#999';
        sprite.textHeight = 1.5;
        return sprite;
      }})
      .linkPositionUpdate((sprite, {{ start, end }}) => {{
        if (sprite && sprite.position && start && end) {{
          Object.assign(sprite.position, {{
            x: start.x + (end.x - start.x) / 2,
            y: start.y + (end.y - start.y) / 2,
            z: start.z + (end.z - start.z) / 2,
          }});
        }}
      }});
  }}

  graphInstance
    .linkWidth(l => l.is_orphan_link ? 0.3 : Math.max(0.5, l.confidence * 1.5))
    .linkOpacity(0.6)
    .linkColor(l => {{
      if (l.is_orphan_link) return '#888888';
      if (l.confidence >= 0.8) return '#4ade80';
      if (l.confidence >= 0.5) return '#fbbf24';
      if (l.confidence >= 0.3) return '#fb923c';
      return '#f87171';
    }})
    .linkDirectionalArrowLength(6)
    .linkDirectionalArrowRelPos(0.95)
    .linkDirectionalArrowColor(l => {{
      if (l.confidence >= 0.8) return '#4ade80';
      if (l.confidence >= 0.5) return '#fbbf24';
      if (l.confidence >= 0.3) return '#fb923c';
      return '#f87171';
    }})
    .backgroundColor('#0f172a')
    .enableNodeDrag(true)
    .onNodeClick(node => {{
      const edges = data.links.filter(l =>
        (l.source.id || l.source) === node.id || (l.target.id || l.target) === node.id);
      let html = '<b>' + node.label + '</b> (' + node.kind + ')';
      if (node.summary) html += '<br>' + node.summary;
      if (node.tags && node.tags.length) html += '<br><span style="color:#64748b">Tags: ' + node.tags.join(', ') + '</span>';
      if (edges.length) {{
        html += '<br><br><b>Connections:</b>';
        edges.forEach(e => {{
          const other = (e.source.id || e.source) === node.id
            ? (data.nodes.find(n => n.id === (e.target.id || e.target)) || {{}}).label || '?'
            : (data.nodes.find(n => n.id === (e.source.id || e.source)) || {{}}).label || '?';
          const conf = Math.round(e.confidence * 100);
          const cls = conf >= 80 ? 'conf-high' : conf >= 50 ? 'conf-mid' : conf >= 30 ? 'conf-low' : 'conf-weak';
          html += '<br>→ ' + e.relation + ' → ' + other + ' <span class="' + cls + '">' + conf + '%</span>';
          if (e.basis) html += ' <span style="color:#64748b">(' + e.basis + ')</span>';
        }});
      }}
      html += '<br><br><button onclick="document.getElementById(\'info\').style.display=\'none\'" style="background:#334155;color:#e2e8f0;border:none;border-radius:4px;padding:4px 10px;cursor:pointer;font-size:12px">Close</button>';
      document.getElementById('info').innerHTML = html;
      document.getElementById('info').style.display = 'block';

      const distance = 60;
      const distRatio = 1 + distance / Math.hypot(node.x, node.y, node.z);
      graphInstance.cameraPosition(
        {{ x: node.x * distRatio, y: node.y * distRatio, z: node.z * distRatio }},
        {{ x: node.x, y: node.y, z: node.z }},
        1500
      );
    }});

  document.getElementById('stats').innerHTML =
    g.name + ': ' + data.nodes.length + ' nodes, ' + data.links.length + ' edges';
}}

function searchNodes(query) {{
  if (!currentData || !graphInstance) return;
  if (!query.trim()) {{
    graphInstance.nodeColor(n => {{
      const c = n.confidence !== undefined ? n.confidence : 0.5;
      const alpha = Math.max(0.15, c);
      const hex = NODE_COLORS[n.kind] || '#888888';
      const r = parseInt(hex.slice(1,3), 16) || 136;
      const g = parseInt(hex.slice(3,5), 16) || 136;
      const b = parseInt(hex.slice(5,7), 16) || 136;
      return 'rgba(' + r + ',' + g + ',' + b + ',' + alpha + ')';
    }});
    return;
  }}
  const q = query.toLowerCase();
  graphInstance.nodeColor(n => {{
    const match = n.label.toLowerCase().includes(q) ||
      (n.summary || '').toLowerCase().includes(q) ||
      (n.tags || []).some(t => t.toLowerCase().includes(q));
    if (match) {{
      return NODE_COLORS[n.kind] || '#ffffff';
    }} else {{
      return 'rgba(100,100,100,0.1)';
    }}
  }});
}}

// Load first graph.
loadGraph(firstNonEmpty);
</script>
</body>
</html>"#,
        ant_name = ant_name,
        graphs_json = graphs_json,
    );

    std::fs::write(output_path, &html)?;
    let size_kb = html.len() / 1024;
    println!("Exported {}'s knowledge graphs to {} ({} KB)",
        ant_name, output_path.display(), size_kb);
    println!("  {} graphs, open in any browser — no server needed.", all_data.len());

    Ok(())
}
