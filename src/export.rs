//! Export an ANT's knowledge graph as a self-contained HTML file.
//!
//! The exported file includes:
//! - 3D force-directed graph visualisation
//! - Pre-computed insights (strongest beliefs, gaps, contradictions)
//! - AI-generated summary (if available)
//! - All graph data embedded as JSON for client-side querying
//! - Unique UUID per snapshot (immutable permalink)
//!
//! Usage: anthill --export-graph --ant <name> --output graph.html

use std::path::Path;
use crate::store::KnowledgeStore;
use crate::store::live::LiveKnowledgeStore;

// Embed vendor JS at compile time — makes the export fully self-contained.
const THREE_JS: &str = include_str!("vendor/three.min.js");
const SPRITETEXT_JS: &str = include_str!("vendor/three-spritetext.min.js");
const FORCEGRAPH_JS: &str = include_str!("vendor/3d-force-graph.min.js");

/// Pre-computed insights about a graph.
struct GraphInsights {
    total_nodes: usize,
    total_edges: usize,
    avg_confidence: f64,
    strongest_beliefs: Vec<(String, String, String, f64)>, // from, to, relation, confidence
    weakest_beliefs: Vec<(String, String, String, f64)>,
    most_connected: Vec<(String, usize)>, // label, connection count
    topic_summaries: Vec<(String, usize, usize, f64)>, // name, nodes, edges, avg_conf
}

/// Compute insights from the graph data.
fn compute_insights(all_data: &[serde_json::Value]) -> GraphInsights {
    let mut total_nodes = 0;
    let mut total_edges = 0;
    let mut total_conf = 0.0;
    let mut conf_count = 0;
    let mut strongest = Vec::new();
    let mut weakest = Vec::new();
    let mut node_connections: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut topic_summaries = Vec::new();

    for graph in all_data {
        let name = graph["name"].as_str().unwrap_or("?");
        let data = &graph["data"];
        let nodes = data["nodes"].as_array();
        let links = data["links"].as_array();

        let n_count = nodes.map(|n| n.len()).unwrap_or(0);
        let e_count = links.map(|l| l.len()).unwrap_or(0);
        total_nodes += n_count;
        total_edges += e_count;

        let mut graph_conf_sum = 0.0;
        let mut graph_conf_count = 0;

        // Build node label lookup.
        let node_labels: std::collections::HashMap<i64, String> = nodes.unwrap_or(&vec![]).iter()
            .filter_map(|n| {
                let id = n["id"].as_i64()?;
                let label = n["label"].as_str()?.to_string();
                Some((id, label))
            })
            .collect();

        if let Some(links) = links {
            for link in links {
                let conf = link["confidence"].as_f64().unwrap_or(0.5);
                let relation = link["relation"].as_str().unwrap_or("?");
                let source_id = link["source"].as_i64().unwrap_or(-1);
                let target_id = link["target"].as_i64().unwrap_or(-1);
                let from = node_labels.get(&source_id).cloned().unwrap_or_else(|| "?".into());
                let to = node_labels.get(&target_id).cloned().unwrap_or_else(|| "?".into());

                if relation == "?" { continue; } // Skip undetermined

                total_conf += conf;
                conf_count += 1;
                graph_conf_sum += conf;
                graph_conf_count += 1;

                *node_connections.entry(from.clone()).or_default() += 1;
                *node_connections.entry(to.clone()).or_default() += 1;

                strongest.push((from.clone(), to.clone(), relation.to_string(), conf));
                weakest.push((from.clone(), to.clone(), relation.to_string(), conf));
            }
        }

        let graph_avg = if graph_conf_count > 0 { graph_conf_sum / graph_conf_count as f64 } else { 0.0 };
        topic_summaries.push((name.to_string(), n_count, e_count, graph_avg));
    }

    // Sort and trim.
    strongest.sort_by(|a, b| b.3.partial_cmp(&a.3).unwrap_or(std::cmp::Ordering::Equal));
    strongest.truncate(10);

    weakest.sort_by(|a, b| a.3.partial_cmp(&b.3).unwrap_or(std::cmp::Ordering::Equal));
    weakest.retain(|e| e.3 > 0.05); // Skip orphan links
    weakest.truncate(10);

    let mut most_connected: Vec<(String, usize)> = node_connections.into_iter().collect();
    most_connected.sort_by(|a, b| b.1.cmp(&a.1));
    most_connected.truncate(10);

    GraphInsights {
        total_nodes,
        total_edges,
        avg_confidence: if conf_count > 0 { total_conf / conf_count as f64 } else { 0.0 },
        strongest_beliefs: strongest,
        weakest_beliefs: weakest,
        most_connected,
        topic_summaries,
    }
}

