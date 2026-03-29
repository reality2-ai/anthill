//! Lightweight per-turn conversation compaction.
//!
//! When chat history exceeds the threshold, older messages are evicted and
//! a brief summary is added to the conversation graph. This is deliberately
//! minimal — just a turn summary node linked to the hub. Heavy-duty entity
//! extraction and thematic analysis is done by the "Refine" button, which
//! triggers an AI pass.

use crate::history::ChatMessage;

/// Result of compacting evicted messages into a turn summary.
pub struct CompactionResult {
    /// One-line summary of the compacted turn(s).
    pub summary: String,
    /// The messages that were compacted (for optional background AI refinement).
    pub compacted_messages: Vec<ChatMessage>,
}

/// Build a brief summary from messages being compacted.
///
/// This is intentionally lightweight — no entity extraction, no heuristics.
/// Just a concise summary of what was discussed, suitable as a graph node.
pub fn summarise_turn(messages: &[ChatMessage]) -> CompactionResult {
    // Build summary: first sentence of each user message + first sentence of each bot response.
    let mut user_parts = Vec::new();
    let mut bot_parts = Vec::new();

    for m in messages {
        if m.role == "user" {
            let s = first_sentence(&m.text);
            if !s.is_empty() && !user_parts.contains(&s) {
                user_parts.push(s);
            }
        } else if m.role == "bot" {
            let s = first_sentence(&m.text);
            if !s.is_empty() && !bot_parts.contains(&s) {
                bot_parts.push(s);
            }
        }
    }

    // Combine into a concise summary.
    let summary = if user_parts.is_empty() && bot_parts.is_empty() {
        "Conversation turn".to_string()
    } else {
        let user_summary = user_parts.join("; ");
        let bot_summary = bot_parts.join("; ");
        if bot_summary.is_empty() {
            user_summary
        } else if user_summary.is_empty() {
            bot_summary
        } else {
            // Cap total length.
            let combined = format!("{} → {}", user_summary, bot_summary);
            if combined.len() > 300 {
                format!("{}...", &combined[..combined.floor_char_boundary(300)])
            } else {
                combined
            }
        }
    };

    CompactionResult {
        summary,
        compacted_messages: messages.to_vec(),
    }
}

/// Get the first sentence from text.
fn first_sentence(text: &str) -> String {
    let text = text.trim();
    // Skip markdown headers, code blocks, bold markers at the start.
    let text = text.trim_start_matches('#').trim_start_matches('*').trim();

    for (i, c) in text.char_indices() {
        if (c == '.' || c == '!' || c == '?') && i > 10 {
            let next = text.get(i + 1..i + 2).unwrap_or(" ");
            if next == " " || next == "\n" || next.is_empty() {
                let sentence = text[..=i].trim().to_string();
                if sentence.len() > 150 {
                    return format!("{}...", &sentence[..sentence.floor_char_boundary(150)]);
                }
                return sentence;
            }
        }
    }
    // No sentence end found — take first 150 chars.
    if text.len() > 150 {
        format!("{}...", &text[..text.floor_char_boundary(150)])
    } else {
        text.to_string()
    }
}

/// Pre-cleanup the conversation graph before AI refinement.
///
/// Removes nodes that already exist in topic graphs and obvious noise.
/// Keeps turn summaries for the AI to analyse thematically.
/// Returns a summary string describing what was done.
pub fn pre_cleanup_conversation_graph(
    memory_dir: &std::path::Path,
    bot_name: &str,
) -> String {
    use crate::store::KnowledgeStore;

    let store = crate::store::live::LiveKnowledgeStore::new(memory_dir.to_path_buf());
    let graph_name = format!("conversation-{}", bot_name);

    // Collect all node labels from topic graphs.
    let mut topic_labels: std::collections::HashSet<String> = std::collections::HashSet::new();
    if let Ok(graphs) = store.list_graphs() {
        for g in &graphs {
            if g.name == "meta"
                || g.name == "citations"
                || g.name == "uncategorised"
                || g.name.starts_with("conversation-")
            {
                continue;
            }
            if let Ok(labels) = store.list_nodes(&g.name) {
                for label in labels {
                    topic_labels.insert(label.to_lowercase());
                }
            }
        }
    }

    // Get conversation graph nodes.
    let conv_labels = match store.list_nodes(&graph_name) {
        Ok(l) => l,
        Err(_) => return "No conversation graph found.".to_string(),
    };

    let total_before = conv_labels.len();
    let mut to_remove = Vec::new();
    let mut turn_summaries = Vec::new();
    let hub_label = format!("conversation-{}", bot_name);

    for label in &conv_labels {
        // Never remove the hub node.
        if *label == hub_label {
            continue;
        }

        // Keep turn summaries — collect their text for the AI.
        if label.starts_with("turn-") {
            if let Ok(node) = store.get_node(&graph_name, label) {
                turn_summaries.push(node.summary.clone());
            }
            continue;
        }

        // Remove if it exists in a topic graph.
        if topic_labels.contains(&label.to_lowercase()) {
            to_remove.push(label.clone());
            continue;
        }

        // Remove noise.
        if is_noise_label(label) {
            to_remove.push(label.clone());
        }
    }

    // Actually remove the nodes (and their edges) from the graph.
    let removed = store.remove_nodes(&graph_name, &to_remove).unwrap_or(0);
    let remaining = total_before - removed as usize;

    let mut summary = format!(
        "Pre-cleanup of '{}':\n\
         - Started with {} nodes\n\
         - Removed {} nodes (duplicates from topic graphs + noise)\n\
         - {} nodes remaining (including {} turn summaries)\n",
        graph_name, total_before, removed, remaining, turn_summaries.len(),
    );

    if !turn_summaries.is_empty() {
        summary.push_str("\nTurn summaries to analyse:\n");
        for (i, ts) in turn_summaries.iter().enumerate() {
            summary.push_str(&format!("  {}. {}\n", i + 1, ts));
        }
    }

    log::info!(
        "[{}] Conversation graph pre-cleanup: {} → removed {} → {} remaining",
        bot_name, total_before, removed, remaining
    );

    summary
}

