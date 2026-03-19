#![allow(dead_code)]
//! Specification and test vector generation pipeline.
//!
//! Analyses code or documents and generates:
//! 1. Formal specifications (RFC 2119 style, like the ANTHILL-* specs)
//! 2. Testing vectors (concrete inputs, expected outputs, edge cases)
//!
//! Uses the same thematic analysis pattern as document→graph conversion:
//!   Phase 1: Familiarisation — read and chunk the source
//!   Phase 2: Coding — extract behaviors, invariants, contracts
//!   Phase 3: Themes — group into specification sections
//!   Phase 4: Review — validate against source
//!   Phase 5: Refinement — generate formal spec language + test vectors
//!   Phase 6: Integration — write spec files and test stubs

use serde::{Deserialize, Serialize};

/// A behavior or contract extracted from code (Phase 2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Behavior {
    /// Short identifier.
    pub name: String,
    /// What the code does (descriptive).
    pub description: String,
    /// The invariant or contract (normative — MUST/SHOULD/MAY).
    pub invariant: String,
    /// Source evidence (function name, line reference).
    pub source: String,
    /// Severity: "must", "should", "may".
    pub level: String,
    /// Related behaviors.
    #[serde(default)]
    pub related: Vec<String>,
}

/// A specification section (Phase 3).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpecSection {
    /// Section title.
    pub title: String,
    /// Section number (e.g. "3.2").
    pub number: String,
    /// Behaviors in this section.
    pub behaviors: Vec<String>,
    /// Prose description.
    pub description: String,
}

/// A test vector (Phase 5).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestVector {
    /// Which behavior this tests.
    pub behavior: String,
    /// Test name (snake_case, suitable for a #[test] function).
    pub test_name: String,
    /// Human-readable description.
    pub description: String,
    /// Setup / preconditions.
    #[serde(default)]
    pub setup: String,
    /// Input or action.
    pub input: String,
    /// Expected output or state.
    pub expected: String,
    /// Edge case? Normal case?
    pub category: String,
}

/// Result of specification generation.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SpecGenResult {
    pub behaviors: Vec<Behavior>,
    pub sections: Vec<SpecSection>,
    pub test_vectors: Vec<TestVector>,
}

/// Generate the prompt for Phase 2: extract behaviors from code.
pub fn behavior_extraction_prompt(code: &str, file_name: &str) -> String {
    format!(
        r#"You are performing SPECIFICATION EXTRACTION on source code.

PHASE 2: BEHAVIOR EXTRACTION — Identify all behaviors, invariants, and contracts.

Analyse this code from "{file_name}" and extract:

For each significant behavior, provide:
- name: short identifier (e.g. "auth_rejects_empty_credential")
- description: what the code does (descriptive, not normative)
- invariant: the contract in RFC 2119 language (e.g. "The server MUST reject requests with empty credentials")
- source: function name or code reference
- level: "must", "should", or "may"
- related: names of related behaviors

Focus on:
- Input validation and error handling
- State transitions and lifecycle
- Security boundaries and access control
- Data persistence and consistency
- Concurrency and ordering guarantees
- Configuration defaults and overrides

Return ONLY a JSON array of behaviors. No explanation, no markdown.

CODE:
{code}"#,
        file_name = file_name,
        code = code
    )
}

/// Generate the prompt for Phase 3: group behaviors into spec sections.
pub fn spec_structure_prompt(behaviors_json: &str, spec_name: &str) -> String {
    format!(
        r#"You are generating a FORMAL SPECIFICATION from extracted behaviors.

PHASE 3: SPECIFICATION STRUCTURE — Group behaviors into logical sections.

Spec name: "{spec_name}"

BEHAVIORS:
{behaviors_json}

Group these behaviors into specification sections. For each section:
- title: section heading (e.g. "Authentication", "Worker Lifecycle")
- number: section number (e.g. "3.1", "4.2")
- behaviors: array of behavior names that belong here
- description: 2-3 sentence overview of this section

Aim for 4-8 sections that tell a coherent story. Order them logically
(setup → normal operation → error handling → security).

Return ONLY a JSON array of sections. No explanation."#,
        spec_name = spec_name,
        behaviors_json = behaviors_json
    )
}

