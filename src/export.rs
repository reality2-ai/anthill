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
const FORCEGRAPH_2D_JS: &str = include_str!("vendor/force-graph.min.js");

/// Pre-computed insights about a graph.
struct GraphInsights {
    total_nodes: usize,
    total_edges: usize,
    avg_confidence: f64,
    strongest_beliefs: Vec<(String, String, String, f64)>,
    weakest_beliefs: Vec<(String, String, String, f64)>,
    most_connected: Vec<(String, usize)>,
    topic_summaries: Vec<(String, usize, usize, f64)>,
    node_summaries: std::collections::HashMap<String, String>,
    topic_descriptions: std::collections::HashMap<String, String>,
    /// All citations collected from edges, deduplicated by URL.
    all_citations: Vec<CollectedCitation>,
}

/// A citation collected during export, with the edge it supports.
#[derive(Clone)]
struct CollectedCitation {
    /// Unique citation code from the graph.
    cite_id: String,
    url: String,
    title: String,
    author: String,
    date: String,
    ref_type: String,
    quality: f64,
    /// Which relationship this citation supports.
    supports: String,
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
    let mut node_summaries: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut topic_descriptions: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut all_citations: Vec<CollectedCitation> = Vec::new();
    let mut seen_urls: std::collections::HashSet<String> = std::collections::HashSet::new();

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

        // Build node label lookup and collect summaries.
        let empty_vec = vec![];
        let nodes_arr = nodes.unwrap_or(&empty_vec);
        for n in nodes_arr {
            if let (Some(label), Some(summary)) = (n["label"].as_str(), n["summary"].as_str()) {
                if !summary.is_empty() && summary.len() > 10 {
                    node_summaries.entry(label.to_string())
                        .or_insert_with(|| summary.to_string());
                }
                // First node in each topic with a good summary becomes the topic description.
                if n["tags"].as_array().map(|t| t.iter().any(|v| v.as_str() == Some("hub") || v.as_str() == Some("topic"))).unwrap_or(false) {
                    topic_descriptions.entry(name.to_string())
                        .or_insert_with(|| summary.to_string());
                }
            }
        }

        let node_labels: std::collections::HashMap<i64, String> = nodes_arr.iter()
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

                // Collect citations from this edge.
                if let Some(cites) = link["citations"].as_array() {
                    for cite in cites {
                        let url = cite["url"].as_str().unwrap_or("").to_string();
                        let cite_id = cite["cite_id"].as_str().unwrap_or("").to_string();
                        let key = if !url.is_empty() { url.clone() } else { cite_id.clone() };
                        if !key.is_empty() && seen_urls.insert(key) {
                            let final_cite_id = if cite_id.is_empty() {
                                format!("cite-{:04x}", all_citations.len() + 1)
                            } else { cite_id };
                            all_citations.push(CollectedCitation {
                                cite_id: final_cite_id,
                                url,
                                title: cite["title"].as_str().unwrap_or("").to_string(),
                                author: cite["author"].as_str().unwrap_or("").to_string(),
                                date: cite["date"].as_str().unwrap_or("").to_string(),
                                ref_type: cite["ref_type"].as_str().unwrap_or("website").to_string(),
                                quality: cite["quality"].as_f64().unwrap_or(0.5),
                                supports: format!("{} {} {}", from, relation, to),
                            });
                        }
                    }
                }

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
        node_summaries,
        topic_descriptions,
        all_citations,
    }
}