fn render_insights_html(insights: &GraphInsights, ant_name: &str, snapshot_id: &str) -> String {
    let mut html = String::new();

    html.push_str(&format!(
        "<h2>{} — Knowledge Snapshot</h2>\n\
         <p style='color:#94a3b8'>Snapshot ID: {} | {} nodes, {} edges | Average confidence: {:.0}%</p>\n",
        ant_name, snapshot_id, insights.total_nodes, insights.total_edges,
        insights.avg_confidence * 100.0
    ));

    // ── Executive Summary (narrative) ──
    html.push_str("<div style='background:#1e293b;border-radius:8px;padding:20px;margin:20px 0;line-height:1.7'>\n");
    html.push_str("<h3 style='margin-top:0'>Executive Summary</h3>\n");

    // Overall assessment.
    let confidence_desc = if insights.avg_confidence >= 0.7 { "well-established" }
        else if insights.avg_confidence >= 0.5 { "moderately confident" }
        else if insights.avg_confidence >= 0.3 { "still developing" }
        else { "largely exploratory" };

    let active_topics: Vec<&(String, usize, usize, f64)> = insights.topic_summaries.iter()
        .filter(|(_, n, _, _)| *n > 0)
        .collect();

    html.push_str(&format!(
        "<p>{} maintains knowledge across <b>{} topic areas</b> comprising {} entities \
         and {} relationships. The overall confidence level is <b>{}</b> at {:.0}%, \
         meaning {}.</p>\n",
        ant_name,
        active_topics.len(),
        insights.total_nodes,
        insights.total_edges,
        confidence_desc,
        insights.avg_confidence * 100.0,
        match confidence_desc {
            "well-established" => "most beliefs have survived testing and are backed by diverse evidence",
            "moderately confident" => "many beliefs have supporting evidence but would benefit from further refutation testing",
            "still developing" => "the knowledge base is growing but many conjectures need more rigorous testing",
            _ => "most ideas are early-stage conjectures that need significant evidence gathering",
        }
    ));

    // Topic-by-topic narrative.
    if !active_topics.is_empty() {
        html.push_str("<h4>Knowledge Areas</h4>\n");
        for (name, nodes, edges, avg) in &insights.topic_summaries {
            if *nodes == 0 { continue; }
            let density = if *nodes > 1 { *edges as f64 / *nodes as f64 } else { 0.0 };
            let density_desc = if density >= 3.0 { "densely connected" }
                else if density >= 1.5 { "well-connected" }
                else if density >= 0.5 { "sparsely connected" }
                else { "mostly isolated entities" };
            let conf_desc = if *avg >= 0.7 { "high confidence" }
                else if *avg >= 0.5 { "moderate confidence" }
                else { "low confidence — needs more evidence" };

            html.push_str(&format!(
                "<p><b>{}</b> — {} entities with {} relationships ({}). \
                 Average confidence: {:.0}% ({}). {}</p>\n",
                name.replace('-', " "),
                nodes, edges, density_desc,
                avg * 100.0, conf_desc,
                if *avg < 0.5 { "This area would benefit from active refutation testing and external source corroboration." }
                else if *avg < 0.7 { "The foundations are present but could be strengthened through diverse evidence types." }
                else { "This is a well-developed knowledge area with strong evidential support." }
            ));
        }
    }

    // Key findings from strongest beliefs.
    if !insights.strongest_beliefs.is_empty() {
        html.push_str("<h4>Key Findings</h4>\n<p>The strongest beliefs in this knowledge base — those that have earned high confidence through diverse evidence — are:</p>\n<ul>\n");
        for (from, to, rel, conf) in insights.strongest_beliefs.iter().take(5) {
            html.push_str(&format!(
                "<li><b>{}</b> {} <b>{}</b> ({:.0}% confidence)</li>\n",
                from, rel, to, conf * 100.0
            ));
        }
        html.push_str("</ul>\n");
        html.push_str("<p style='color:#94a3b8;font-size:12px'>Note: In this system, high confidence means the belief has survived genuine attempts at refutation, \
            not just been confirmed repeatedly. Confidence is capped based on evidence diversity — \
            an idea needs different <em>kinds</em> of evidence, not just more of the same.</p>\n");
    }

    // Areas of uncertainty.
    if !insights.weakest_beliefs.is_empty() {
        html.push_str("<h4>Open Questions</h4>\n<p>These relationships have low confidence and would benefit from investigation:</p>\n<ul>\n");
        for (from, to, rel, conf) in insights.weakest_beliefs.iter().take(5) {
            html.push_str(&format!(
                "<li><b>{}</b> {} <b>{}</b> ({:.0}% — {})</li>\n",
                from, rel, to, conf * 100.0,
                if *conf < 0.2 { "doubtful" } else if *conf < 0.4 { "uncertain" } else { "possible" }
            ));
        }
        html.push_str("</ul>\n");
    }

    // Central concepts.
    if !insights.most_connected.is_empty() {
        html.push_str("<h4>Central Concepts</h4>\n<p>These entities are the most connected in the knowledge graph — they appear in the most relationships and are central to the overall understanding:</p>\n<ol>\n");
        for (label, count) in insights.most_connected.iter().take(5) {
            html.push_str(&format!("<li><b>{}</b> — {} connections</li>\n", label, count));
        }
        html.push_str("</ol>\n");
    }

    html.push_str("</div>\n");

    // ── Detailed Tables ──
    html.push_str("<h3>Detailed Breakdown</h3>\n");

    // Topic table.
    html.push_str("<h4>Topic Graphs</h4><table><tr><th>Topic</th><th>Nodes</th><th>Edges</th><th>Avg Confidence</th></tr>\n");
    for (name, nodes, edges, avg) in &insights.topic_summaries {
        if *nodes == 0 { continue; }
        let colour = if *avg >= 0.7 { "#4ade80" } else if *avg >= 0.5 { "#fbbf24" } else { "#f87171" };
        html.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td><td style='color:{}'>{:.0}%</td></tr>\n",
            name, nodes, edges, colour, avg * 100.0
        ));
    }
    html.push_str("</table>\n");

    // All strongest beliefs.
    html.push_str("<h4>All Established Beliefs</h4><ul>\n");
    for (from, to, rel, conf) in &insights.strongest_beliefs {
        html.push_str(&format!(
            "<li><b>{}</b> → {} → <b>{}</b> <span class='conf-high'>{:.0}%</span></li>\n",
            from, rel, to, conf * 100.0
        ));
    }
    html.push_str("</ul>\n");

    // All weakest beliefs.
    html.push_str("<h4>All Uncertain Beliefs</h4><ul>\n");
    for (from, to, rel, conf) in &insights.weakest_beliefs {
        let cls = if *conf >= 0.3 { "conf-low" } else { "conf-weak" };
        html.push_str(&format!(
            "<li><b>{}</b> → {} → <b>{}</b> <span class='{}'>{:.0}%</span></li>\n",
            from, rel, to, cls, conf * 100.0
        ));
    }
    html.push_str("</ul>\n");

    // All connected nodes.
    html.push_str("<h4>All Connected Nodes</h4><ul>\n");
    for (label, count) in &insights.most_connected {
        html.push_str(&format!("<li><b>{}</b> — {} connections</li>\n", label, count));
    }
    html.push_str("</ul>\n");

    html
}

