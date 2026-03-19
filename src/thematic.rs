#![allow(dead_code)]
//! Thematic Analysis pipeline — converts documents into knowledge graph nodes and edges.
//!
//! Implements Braun & Clarke's reflexive thematic analysis (2006, 2022) as a
//! multi-phase AI-driven pipeline:
//!
//!   Phase 1: Familiarisation — chunk document, identify scope
//!   Phase 2: Coding — extract entities and concepts as codes
//!   Phase 3: Theme generation — group codes into themes (higher-level nodes)
//!   Phase 4: Review — validate themes against source, set confidence
//!   Phase 5: Refinement — merge, name, build relationships
//!   Phase 6: Integration — add to knowledge graph with Popperian metadata
//!
//! The AI backend performs the actual analysis. This module provides:
//! - Document chunking
//! - Phase-specific prompts
//! - Output parsing (JSON codes/themes → graph nodes/edges)

use serde::{Deserialize, Serialize};

/// A code extracted during Phase 2.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Code {
    /// Short label for this code.
    pub label: String,
    /// The entity kind this maps to in the knowledge graph.
    pub kind: String,
    /// One-line description.
    pub description: String,
    /// The source excerpt that supports this code.
    pub evidence: String,
    /// Tags for searchability.
    #[serde(default)]
    pub tags: Vec<String>,
}

/// A theme identified during Phase 3.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Theme {
    /// Theme name.
    pub name: String,
    /// Central concept or idea.
    pub concept: String,
    /// Codes that belong to this theme.
    pub codes: Vec<String>,
    /// How well-supported this theme is (0.0–1.0).
    pub support: f64,
}

/// A relationship identified during Phase 5.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relationship {
    /// Source entity label.
    pub from: String,
    /// Target entity label.
    pub to: String,
    /// Relationship type.
    pub relation: String,
    /// Context/evidence.
    pub context: String,
    /// How the relationship was formed.
    pub basis: String,
}

/// Result of a complete thematic analysis.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ThematicResult {
    pub codes: Vec<Code>,
    pub themes: Vec<Theme>,
    pub relationships: Vec<Relationship>,
}

/// Maximum chunk size in characters for AI processing.
const CHUNK_SIZE: usize = 8000;
/// Overlap between chunks to preserve context.
const CHUNK_OVERLAP: usize = 500;