fn render_insights_html(insights: &GraphInsights, ant_name: &str, snapshot_id: &str) -> String {
    let mut html = String::new();

    html.push_str(&format!(
        "<h2>{} — Knowledge Summary</h2>\n\
         <p style='color:#64748b;font-size:13px'>Snapshot {} | Generated {}</p>\n",
        ant_name, snapshot_id, crate::dateutil::datetime_now()
    ));

    html.push_str("<div style='background:#1e293b;border-radius:8px;padding:24px 28px;margin:20px 0;line-height:1.8;font-size:14px'>\n");

    // Opening: what this document is.
    html.push_str("<p style='color:#94a3b8;font-style:italic;border-left:3px solid #334155;padding-left:12px;margin-bottom:20px'>\
        This document presents what an AI reasoning agent has learned about its subject areas. \
        Every piece of knowledge here is treated as a conjecture — an idea that must earn its \
        confidence by surviving genuine attempts to disprove it. When we say an idea has high \
        confidence, we mean it has been challenged from multiple different angles and held up \
        each time. Ideas with lower confidence are still being investigated.</p>\n");

    // Overview.
    let active_topics: Vec<&(String, usize, usize, f64)> = insights.topic_summaries.iter()
        .filter(|(_, n, _, _)| *n > 0).collect();

    let confidence_word = if insights.avg_confidence >= 0.7 { "strong" }
        else if insights.avg_confidence >= 0.5 { "moderate" }
        else if insights.avg_confidence >= 0.3 { "developing" }
        else { "early" };

    if active_topics.len() == 1 {
        let (name, nodes, edges, avg) = active_topics[0];
        let pretty = name.replace('-', " ");
        let desc = insights.topic_descriptions.get(name.as_str()).cloned().unwrap_or_default();
        html.push_str(&format!(
            "<p>This summary covers <b>{}</b>{}. The knowledge base contains {} concepts \
             connected by {} relationships, with an overall confidence of {:.0}%. \
             The evidence base is {} — {}.</p>\n",
            pretty,
            if desc.is_empty() { String::new() } else { format!(". {}", desc) },
            nodes, edges, avg * 100.0, confidence_word,
            match confidence_word {
                "strong" => "most ideas here have been rigorously tested and are well-supported",
                "moderate" => "the core ideas are supported but would benefit from further investigation",
                "developing" => "many ideas are still being explored and tested",
                _ => "this is an early exploration of the subject",
            }
        ));
    } else if !active_topics.is_empty() {
        let topic_names: Vec<String> = active_topics.iter()
            .map(|(n, _, _, _)| n.replace('-', " ")).collect();
        html.push_str(&format!(
            "<p>This summary covers {} areas of knowledge: {}. \
             Across all topics, there are {} concepts connected by {} relationships, \
             with an overall confidence level of {:.0}%.</p>\n",
            active_topics.len(), format_list(&topic_names),
            insights.total_nodes, insights.total_edges, insights.avg_confidence * 100.0
        ));
    }

    // Topic-by-topic narrative.
    for (idx, (name, nodes, edges, avg)) in insights.topic_summaries.iter().enumerate() {
        if *nodes == 0 { continue; }
        let pretty = name.replace('-', " ");
        let desc = insights.topic_descriptions.get(name.as_str()).cloned().unwrap_or_default();

        html.push_str(&format!("<h3 style=\'margin-top:24px\'>{} <a href=\'#\' onclick=\'showTab(\"graph\",{});return false;\' \
            style=\'font-size:12px;color:#60a5fa;text-decoration:none;margin-left:8px\'>View graph →</a></h3>\n",
            pretty, idx));

        if !desc.is_empty() {
            html.push_str(&format!("<p>{}</p>\n", desc));
        }

        let density = if *nodes > 1 { *edges as f64 / *nodes as f64 } else { 0.0 };
        html.push_str(&format!(
            "<p>This area encompasses {} concepts with {} connections between them{}. \
             The average confidence across these relationships is {:.0}%{}.</p>\n",
            nodes, edges,
            if density >= 2.0 { ", forming a richly interconnected body of knowledge" }
            else if density >= 1.0 { "" }
            else { ", though many concepts are not yet well connected to each other" },
            avg * 100.0,
            if *avg >= 0.7 { ", indicating well-tested understanding" }
            else if *avg >= 0.5 { ", suggesting the foundations are present but would benefit from deeper investigation" }
            else { ", meaning much of this knowledge is still in the conjecture phase" }
        ));

        // Key entities as flowing prose.
        let relevant: Vec<(&String, &String)> = insights.node_summaries.iter()
            .filter(|(_, s)| s.len() > 20).take(5).collect();
        if !relevant.is_empty() {
            html.push_str("<p>Key concepts include ");
            for (i, (label, summary)) in relevant.iter().enumerate() {
                let short = if summary.len() > 150 {
                    let end = summary[..150].rfind(' ').unwrap_or(150);
                    format!("{}...", &summary[..end])
                } else { summary.to_string() };
                if i > 0 { html.push_str(". "); }
                html.push_str(&format!("<b>{}</b>, which {}", label,
                    if short.starts_with(|c: char| c.is_uppercase()) { short[..1].to_lowercase() + &short[1..] }
                    else { short }));
            }
            html.push_str(".</p>\n");
        }
    }

    // What is well established.
    if !insights.strongest_beliefs.is_empty() {
        html.push_str("<h3 style=\'margin-top:24px\'>What Is Well Established</h3>\n");
        html.push_str("<p>The following relationships have earned high confidence through diverse evidence — \
            meaning they have survived genuine attempts at disproof from multiple angles, not simply \
            been confirmed repeatedly.</p>\n<p>");
        for (i, (from, to, rel, conf)) in insights.strongest_beliefs.iter().take(7).enumerate() {
            let from_desc = insights.node_summaries.get(from.as_str())
                .map(|s| { let short = if s.len() > 80 { format!("{}...", &s[..s[..80].rfind(' ').unwrap_or(80)]) } else { s.clone() }; format!(" ({})", short) })
                .unwrap_or_default();
            if i > 0 { html.push_str(" "); }
            html.push_str(&format!("<b>{}</b>{} {} <b>{}</b> ({:.0}% confidence).",
                from, from_desc, rel, to, conf * 100.0));
        }
        html.push_str("</p>\n");
    }

    // Areas needing investigation.
    if !insights.weakest_beliefs.is_empty() {
        html.push_str("<h3 style=\'margin-top:24px\'>Areas Needing Further Investigation</h3>\n");
        html.push_str("<p>These ideas are still at an early stage. Low confidence does not mean an idea is \
            wrong — it means it has not yet been sufficiently tested against independent evidence.</p>\n<p>");
        for (i, (from, to, rel, conf)) in insights.weakest_beliefs.iter().take(5).enumerate() {
            if i > 0 { html.push_str(" "); }
            html.push_str(&format!("The relationship between <b>{}</b> and <b>{}</b> ({}) currently sits at {:.0}% confidence.",
                from, to, rel, conf * 100.0));
        }
        html.push_str("</p>\n");
    }

    // Central themes.
    if !insights.most_connected.is_empty() {
        html.push_str("<h3 style=\'margin-top:24px\'>Central Themes</h3>\n");
        let central: Vec<String> = insights.most_connected.iter().take(5)
            .map(|(label, count)| {
                let desc = insights.node_summaries.get(label.as_str())
                    .map(|s| { let short = if s.len() > 100 { format!("{}...", &s[..s[..100].rfind(' ').unwrap_or(100)]) } else { s.clone() }; format!(" — {}", short) })
                    .unwrap_or_default();
                format!("<b>{}</b> ({} connections{})", label, count, desc)
            }).collect();
        html.push_str(&format!(
            "<p>The concepts that tie this knowledge together most strongly are: {}. \
             These act as hubs — understanding them provides the best foundation for \
             understanding the broader subject matter.</p>\n", format_list(&central)));
    }

    // Closing.
    html.push_str("<p style=\'margin-top:24px;padding-top:16px;border-top:1px solid #334155;color:#94a3b8;font-size:13px\'>\
        This summary was generated from a Popperian knowledge graph — a system where every \
        idea must earn its confidence through surviving genuine challenges. The interactive \
        3D visualisation (Graph tab) allows exploration of individual concepts and their \
        connections. All knowledge is provisional and subject to revision as new evidence emerges. \
        Generated by <a href=\'https://github.com/reality2-ai/anthill\' style=\'color:#60a5fa\'>Anthill</a>.</p>\n");

    html.push_str("</div>\n");

    html
}