/// Export all graphs for an ANT as a single self-contained HTML file.
pub fn export_ant_graphs(memory_dir: &Path, ant_name: &str, output_path: &Path) -> anyhow::Result<()> {
    let store = LiveKnowledgeStore::new(memory_dir.to_path_buf());

    // Generate unique snapshot ID.
    let snapshot_id = format!("{:08x}", std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs());

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

    let insights = compute_insights(&all_data);
    let insights_html = render_insights_html(&insights, ant_name, &snapshot_id);
    let graphs_json = serde_json::to_string(&all_data)?;
    let timestamp = crate::dateutil::datetime_now();

    let html = format!(r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{ant_name} — Knowledge Snapshot {snapshot_id}</title>
<script>{three_js}</script>
<script>{spritetext_js}</script>
<script>{forcegraph_js}</script>
<style>
* {{ margin: 0; padding: 0; box-sizing: border-box; }}
body {{ background: #0f172a; color: #e2e8f0; font-family: -apple-system, system-ui, sans-serif; }}
#header {{ background: rgba(15,23,42,0.95); padding: 12px 20px; display: flex; align-items: center;
  gap: 16px; border-bottom: 1px solid #1e293b; position: sticky; top: 0; z-index: 100; }}
#header h1 {{ font-size: 18px; font-weight: 600; }}
.subtitle {{ color: #94a3b8; font-size: 13px; }}
#tabs {{ display: flex; gap: 0; margin-left: auto; }}
#tabs button {{ background: none; border: none; color: #94a3b8; padding: 8px 16px; cursor: pointer;
  font-size: 13px; border-bottom: 2px solid transparent; }}
#tabs button.active {{ color: #e2e8f0; border-bottom-color: #60a5fa; }}
#selector {{ background: #1e293b; color: #e2e8f0; border: 1px solid #334155; border-radius: 6px;
  padding: 6px 10px; font-size: 13px; }}
#search {{ background: #1e293b; color: #e2e8f0; border: 1px solid #334155; border-radius: 6px;
  padding: 6px 12px; font-size: 13px; width: 200px; }}