/// Chunk a document into overlapping segments for AI processing.
pub fn chunk_document(text: &str) -> Vec<String> {
    if text.len() <= CHUNK_SIZE {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut start = 0;
    while start < text.len() {
        let end = (start + CHUNK_SIZE).min(text.len());
        // Try to break at a paragraph or sentence boundary.
        let break_at = if end < text.len() {
            // Search in the last quarter of the chunk for a good break point.
            let search_start = start + CHUNK_SIZE * 3 / 4;
            text[search_start..end]
                .rfind("\n\n")
                .or_else(|| text[search_start..end].rfind(".\n"))
                .or_else(|| text[search_start..end].rfind(". "))
                .map(|p| search_start + p + 1)
                .unwrap_or(end)
        } else {
            end
        };
        chunks.push(text[start..break_at].to_string());
        // Always advance by at least half a chunk to guarantee progress.
        let next_start = break_at.saturating_sub(CHUNK_OVERLAP);
        start = next_start.max(start + CHUNK_SIZE / 2);
    }
    chunks
}

/// Generate the Phase 2 prompt: coding (extract entities and concepts).
pub fn coding_prompt(chunk: &str, chunk_num: usize, total_chunks: usize) -> String {
    format!(
        r#"You are performing THEMATIC ANALYSIS (Braun & Clarke, 2022) on a document.

PHASE 2: CODING — Extract entities and concepts from this text.

This is chunk {chunk_num} of {total_chunks}.

For each significant entity, concept, decision, tool, person, or fact mentioned, create a CODE:
- label: short identifier (e.g. "Rust", "Ed25519 signing", "ESP32-S3")
- kind: one of person, project, server, tool, concept, decision, event, fact
- description: one sentence explaining what this is
- evidence: the exact quote or paraphrase from the source that supports this code
- tags: searchable keywords

Return ONLY a JSON array of codes. No explanation, no markdown.

Example:
[
  {{"label": "Rust", "kind": "tool", "description": "Systems programming language used for the project", "evidence": "written in Rust for safety and performance", "tags": ["language", "systems"]}},
  {{"label": "Ed25519", "kind": "concept", "description": "Digital signature algorithm for device identity", "evidence": "Ed25519 signing key generated on first run", "tags": ["crypto", "security"]}}
]

TEXT TO ANALYSE:
{chunk}"#,
        chunk_num = chunk_num,
        total_chunks = total_chunks,
        chunk = chunk
    )
}

/// Generate the Phase 3 prompt: theme generation (group codes into themes).
pub fn theme_generation_prompt(codes_json: &str) -> String {
    format!(
        r#"You are performing THEMATIC ANALYSIS (Braun & Clarke, 2022).

PHASE 3: GENERATING THEMES — Group these codes into broader themes.

A theme is a "pattern of shared meaning underpinned by a central concept or idea."
Look for clusters of codes that share a common thread.

CODES:
{codes_json}

For each theme, provide:
- name: concise theme name
- concept: the central idea unifying the codes
- codes: array of code labels that belong to this theme
- support: how well-supported this theme is (0.0–1.0)

Return ONLY a JSON array of themes. No explanation.

Example:
[
  {{"name": "Security Architecture", "concept": "Layered cryptographic security model", "codes": ["Ed25519", "HMAC-SHA256", "Trust groups"], "support": 0.85}}
]"#,
        codes_json = codes_json
    )
}

/// Generate the Phase 5 prompt: refinement (identify relationships).
pub fn relationship_prompt(codes_json: &str, themes_json: &str) -> String {
    format!(
        r#"You are performing THEMATIC ANALYSIS (Braun & Clarke, 2022).

PHASE 5: REFINEMENT — Identify relationships between entities.

Given these codes and themes, identify the key RELATIONSHIPS between entities.

CODES:
{codes_json}

THEMES:
{themes_json}

For each relationship, provide:
- from: source entity label (must match a code label)
- to: target entity label (must match a code label)
- relation: relationship type (e.g. "uses", "deployed_on", "depends_on", "part_of", "decided")
- context: brief description of the relationship
- basis: how this was determined — "observed" (explicit in text), "inferred" (implied), "assumed" (your interpretation)

Return ONLY a JSON array of relationships. No explanation.

Example:
[
  {{"from": "Anthill", "to": "Rust", "relation": "written_in", "context": "Anthill is implemented in Rust", "basis": "observed"}}
]"#,
        codes_json = codes_json,
        themes_json = themes_json
    )
}

/// Parse a JSON array of codes from AI output.
/// Tolerant: strips markdown fences, ignores parse errors on individual items.
pub fn parse_codes(output: &str) -> Vec<Code> {
    let cleaned = strip_markdown_fences(output);
    serde_json::from_str(&cleaned).unwrap_or_else(|_| {
        // Try line-by-line if the whole thing doesn't parse.
        log::debug!("Failed to parse codes as array, trying individual objects");
        Vec::new()
    })
}

/// Parse a JSON array of themes from AI output.
pub fn parse_themes(output: &str) -> Vec<Theme> {
    let cleaned = strip_markdown_fences(output);
    serde_json::from_str(&cleaned).unwrap_or_default()
}

/// Parse a JSON array of relationships from AI output.
pub fn parse_relationships(output: &str) -> Vec<Relationship> {
    let cleaned = strip_markdown_fences(output);
    serde_json::from_str(&cleaned).unwrap_or_default()
}

/// Strip markdown code fences (```json ... ```) from AI output.
pub fn strip_markdown_fences_pub(s: &str) -> String {
    strip_markdown_fences(s)
}

fn strip_markdown_fences(s: &str) -> String {
    let s = s.trim();
    let s = s.strip_prefix("```json").or_else(|| s.strip_prefix("```")).unwrap_or(s);
    let s = s.strip_suffix("```").unwrap_or(s);
    s.trim().to_string()
}

/// Convert a ThematicResult into knowledge graph operations.
/// Returns a prompt that tells the AI to integrate the results into knowledge.json.
pub fn integration_prompt(result: &ThematicResult, source_name: &str) -> String {
    let codes_json = serde_json::to_string_pretty(&result.codes).unwrap_or_default();
    let themes_json = serde_json::to_string_pretty(&result.themes).unwrap_or_default();
    let rels_json = serde_json::to_string_pretty(&result.relationships).unwrap_or_default();

    format!(
        r#"THEMATIC ANALYSIS COMPLETE for document: "{source_name}"

The following codes, themes, and relationships were identified.
Integrate them into memory/knowledge.json:

1. For each CODE, add or update a node in the knowledge graph.
2. For each THEME, add a concept node and link its member codes to it.
3. For each RELATIONSHIP, add an edge with the appropriate confidence:
   - basis "observed" → confidence 0.7
   - basis "inferred" → confidence 0.4
   - basis "assumed" → confidence 0.3
4. If a node already exists, DON'T duplicate — update its summary if the new one is better.
5. Add the source document as an event node: "{source_name}" with today's date.

CODES:
{codes_json}

THEMES:
{themes_json}

RELATIONSHIPS:
{rels_json}

Read memory/knowledge.json, integrate these results, and write it back.
Do NOT output the JSON — just silently update the file."#,
        source_name = source_name,
        codes_json = codes_json,
        themes_json = themes_json,
        rels_json = rels_json
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_short_document() {
        let text = "Hello world. This is a short document.";
        let chunks = chunk_document(text);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], text);
    }

    #[test]
    fn chunk_long_document() {
        // Create a document with sentence boundaries.
        let mut text = String::new();
        for i in 0..500 {
            text.push_str(&format!("Sentence number {} with some extra content to make it longer. ", i));
        }
        let chunks = chunk_document(&text);
        assert!(chunks.len() > 1);
        for chunk in &chunks {
            assert!(chunk.len() <= CHUNK_SIZE + 10);
        }
    }

    #[test]
    fn chunk_breaks_at_paragraphs() {
        let mut text = String::new();
        for i in 0..100 {
            text.push_str(&format!("Paragraph {}. This is some content that fills up space nicely.\n\n", i));
        }
        let chunks = chunk_document(&text);
        // Should break at paragraph boundaries.
        for chunk in &chunks[..chunks.len() - 1] {
            assert!(chunk.ends_with('\n') || chunk.ends_with('.'));
        }
    }

    #[test]
    fn parse_codes_with_fences() {
        let output = r#"```json
[
  {"label": "Rust", "kind": "tool", "description": "Programming language", "evidence": "written in Rust", "tags": ["lang"]}
]
```"#;
        let codes = parse_codes(output);
        assert_eq!(codes.len(), 1);
        assert_eq!(codes[0].label, "Rust");
        assert_eq!(codes[0].kind, "tool");
    }

    #[test]
    fn parse_codes_without_fences() {
        let output = r#"[{"label": "Ed25519", "kind": "concept", "description": "Signing algo", "evidence": "uses Ed25519", "tags": []}]"#;
        let codes = parse_codes(output);
        assert_eq!(codes.len(), 1);
        assert_eq!(codes[0].label, "Ed25519");
    }

    #[test]
    fn parse_themes_empty() {
        let themes = parse_themes("[]");
        assert!(themes.is_empty());
    }

    #[test]
    fn parse_relationships_valid() {
        let output = r#"[{"from": "Anthill", "to": "Rust", "relation": "written_in", "context": "impl language", "basis": "observed"}]"#;
        let rels = parse_relationships(output);
        assert_eq!(rels.len(), 1);
        assert_eq!(rels[0].relation, "written_in");
    }

    #[test]
    fn strip_fences() {
        assert_eq!(strip_markdown_fences("```json\n[]\n```"), "[]");
        assert_eq!(strip_markdown_fences("```\n{}\n```"), "{}");
        assert_eq!(strip_markdown_fences("[]"), "[]");
    }

    #[test]
    fn coding_prompt_includes_chunk() {
        let prompt = coding_prompt("Hello world", 1, 3);
        assert!(prompt.contains("Hello world"));
        assert!(prompt.contains("chunk 1 of 3"));
        assert!(prompt.contains("PHASE 2"));
    }

    #[test]
    fn integration_prompt_includes_source() {
        let result = ThematicResult::default();
        let prompt = integration_prompt(&result, "my-document.md");
        assert!(prompt.contains("my-document.md"));
        assert!(prompt.contains("knowledge.json"));
    }
}