/// Ask an AI to rewrite the raw insights as polished plain English.
/// The user's guidance prompt is the primary instruction to the AI.
/// Falls back to the algorithmic version if the AI is unavailable.
fn ai_polish_summary(raw_insights: &str, ant_name: &str, guidance: Option<&str>) -> String {
    let has_citations = raw_insights.contains("[cite-");
    let citation_instruction = if has_citations {
        "\n\nCITATIONS ARE MANDATORY.\n\
         At the end of the data you will find a SOURCES AND REFERENCES section listing \
         citation codes like [cite-a1b2c3d4]. You MUST use these codes inline in your text.\n\n\
         HOW TO CITE:\n\
         - Write a claim, then put the citation code immediately after it in square brackets.\n\
         - Example: 'Research has shown that early intervention significantly improves outcomes [cite-a1b2c3d4].'\n\
         - Example: 'According to recent analysis [cite-f5e6d7c8], the trend is accelerating.'\n\
         - You can cite multiple sources: 'This finding is well-supported [cite-a1b2c3d4] [cite-b2c3d4e5].'\n\n\
         RULES:\n\
         - Cite the source whenever you introduce a new idea, finding, or claim from the data.\n\
         - If a claim has no supporting citation, it is SPECULATION — use language like \
           'it appears that', 'this suggests', or 'further investigation is needed' to signal this.\n\
         - Use as MANY of the provided citations as are relevant — don't just pick a few.\n\
         - The [cite-xxxx] codes will be automatically renumbered to [1], [2], etc.\n\
         - ONLY use codes from the provided list — NEVER invent a citation code."
    } else { "" };

    // The user's guidance is the primary prompt. If none provided, use a sensible default.
    let user_prompt = guidance.unwrap_or(
        "Summarise the knowledge in this area, looking for practical insights and solutions \
         that would work in the real world. Highlight what is well-established, what needs \
         further investigation, and any surprising connections between ideas."
    );

    let prompt = format!(
        "You are writing a report based on knowledge graph data for '{ant_name}'.\n\n\
         YOUR TASK:\n{user_prompt}\n\n\
         FORMATTING RULES:\n\
         - Write flowing prose — no bullet points, no tables, no technical jargon about graphs or nodes.\n\
         - Use markdown ## headings to structure the document.\n\
         - Include specific facts and evidence from the data.\n\
         - Write as a unified whole — not individual summaries of each topic, but an integrated narrative \
           that draws connections across the entire knowledge base.\n\
         - Write in third person, referring to the knowledge as belonging to '{ant_name}'.{citation_instruction}\n\n\
         Knowledge data:\n\n{raw_insights}",
        ant_name = ant_name,
        user_prompt = user_prompt,
        citation_instruction = citation_instruction,
        raw_insights = raw_insights,
    );

    // Cap prompt size. Claude Code has a 1M token context window, so we can
    // be generous. 100000 chars ~= 25000 tokens — plenty of room for citations
    // and data while leaving space for the AI's response.
    let max_prompt_chars = 100000;
    let prompt = if prompt.len() > max_prompt_chars {
        let truncated = &prompt[..prompt[..max_prompt_chars].rfind('\n').unwrap_or(max_prompt_chars)];
        format!("{}\n\n[... data truncated for length — focus on the topics and beliefs shown above ...]", truncated)
    } else {
        prompt
    };

    // Try claude CLI directly.
    let output = std::process::Command::new("claude")
        .args(["-p", "--max-turns", "1"])
        .arg(&prompt)
        .output();

    match output {
        Ok(o) if o.status.success() => {
            let text = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if text.len() > 100 {
                return text;
            }
        }
        _ => {}
    }

    // Fallback: return raw insights unchanged.
    raw_insights.to_string()
}