/// Generate the prompt for Phase 5: create test vectors.
pub fn test_vector_prompt(behaviors_json: &str, spec_name: &str) -> String {
    format!(
        r#"You are generating TEST VECTORS for a specification.

PHASE 5: TEST VECTORS — Create concrete test cases for each behavior.

Spec: "{spec_name}"

BEHAVIORS:
{behaviors_json}

For each behavior, generate 2-3 test vectors:
- One normal/happy path test
- One edge case or boundary test
- One error/negative test (if applicable)

For each test vector:
- behavior: which behavior this tests (must match a behavior name)
- test_name: snake_case name suitable for a Rust #[test] function
- description: human-readable explanation
- setup: preconditions (empty string if none)
- input: what to do / what to feed in
- expected: what should happen
- category: "normal", "edge", "error", or "security"

Return ONLY a JSON array of test vectors. No explanation."#,
        spec_name = spec_name,
        behaviors_json = behaviors_json
    )
}

/// Generate the prompt for Phase 6: write the actual spec document.
pub fn spec_document_prompt(
    spec_name: &str,
    sections_json: &str,
    behaviors_json: &str,
    source_files: &[String],
) -> String {
    format!(
        r#"Generate a FORMAL SPECIFICATION document in Markdown.

Spec name: {spec_name}
Source files analysed: {sources}

Follow the R2-specifications style:
- Header: title, version 0.1 Draft, date, status, dependencies
- RFC 2119 keywords (MUST, SHOULD, MAY, etc.)
- Terminology table
- Numbered sections matching the structure below
- Security considerations section at the end
- Each behavior becomes a normative statement in its section

SECTIONS:
{sections_json}

BEHAVIORS:
{behaviors_json}

Write the complete specification document. Use the section structure provided.
For each behavior, write it as a formal requirement in the appropriate section.
Include examples where helpful.

Output the complete Markdown document."#,
        spec_name = spec_name,
        sources = source_files.join(", "),
        sections_json = sections_json,
        behaviors_json = behaviors_json
    )
}

/// Generate the prompt for writing Rust test stubs from test vectors.
pub fn test_stubs_prompt(test_vectors_json: &str, module_name: &str) -> String {
    format!(
        r#"Generate Rust test code from these test vectors.

Module: {module_name}

TEST VECTORS:
{test_vectors_json}

For each test vector, generate a #[test] function with:
- The test_name as the function name
- A doc comment with the description
- Setup code (if setup is non-empty)
- The test logic implementing the input → expected check
- Use assert!, assert_eq!, or should_panic as appropriate

Output ONLY the Rust code. No explanation. Include:
- #[cfg(test)] mod tests {{ ... }}
- All necessary imports
- TODO comments where actual implementation details are needed"#,
        module_name = module_name,
        test_vectors_json = test_vectors_json
    )
}

/// Parse behaviors from AI output.
pub fn parse_behaviors(output: &str) -> Vec<Behavior> {
    let cleaned = crate::thematic::strip_markdown_fences_pub(output);
    serde_json::from_str(&cleaned).unwrap_or_default()
}

/// Parse spec sections from AI output.
pub fn parse_sections(output: &str) -> Vec<SpecSection> {
    let cleaned = crate::thematic::strip_markdown_fences_pub(output);
    serde_json::from_str(&cleaned).unwrap_or_default()
}

/// Parse test vectors from AI output.
pub fn parse_test_vectors(output: &str) -> Vec<TestVector> {
    let cleaned = crate::thematic::strip_markdown_fences_pub(output);
    serde_json::from_str(&cleaned).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn behavior_prompt_includes_code() {
        let prompt = behavior_extraction_prompt("fn auth() {}", "web.rs");
        assert!(prompt.contains("fn auth() {}"));
        assert!(prompt.contains("web.rs"));
        assert!(prompt.contains("PHASE 2"));
    }

    #[test]
    fn test_vector_prompt_includes_spec() {
        let prompt = test_vector_prompt("[]", "ANTHILL-WEB");
        assert!(prompt.contains("ANTHILL-WEB"));
        assert!(prompt.contains("snake_case"));
    }

    #[test]
    fn parse_behaviors_tolerant() {
        let output = r#"```json
[{"name": "auth_check", "description": "Checks credential", "invariant": "MUST reject empty", "source": "web.rs:448", "level": "must", "related": []}]
```"#;
        let behaviors = parse_behaviors(output);
        assert_eq!(behaviors.len(), 1);
        assert_eq!(behaviors[0].name, "auth_check");
    }

    #[test]
    fn parse_test_vectors_valid() {
        let output = r#"[{"behavior": "auth_check", "test_name": "reject_empty_credential", "description": "Empty cred returns 401", "setup": "", "input": "X-Credential: \"\"", "expected": "401 Unauthorized", "category": "error"}]"#;
        let vectors = parse_test_vectors(output);
        assert_eq!(vectors.len(), 1);
        assert_eq!(vectors[0].category, "error");
    }
}