#search::placeholder {{ color: #64748b; }}
#graph-view {{ width: 100%; height: 80vh; }}
#insights-view {{ display: none; max-width: 900px; margin: 0 auto; padding: 30px 20px; }}
#insights-view h2 {{ font-size: 22px; margin-bottom: 8px; }}
#insights-view h3 {{ font-size: 16px; margin: 20px 0 8px; color: #60a5fa; }}
#insights-view table {{ width: 100%; border-collapse: collapse; font-size: 13px; }}
#insights-view th {{ text-align: left; padding: 6px 12px; color: #94a3b8; border-bottom: 1px solid #334155; }}
#insights-view td {{ padding: 6px 12px; border-bottom: 1px solid #1e293b; }}
#insights-view ul {{ list-style: none; padding: 0; }}
#insights-view li {{ padding: 4px 0; font-size: 13px; }}
#info {{ position: fixed; bottom: 20px; left: 20px; background: rgba(15,23,42,0.95);
  border: 1px solid #334155; border-radius: 8px; padding: 14px 18px; max-width: 500px;
  max-height: 40vh; overflow-y: auto; font-size: 13px; display: none; z-index: 100; }}
.conf-high {{ color: #4ade80; }} .conf-mid {{ color: #fbbf24; }}
.conf-low {{ color: #fb923c; }} .conf-weak {{ color: #f87171; }}
#legend {{ position: fixed; top: 60px; right: 20px; background: rgba(15,23,42,0.9);
  border: 1px solid #334155; border-radius: 8px; padding: 10px 14px; font-size: 12px; z-index: 100; }}
#legend div {{ margin: 3px 0; }}
.dot {{ display: inline-block; width: 10px; height: 10px; border-radius: 50%; margin-right: 6px; }}
#footer {{ text-align: center; padding: 20px; color: #64748b; font-size: 12px; border-top: 1px solid #1e293b; }}
a {{ color: #60a5fa; }}
</style>
</head>
<body>

<div id="header">
  <h1>{ant_name}</h1>
  <span class="subtitle">Knowledge Snapshot — {timestamp}</span>
  <div id="tabs">
    <button class="active" onclick="showTab('graph')">Graph</button>
    <button onclick="showTab('insights')">Insights</button>
  </div>
  <select id="selector" onchange="loadGraph(this.value)"></select>
  <input id="search" type="text" placeholder="Search nodes..." oninput="searchNodes(this.value)">
</div>

<div id="graph-view"></div>

<div id="insights-view">
{insights_html}
</div>

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
  <div style="margin-top:8px;border-top:1px solid #334155;padding-top:6px">
    <span class="conf-high">■</span> ≥80% &nbsp;<span class="conf-mid">■</span> ≥50% &nbsp;
    <span class="conf-low">■</span> ≥30% &nbsp;<span class="conf-weak">■</span> &lt;30%
  </div>
</div>

<div id="footer">
  Generated by <a href="https://github.com/reality2-ai/anthill">Anthill</a> v{version} — Snapshot {snapshot_id}
  <br>A Popperian reasoning engine where ideas compete for survival.
</div>

<script>
const ALL_GRAPHS = {graphs_json};
const NODE_COLORS = {{
  person:'#e94560',project:'#4ade80',tool:'#fbbf24',concept:'#60a5fa',
  decision:'#c084fc',server:'#f472b6',event:'#fb923c',fact:'#94a3b8',
  theory:'#818cf8',mechanism:'#2dd4bf',principle:'#a78bfa',constraint:'#fb7185',
}};
let graphInstance = null, currentData = null;

function showTab(tab) {{
  document.getElementById('graph-view').style.display = tab === 'graph' ? 'block' : 'none';
  document.getElementById('insights-view').style.display = tab === 'insights' ? 'block' : 'none';
  document.getElementById('legend').style.display = tab === 'graph' ? 'block' : 'none';
  document.querySelectorAll('#tabs button').forEach(b => b.classList.remove('active'));
  event.target.classList.add('active');
}}

const selector = document.getElementById('selector');
ALL_GRAPHS.forEach((g,i) => {{
  const opt = document.createElement('option');
  opt.value = i;
  opt.textContent = g.name === 'meta' ? 'Meta-graph' : g.name + ' (' + g.node_count + ')';
  selector.appendChild(opt);
}});
let firstNonEmpty = ALL_GRAPHS.findIndex(g => g.node_count > 0 && g.name !== 'meta');
if (firstNonEmpty < 0) firstNonEmpty = 0;
selector.value = firstNonEmpty;

function loadGraph(idx) {{
  const g = ALL_GRAPHS[idx]; if (!g) return;
  const data = g.data; currentData = data;
  if (!data.nodes || !data.nodes.length) {{
    document.getElementById('graph-view').innerHTML='<div style="color:#64748b;text-align:center;padding:100px">Empty graph.</div>';
    return;
  }}
  const nodeIds = new Set(data.nodes.map(n=>n.id));
  data.links = (data.links||[]).filter(l=>l.source!==l.target && nodeIds.has(l.source) && nodeIds.has(l.target));
  const container = document.getElementById('graph-view');
  if (graphInstance) {{ graphInstance._destructor && graphInstance._destructor(); graphInstance=null; }}
  container.innerHTML='';
  graphInstance = ForceGraph3D()(container).graphData(data)
    .nodeLabel(n=>'<div style="background:rgba(15,23,42,0.95);padding:6px 10px;border-radius:6px;font-size:13px;color:#e2e8f0"><b>'+n.label+'</b> ('+n.kind+')<br>'+(n.summary||'')+'</div>')
    .nodeColor(n=>{{ const c=n.confidence!==undefined?n.confidence:0.5; const a=Math.max(0.15,c); const h=NODE_COLORS[n.kind]||'#888'; const r=parseInt(h.slice(1,3),16)||136; const g=parseInt(h.slice(3,5),16)||136; const b=parseInt(h.slice(5,7),16)||136; return 'rgba('+r+','+g+','+b+','+a+')'; }})
    .nodeOpacity(1).nodeVal(n=>n.is_hub?6:3).nodeResolution(12);
  if(typeof SpriteText!=='undefined'){{
    graphInstance.nodeThreeObjectExtend(true).nodeThreeObject(n=>{{ const s=new SpriteText(n.label); s.color=NODE_COLORS[n.kind]||'#ccc'; s.textHeight=2.5; s.position.set(0,5,0); return s; }})
    .linkThreeObjectExtend(true).linkThreeObject(l=>{{ const s=new SpriteText(l.relation); s.color='#999'; s.textHeight=1.5; return s; }})
    .linkPositionUpdate((s,{{start,end}})=>{{ if(s&&s.position&&start&&end) Object.assign(s.position,{{x:start.x+(end.x-start.x)/2,y:start.y+(end.y-start.y)/2,z:start.z+(end.z-start.z)/2}}); }});
  }}
  graphInstance.linkWidth(l=>l.is_orphan_link?0.3:Math.max(0.5,l.confidence*1.5)).linkOpacity(0.6)
    .linkColor(l=>{{ if(l.is_orphan_link)return'#888'; if(l.confidence>=0.8)return'#4ade80'; if(l.confidence>=0.5)return'#fbbf24'; if(l.confidence>=0.3)return'#fb923c'; return'#f87171'; }})
    .linkDirectionalArrowLength(6).linkDirectionalArrowRelPos(0.95)
    .linkDirectionalArrowColor(l=>{{ if(l.confidence>=0.8)return'#4ade80'; if(l.confidence>=0.5)return'#fbbf24'; if(l.confidence>=0.3)return'#fb923c'; return'#f87171'; }})
    .backgroundColor('#0f172a').enableNodeDrag(true)
    .onNodeClick(node=>{{
      const edges=data.links.filter(l=>(l.source.id||l.source)===node.id||(l.target.id||l.target)===node.id);
      let h='<b>'+node.label+'</b> ('+node.kind+')';
      if(node.summary) h+='<br>'+node.summary;
      if(node.tags&&node.tags.length) h+='<br><span style="color:#64748b">Tags: '+node.tags.join(', ')+'</span>';
      if(edges.length){{ h+='<br><br><b>Connections:</b>';
        edges.forEach(e=>{{ const o=(e.source.id||e.source)===node.id?(data.nodes.find(n=>n.id===(e.target.id||e.target))||{{}}).label||'?':(data.nodes.find(n=>n.id===(e.source.id||e.source))||{{}}).label||'?';
          const c=Math.round(e.confidence*100); const cls=c>=80?'conf-high':c>=50?'conf-mid':c>=30?'conf-low':'conf-weak';
          h+='<br>→ '+e.relation+' → '+o+' <span class="'+cls+'">'+c+'%</span>';
          if(e.basis) h+=' <span style="color:#64748b">('+e.basis+')</span>';
        }});
      }}
      h+='<br><br><button onclick="document.getElementById(\'info\').style.display=\'none\'" style="background:#334155;color:#e2e8f0;border:none;border-radius:4px;padding:4px 10px;cursor:pointer;font-size:12px">Close</button>';
      document.getElementById('info').innerHTML=h; document.getElementById('info').style.display='block';
      const d=60,dr=1+d/Math.hypot(node.x,node.y,node.z);
      graphInstance.cameraPosition({{x:node.x*dr,y:node.y*dr,z:node.z*dr}},{{x:node.x,y:node.y,z:node.z}},1500);
    }});
}}

function searchNodes(q) {{
  if(!currentData||!graphInstance) return;
  if(!q.trim()){{ graphInstance.nodeColor(n=>{{ const c=n.confidence!==undefined?n.confidence:0.5; const a=Math.max(0.15,c); const h=NODE_COLORS[n.kind]||'#888'; const r=parseInt(h.slice(1,3),16)||136; const g=parseInt(h.slice(3,5),16)||136; const b=parseInt(h.slice(5,7),16)||136; return'rgba('+r+','+g+','+b+','+a+')'; }}); return; }}
  const ql=q.toLowerCase();
  graphInstance.nodeColor(n=>{{ const m=n.label.toLowerCase().includes(ql)||(n.summary||'').toLowerCase().includes(ql)||(n.tags||[]).some(t=>t.toLowerCase().includes(ql));
    return m?(NODE_COLORS[n.kind]||'#fff'):'rgba(100,100,100,0.1)'; }});
}}

loadGraph(firstNonEmpty);
</script>
</body></html>"##,
        ant_name = ant_name,
        snapshot_id = snapshot_id,
        timestamp = timestamp,
        insights_html = insights_html,
        graphs_json = graphs_json,
        version = env!("CARGO_PKG_VERSION"),
        three_js = THREE_JS,
        spritetext_js = SPRITETEXT_JS,
        forcegraph_js = FORCEGRAPH_JS,
    );

    std::fs::write(output_path, &html)?;
    let size_kb = html.len() / 1024;
    println!("Exported {}'s knowledge graphs to {} ({} KB, snapshot {})",
        ant_name, output_path.display(), size_kb, snapshot_id);
    println!("  {} graphs with {} nodes, {} edges",
        insights.topic_summaries.len(), insights.total_nodes, insights.total_edges);
    println!("  Open in any browser — no server needed.");
    println!("  Tabs: Graph (3D interactive) | Insights (summary + key beliefs)");

    Ok(())
}

/// Publish an exported HTML file to GitHub Pages via `gh` CLI.
/// Creates a gist (public, shareable link) or publishes to a repo's gh-pages branch.
/// Returns the URL if successful.
pub fn publish_to_github(html_path: &Path, ant_name: &str) -> anyhow::Result<String> {
    // Check if gh is available.
    let gh_available = std::process::Command::new("gh")
        .args(["--version"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if !gh_available {
        return Err(anyhow::anyhow!("GitHub CLI (gh) not installed. Install from https://cli.github.com/"));
    }

    // Check if gh is authenticated.
    let auth_ok = std::process::Command::new("gh")
        .args(["auth", "status"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if !auth_ok {
        return Err(anyhow::anyhow!("GitHub CLI not authenticated. Run: gh auth login"));
    }

    let filename = html_path.file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| format!("{}-knowledge.html", ant_name));

    // Create a public gist — simplest way to get a shareable URL.
    let output = std::process::Command::new("gh")
        .args([
            "gist", "create",
            "--public",
            "--desc", &format!("{} — Knowledge Graph Snapshot (Anthill)", ant_name),
            "--filename", &filename,
        ])
        .arg(html_path)
        .output()
        .map_err(|e| anyhow::anyhow!("gh gist create failed: {}", e))?;

    if output.status.success() {
        let gist_url = String::from_utf8_lossy(&output.stdout).trim().to_string();

        // Convert gist URL to raw URL for direct HTML viewing.
        // gh returns: https://gist.github.com/user/hash
        // Raw view: https://gist.githack.com/user/hash/raw/filename
        // Or use htmlpreview: https://htmlpreview.github.io/?<gist-raw-url>
        let view_url = if gist_url.contains("gist.github.com") {
            format!("{}\n  (View raw HTML: open the gist and click 'Raw')", gist_url)
        } else {
            gist_url.clone()
        };

        println!("Published to GitHub: {}", view_url);
        Ok(gist_url)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(anyhow::anyhow!("gh gist create failed: {}", stderr))
    }
}