/// Post-process AI text to renumber citation codes [cite-xxxx] to [1], [2]...
/// in order of first appearance. Returns the renumbered text and the
/// ordered list of citations for the reference section.
fn renumber_citations(text: &str, all_citations: &[CollectedCitation]) -> (String, Vec<CollectedCitation>) {
    // Build a lookup from cite_id to citation.
    let cite_map: std::collections::HashMap<&str, &CollectedCitation> = all_citations.iter()
        .map(|c| (c.cite_id.as_str(), c))
        .collect();

    // Find all [cite-xxxx] references in order of appearance.
    let mut ordered: Vec<CollectedCitation> = Vec::new();
    let mut id_to_num: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut result = text.to_string();

    // Regex-like scan for [cite-xxxx] patterns.
    let mut pos = 0;
    while let Some(start) = result[pos..].find("[cite-") {
        let abs_start = pos + start;
        if let Some(end) = result[abs_start..].find(']') {
            let cite_id = &result[abs_start + 1..abs_start + end].to_string();
            if !id_to_num.contains_key(cite_id.as_str()) {
                let num = ordered.len() + 1;
                id_to_num.insert(cite_id.clone(), num);
                if let Some(&cite) = cite_map.get(cite_id.as_str()) {
                    ordered.push(cite.clone());
                }
            }
            pos = abs_start + end + 1;
        } else {
            break;
        }
    }

    // Replace all [cite-xxxx] with [N] using the numbering.
    for (cite_id, num) in &id_to_num {
        result = result.replace(&format!("[{}]", cite_id), &format!("[{}]", num));
    }

    (result, ordered)
}

/// Format a list as "A, B, and C" (Oxford comma).
fn format_list(items: &[String]) -> String {
    match items.len() {
        0 => String::new(),
        1 => items[0].clone(),
        2 => format!("{} and {}", items[0], items[1]),
        _ => {
            let last = &items[items.len() - 1];
            let rest: Vec<&str> = items[..items.len() - 1].iter().map(|s| s.as_str()).collect();
            format!("{}, and {}", rest.join(", "), last)
        }
    }
}

/// Export a single named graph.
pub fn export_single_graph(memory_dir: &Path, ant_name: &str, graph_name: &str, output_path: &Path, guidance: Option<&str>, include_citations: bool) -> anyhow::Result<()> {
    let store = LiveKnowledgeStore::new(memory_dir.to_path_buf());

    let mut all_data = Vec::new();
    if let Ok(viz) = store.to_visualization(graph_name) {
        let stats = store.graph_stats(graph_name).ok();
        all_data.push(serde_json::json!({
            "name": graph_name,
            "node_count": stats.as_ref().map(|s| s.node_count).unwrap_or(0),
            "edge_count": stats.as_ref().map(|s| s.edge_count).unwrap_or(0),
            "data": viz,
        }));
    }

    let title = format!("{} — {}", ant_name, graph_name.replace('-', " "));
    generate_export_html(&all_data, &title, output_path, guidance, include_citations)
}