/// Check if a label is noise: too short, common word, URL fragment, etc.
fn is_noise_label(label: &str) -> bool {
    let trimmed = label.trim();

    // Too short.
    if trimmed.len() <= 2 {
        return true;
    }

    // Single common word.
    let lower = trimmed.to_lowercase();
    if !lower.contains(' ') && is_common_single_word(&lower) {
        return true;
    }

    // URL fragments.
    if trimmed.contains("](http") || trimmed.contains("](https") {
        return true;
    }

    // Parenthetical fragments like "(-0" or "(0.1".
    if trimmed.starts_with('(') && trimmed.len() < 10 {
        return true;
    }

    // Labels that are just numbers or punctuation.
    if trimmed.chars().all(|c| c.is_ascii_digit() || c.is_ascii_punctuation() || c == ' ') {
        return true;
    }

    false
}

fn is_common_single_word(word: &str) -> bool {
    matches!(
        word,
        "the" | "a" | "an" | "and" | "or" | "but" | "in" | "on" | "at" | "to" | "for"
        | "of" | "with" | "by" | "from" | "is" | "are" | "was" | "were" | "be" | "been"
        | "have" | "has" | "had" | "do" | "does" | "did" | "will" | "would" | "could"
        | "should" | "may" | "might" | "can" | "this" | "that" | "it" | "its" | "not"
        | "no" | "yes" | "if" | "then" | "else" | "when" | "where" | "how" | "what"
        | "which" | "who" | "why" | "so" | "up" | "all" | "each" | "every" | "both"
        | "few" | "more" | "most" | "other" | "some" | "such" | "only" | "same" | "than"
        | "too" | "very" | "just" | "also" | "now" | "here" | "there" | "about" | "after"
        | "before" | "new" | "old" | "one" | "two" | "three" | "first" | "last" | "next"
        | "high" | "low" | "large" | "small" | "long" | "short" | "good" | "bad"
        | "key" | "main" | "full" | "total" | "added" | "noted" | "held" | "won"
        | "used" | "made" | "found" | "based" | "shown" | "given" | "taken" | "known"
        | "changed" | "complete" | "breaking" | "changing" | "starting" | "happening"
        | "absorbed" | "shifting" | "surviving" | "specific" | "dark" | "deep" | "edge"
        | "type" | "design" | "as" | "probably" | "failed" | "weakened" | "claim"
        | "original" | "actual" | "reversed" | "unverified" | "possible" | "doubtful"
        | "query" | "summary" | "stats" | "legend" | "category" | "nodes"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_summarise_turn() {
        let msgs = vec![
            ChatMessage {
                role: "user".into(),
                text: "What is the capital of France? I need to know for my report.".into(),
                task_id: 0,
                timestamp: 0,
                graph_ref: None,
            },
            ChatMessage {
                role: "bot".into(),
                text: "The capital of France is Paris. It has been the capital since the 10th century.".into(),
                task_id: 0,
                timestamp: 0,
                graph_ref: None,
            },
        ];
        let result = summarise_turn(&msgs);
        assert!(result.summary.contains("capital of France"));
        assert!(result.summary.contains("Paris"));
    }

    #[test]
    fn test_first_sentence() {
        assert_eq!(
            first_sentence("Hello world. This is a test."),
            "Hello world."
        );
        assert_eq!(first_sentence("Short"), "Short");
    }
}