/// Export all graphs for an ANT.
pub fn export_ant_graphs(memory_dir: &Path, ant_name: &str, output_path: &Path, guidance: Option<&str>, include_citations: bool) -> anyhow::Result<()> {
    let store = LiveKnowledgeStore::new(memory_dir.to_path_buf());

    let graphs = store.list_graphs()?;
    let mut all_data = Vec::new();

    for g in &graphs {
        // Skip the citations graph in "export all" — it's an internal index,
        // not a topic. Citations are already attached to edges in topic graphs.
        if g.name == "citations" || g.name == "uncategorised" { continue; }
        if let Ok(viz) = store.to_visualization(&g.name) {
            all_data.push(serde_json::json!({
                "name": g.name,
                "node_count": g.node_count,
                "edge_count": g.edge_count,
                "data": viz,
            }));
        }
    }

    generate_export_html(&all_data, ant_name, output_path, guidance, include_citations)
}

fn generate_export_html(all_data: &[serde_json::Value], title: &str, output_path: &Path, guidance: Option<&str>, include_citations: bool) -> anyhow::Result<()> {
    let ant_name = title;

    let snapshot_id = format!("{:08x}", std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs());

    let insights = compute_insights(all_data);
    let raw_insights_html = render_insights_html(&insights, ant_name, &snapshot_id);

    // Build the raw text for the AI. Citations go FIRST so they're never truncated.
    let mut raw_text = format!("Knowledge summary for {}\n\n", ant_name);

    // CITATIONS FIRST — sorted by quality (highest first), capped at half the prompt budget.
    if include_citations && !insights.all_citations.is_empty() {
        raw_text.push_str("SOURCES AND REFERENCES — USE THESE IN YOUR TEXT.\n\
            Each source below has a code like [cite-a1b2c3d4]. Place these codes in your text \
            immediately after any claim that the source supports. For example:\n\
            'The evidence suggests X [cite-a1b2c3d4] and this is consistent with Y [cite-b2c3d4e5].'\n\
            Use as many as are relevant. Do not invent codes not on this list.\n\
            Sources are listed highest quality first.\n\n");
        let mut sorted_cites: Vec<&CollectedCitation> = insights.all_citations.iter().collect();
        sorted_cites.sort_by(|a, b| b.quality.partial_cmp(&a.quality).unwrap_or(std::cmp::Ordering::Equal));
        let citation_budget = 50000; // half of the 100K prompt budget
        let mut citation_chars = 0usize;
        let mut included = 0usize;
        for cite in &sorted_cites {
            let line = format!("  [{}] {} — {}{}{} (supports: {})\n",
                cite.cite_id,
                if cite.title.is_empty() { &cite.url } else { &cite.title },
                if cite.author.is_empty() { String::new() } else { format!("by {}. ", cite.author) },
                if cite.date.is_empty() { String::new() } else { format!("({}). ", cite.date) },
                cite.url,
                cite.supports,
            );
            if citation_chars + line.len() > citation_budget { break; }
            raw_text.push_str(&line);
            citation_chars += line.len();
            included += 1;
        }
        if included < sorted_cites.len() {
            raw_text.push_str(&format!("  [{} lower-quality sources omitted]\n", sorted_cites.len() - included));
        }
        raw_text.push('\n');
    }

    // Then the knowledge data.
    raw_text.push_str(&format!("Total: {} concepts, {} relationships, {:.0}% average confidence\n\n",
        insights.total_nodes, insights.total_edges, insights.avg_confidence * 100.0));
    for (name, nodes, edges, avg) in &insights.topic_summaries {
        if *nodes == 0 { continue; }
        let desc = insights.topic_descriptions.get(name.as_str()).cloned().unwrap_or_default();
        raw_text.push_str(&format!("Topic: {} ({} concepts, {} relationships, {:.0}% confidence)\n",
            name.replace('-', " "), nodes, edges, avg * 100.0));
        if !desc.is_empty() { raw_text.push_str(&format!("  Description: {}\n", desc)); }
    }
    raw_text.push_str("\nStrongest beliefs:\n");
    for (from, to, rel, conf) in insights.strongest_beliefs.iter().take(10) {
        let desc = insights.node_summaries.get(from.as_str()).cloned().unwrap_or_default();
        raw_text.push_str(&format!("  {} {} {} ({:.0}%){}\n", from, rel, to, conf * 100.0,
            if desc.is_empty() { String::new() } else { format!(" — {}", if desc.len() > 150 { &desc[..150] } else { &desc }) }));
    }
    raw_text.push_str("\nWeakest beliefs (need investigation):\n");
    for (from, to, rel, conf) in insights.weakest_beliefs.iter().take(7) {
        raw_text.push_str(&format!("  {} {} {} ({:.0}%)\n", from, rel, to, conf * 100.0));
    }
    raw_text.push_str("\nMost connected concepts:\n");
    for (label, count) in insights.most_connected.iter().take(7) {
        let desc = insights.node_summaries.get(label.as_str()).cloned().unwrap_or_default();
        raw_text.push_str(&format!("  {} ({} connections){}\n", label, count,
            if desc.is_empty() { String::new() } else { format!(" — {}", if desc.len() > 150 { &desc[..150] } else { &desc }) }));
    }
    raw_text.push_str("\nKey entity descriptions:\n");
    for (label, summary) in insights.node_summaries.iter().take(15) {
        let short = if summary.len() > 200 { &summary[..200] } else { summary.as_str() };
        raw_text.push_str(&format!("  {}: {}\n", label, short));
    }

    // Ask AI to polish into readable prose.
    let polished = ai_polish_summary(&raw_text, ant_name, guidance);
    let mut ordered_refs: Vec<CollectedCitation> = Vec::new();
    let insights_html = if polished != raw_text {
        // Post-process: renumber cite-xxxx codes to [1], [2]... in order of appearance.
        let (renumbered_text, refs) = renumber_citations(&polished, &insights.all_citations);
        ordered_refs = refs;
        let renumbered = renumbered_text;

        // AI produced a polished version — wrap in HTML.
        let mut html = format!(
            "<h2>{} — Knowledge Summary</h2>\n\
             <p style='color:#64748b;font-size:13px'>Snapshot {} | Generated {}</p>\n\
             <div style='background:#1e293b;border-radius:8px;padding:24px 28px;margin:20px 0;line-height:1.8;font-size:14px'>\n",
            ant_name, snapshot_id, crate::dateutil::datetime_now()
        );
        // Build a lookup from normalized topic names to graph indices for "View graph" links.
        let topic_to_idx: std::collections::HashMap<String, usize> = insights.topic_summaries.iter()
            .enumerate()
            .map(|(i, (name, _, _, _))| (name.replace('-', " ").to_lowercase(), i))
            .collect();

        // Try to find a matching graph index for a heading.
        let find_graph_link = |heading: &str| -> String {
            let h = heading.to_lowercase();
            for (topic, idx) in &topic_to_idx {
                if h.contains(topic.as_str()) {
                    return format!(
                        " <a href='#' onclick='showTab(\"graph\",{});return false;' \
                         style='font-size:12px;color:#60a5fa;text-decoration:none;margin-left:8px'>View graph →</a>",
                        idx
                    );
                }
            }
            String::new()
        };

        // Convert markdown-ish AI output to HTML paragraphs.
        for line in renumbered.lines() {
            let line = line.trim();
            if line.is_empty() { continue; }
            if line.starts_with("# ") {
                let text = &line[2..];
                html.push_str(&format!("<h3 style='margin-top:20px'>{}{}</h3>\n", text, find_graph_link(text)));
            } else if line.starts_with("## ") {
                let text = &line[3..];
                html.push_str(&format!("<h3 style='margin-top:20px'>{}{}</h3>\n", text, find_graph_link(text)));
            } else if line.starts_with("### ") {
                let text = &line[4..];
                html.push_str(&format!("<h4 style='margin-top:16px'>{}{}</h4>\n", text, find_graph_link(text)));
            } else if line.starts_with("**") && line.ends_with("**") {
                let text = &line[2..line.len()-2];
                html.push_str(&format!("<h4 style='margin-top:16px'>{}{}</h4>\n", text, find_graph_link(text)));
            } else {
                // Convert inline **bold** to <b>.
                let line = line.replace("**", "<b>").replace("**", "</b>");
                html.push_str(&format!("<p>{}</p>\n", line));
            }
        }
        html.push_str("<p style='margin-top:20px;color:#94a3b8;font-size:13px'>This summary was written by an AI reasoning agent based on its knowledge graph. \
            The interactive 3D visualisation (Graph tab) allows exploration of individual concepts. \
            Generated by <a href='https://github.com/reality2-ai/anthill' style='color:#60a5fa'>Anthill</a>.</p>\n");
        html.push_str("</div>\n");
        html
    } else {
        // Fallback to algorithmic version.
        raw_insights_html
    };

    // Append reference list — ordered by first appearance in the document.
    let insights_html = if !include_citations || (ordered_refs.is_empty() && insights.all_citations.is_empty()) {
        insights_html
    } else {
        let mut with_refs = insights_html;
        with_refs.push_str("<div style='background:#1e293b;border-radius:8px;padding:24px 28px;margin:20px 0;line-height:1.6;font-size:13px'>\n");
        with_refs.push_str("<h3>References</h3>\n");
        with_refs.push_str("<ol style='padding-left:20px'>\n");
        // Use the ordered list from renumbering (in order of first appearance).
        let refs_to_show = if !ordered_refs.is_empty() { &ordered_refs } else { &insights.all_citations };
        for cite in refs_to_show {
            let mut entry = String::new();
            if !cite.author.is_empty() { entry.push_str(&format!("{}. ", cite.author)); }
            if !cite.title.is_empty() {
                if !cite.url.is_empty() {
                    entry.push_str(&format!("<a href='{}' style='color:#60a5fa'>{}</a>", cite.url, cite.title));
                } else {
                    entry.push_str(&format!("<em>{}</em>", cite.title));
                }
            } else if !cite.url.is_empty() {
                entry.push_str(&format!("<a href='{}' style='color:#60a5fa'>{}</a>", cite.url, cite.url));
            }
            if !cite.date.is_empty() { entry.push_str(&format!(" ({})", cite.date)); }
            entry.push_str(&format!(" <span style='color:#64748b'>[{}]</span>", cite.ref_type));
            with_refs.push_str(&format!("<li>{}</li>\n", entry));
        }
        with_refs.push_str("</ol>\n</div>\n");
        with_refs
    };

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
<script>{forcegraph_2d_js}</script>
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
#graph-view {{ width: 100%; height: 80vh; display: none; }}
#insights-view {{ max-width: 900px; margin: 0 auto; padding: 30px 20px; }}
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
  border: 1px solid #334155; border-radius: 8px; padding: 10px 14px; font-size: 12px; z-index: 100; display: none; }}
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
    <button class="active" onclick="showTab('insights')">Insights</button>
    <button onclick="showTab('graph')">{graph_tab_label}</button>
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

function showTab(tab, graphIdx) {{
  document.getElementById('graph-view').style.display = tab === 'graph' ? 'block' : 'none';
  document.getElementById('insights-view').style.display = tab === 'insights' ? 'block' : 'none';
  document.getElementById('legend').style.display = tab === 'graph' ? 'block' : 'none';
  document.querySelectorAll('#tabs button').forEach(b => b.classList.remove('active'));
  document.querySelectorAll('#tabs button').forEach(b => {{
    if ((tab === 'graph' && b.textContent === 'Graph') || (tab === 'insights' && b.textContent === 'Insights'))
      b.classList.add('active');
  }});
  if (tab === 'graph') {{
    if (graphIdx !== undefined) {{
      document.getElementById('selector').value = graphIdx;
      loadGraph(graphIdx);
    }} else if (!graphInstance) {{
      loadGraph(document.getElementById('selector').value);
    }}
  }}
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

function hasWebGL() {{
  try {{
    const c = document.createElement('canvas');
    return !!(c.getContext('webgl2') || c.getContext('webgl'));
  }} catch(_) {{ return false; }}
}}

function nodeClickHandler(node, data) {{
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
}}

const linkColor=l=>{{ if(l.is_orphan_link)return'#888'; if(l.confidence>=0.8)return'#4ade80'; if(l.confidence>=0.5)return'#fbbf24'; if(l.confidence>=0.3)return'#fb923c'; return'#f87171'; }};

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

  if (hasWebGL()) {{
    graphInstance = ForceGraph3D()(container).graphData(data)
      .nodeLabel(n=>'<div style="background:rgba(15,23,42,0.95);padding:6px 10px;border-radius:6px;font-size:13px;color:#e2e8f0"><b>'+n.label+'</b> ('+n.kind+')<br>'+(n.summary||'')+'</div>')
      .nodeColor(n=>{{ const c=n.confidence!==undefined?n.confidence:0.5; const a=Math.max(0.15,c); const h=NODE_COLORS[n.kind]||'#888'; const r=parseInt(h.slice(1,3),16)||136; const gg=parseInt(h.slice(3,5),16)||136; const b=parseInt(h.slice(5,7),16)||136; return 'rgba('+r+','+gg+','+b+','+a+')'; }})
      .nodeOpacity(1).nodeVal(n=>n.is_hub?6:3).nodeResolution(12);
    if(typeof SpriteText!=='undefined'){{
      graphInstance.nodeThreeObjectExtend(true).nodeThreeObject(n=>{{ const s=new SpriteText(n.label); s.color=NODE_COLORS[n.kind]||'#ccc'; s.textHeight=2.5; s.position.set(0,5,0); return s; }})
      .linkThreeObjectExtend(true).linkThreeObject(l=>{{ const s=new SpriteText(l.relation); s.color='#999'; s.textHeight=1.5; return s; }})
      .linkPositionUpdate((s,{{start,end}})=>{{ if(s&&s.position&&start&&end) Object.assign(s.position,{{x:start.x+(end.x-start.x)/2,y:start.y+(end.y-start.y)/2,z:start.z+(end.z-start.z)/2}}); }});
    }}
    graphInstance.linkWidth(l=>l.is_orphan_link?0.3:Math.max(0.5,l.confidence*1.5)).linkOpacity(0.6)
      .linkColor(linkColor)
      .linkDirectionalArrowLength(6).linkDirectionalArrowRelPos(0.95)
      .linkDirectionalArrowColor(linkColor)
      .backgroundColor('#0f172a').enableNodeDrag(true)
      .onNodeClick(node=>nodeClickHandler(node, data))
      .warmupTicks(100).cooldownTime(3000);
    setTimeout(()=>{{ if(graphInstance) graphInstance.zoomToFit(800,60); }},500);
  }} else if (typeof ForceGraph !== 'undefined') {{
    // 2D Canvas fallback.
    graphInstance = ForceGraph()(container).graphData(data)
      .nodeLabel(n=>'<div style="background:rgba(15,23,42,0.95);padding:6px 10px;border-radius:6px;font-size:13px;color:#e2e8f0"><b>'+n.label+'</b> ('+n.kind+')<br>'+(n.summary||'')+'</div>')
      .nodeColor(n=>NODE_COLORS[n.kind]||'#888')
      .nodeVal(n=>n.is_hub?12:5)
      .nodeCanvasObject((n,ctx,gs)=>{{
        const sz=n.is_hub?6:3; const col=NODE_COLORS[n.kind]||'#888';
        ctx.beginPath(); ctx.arc(n.x,n.y,sz,0,2*Math.PI);
        ctx.fillStyle=col; ctx.globalAlpha=n.confidence!==undefined?Math.max(0.3,n.confidence):0.7;
        ctx.fill(); ctx.globalAlpha=1;
        if(gs>0.8){{ ctx.font=Math.max(12/gs,3)+'px sans-serif'; ctx.textAlign='center'; ctx.textBaseline='top'; ctx.fillStyle=col; ctx.fillText(n.label,n.x,n.y+sz+2); }}
      }})
      .linkWidth(l=>l.is_orphan_link?0.3:Math.max(0.5,l.confidence*1.5))
      .linkColor(linkColor)
      .linkDirectionalArrowLength(6).linkDirectionalArrowRelPos(0.95)
      .backgroundColor('#0f172a')
      .onNodeClick(node=>nodeClickHandler(node, data))
      .warmupTicks(50).cooldownTime(3000);
    setTimeout(()=>{{ if(graphInstance) graphInstance.zoomToFit(400,40); }},800);
  }} else {{
    container.innerHTML='<div style="color:#64748b;text-align:center;padding:100px">Graph rendering unavailable.</div>';
  }}
}}

function searchNodes(q) {{
  if(!currentData||!graphInstance) return;
  if(!q.trim()){{ graphInstance.nodeColor(n=>{{ const c=n.confidence!==undefined?n.confidence:0.5; const a=Math.max(0.15,c); const h=NODE_COLORS[n.kind]||'#888'; const r=parseInt(h.slice(1,3),16)||136; const g=parseInt(h.slice(3,5),16)||136; const b=parseInt(h.slice(5,7),16)||136; return'rgba('+r+','+g+','+b+','+a+')'; }}); return; }}
  const ql=q.toLowerCase();
  graphInstance.nodeColor(n=>{{ const m=n.label.toLowerCase().includes(ql)||(n.summary||'').toLowerCase().includes(ql)||(n.tags||[]).some(t=>t.toLowerCase().includes(ql));
    return m?(NODE_COLORS[n.kind]||'#fff'):'rgba(100,100,100,0.1)'; }});
}}

// Graph loads on demand when the user clicks the Graph tab.
</script>
</body></html>"##,
        ant_name = ant_name,
        snapshot_id = snapshot_id,
        timestamp = timestamp,
        insights_html = insights_html,
        graphs_json = graphs_json,
        version = env!("CARGO_PKG_VERSION"),
        graph_tab_label = if all_data.len() > 1 { "Graphs" } else { "Graph" },
        three_js = THREE_JS,
        spritetext_js = SPRITETEXT_JS,
        forcegraph_js = FORCEGRAPH_JS,
        forcegraph_2d_js = FORCEGRAPH_2D_JS,
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

