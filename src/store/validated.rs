//! Validated builder types for knowledge graph mutations.
//!
//! These types enforce constraints at construction time — if you have a
//! ValidatedNode, it's guaranteed to have valid field values. The AI gets
//! a clear error message through MCP if it provides invalid data.

use crate::knowledge::{Basis, EdgeView, KnowledgeEdge, KnowledgeNode, NodeKind};
use crate::epistemic::EvidenceType;
use crate::store::{StoreError, StoreResult};

// ── ValidatedNode ──────────────────────────────────────────────────

/// A node that has been validated and is ready to insert.
pub struct ValidatedNode {
    pub(crate) inner: KnowledgeNode,
}

impl ValidatedNode {
    /// Create a validated node. Returns an error if any field is invalid.
    pub fn new(
        label: &str,
        kind: &str,
        summary: &str,
        tags: Vec<String>,
    ) -> StoreResult<Self> {
        if label.trim().is_empty() {
            return Err(StoreError::Validation("node label cannot be empty".into()));
        }
        if label.len() > 200 {
            return Err(StoreError::Validation("node label too long (max 200 chars)".into()));
        }

        let kind = parse_node_kind(kind)?;

        let now = today_string();
        Ok(Self {
            inner: KnowledgeNode {
                label: label.trim().into(),
                kind,
                summary: summary.into(),
                created: now.clone(),
                updated: now,
                tags,
                _extra: Default::default(),
            },
        })
    }
}

// ── ValidatedEdge ──────────────────────────────────────────────────

/// An edge that has been validated and is ready to insert.
pub struct ValidatedEdge {
    pub(crate) from_label: String,
    pub(crate) to_label: String,
    pub(crate) inner: KnowledgeEdge,
}

impl ValidatedEdge {
    /// Create a validated edge. Returns an error if any field is invalid.
    pub fn new(
        from: &str,
        to: &str,
        relation: &str,
        context: &str,
        basis: &str,
        view: &str,
        source: &str,
        beneficial_impact: f64,
    ) -> StoreResult<Self> {
        if from.trim().is_empty() {
            return Err(StoreError::Validation("'from' node label cannot be empty".into()));
        }
        if to.trim().is_empty() {
            return Err(StoreError::Validation("'to' node label cannot be empty".into()));
        }
        if relation.trim().is_empty() {
            return Err(StoreError::Validation("relation cannot be empty".into()));
        }

        let basis = parse_basis(basis)?;
        let view = parse_edge_view(view)?;
        let beneficial_impact = beneficial_impact.clamp(-1.0, 1.0);

        let now = today_string();
        let mut edge = KnowledgeEdge::new(relation, context, &now, basis);
        edge.view = view;
        edge.source = source.into();
        edge.source_id = source.into();
        edge.beneficial_impact = beneficial_impact;

        Ok(Self {
            from_label: from.trim().into(),
            to_label: to.trim().into(),
            inner: edge,
        })
    }
}

// ── ValidatedEvidence ──────────────────────────────────────────────

/// Evidence that has been validated and is ready to apply.
pub struct ValidatedEvidence {
    pub(crate) evidence_type: EvidenceType,
    pub(crate) date: String,
    pub(crate) test: String,
    pub(crate) detail: String,
    pub(crate) source_id: String,
    pub(crate) source_reputation: f64,
}

impl ValidatedEvidence {
    /// Create validated evidence. Returns an error if any field is invalid.
    pub fn new(
        evidence_type: &str,
        test: &str,
        detail: &str,
        source_id: &str,
        source_reputation: f64,
    ) -> StoreResult<Self> {
        let evidence_type = parse_evidence_type(evidence_type)?;

        if test.trim().is_empty() {
            return Err(StoreError::Validation("evidence 'test' cannot be empty — describe what was tested".into()));
        }

        let source_reputation = source_reputation.clamp(0.0, 1.0);

        Ok(Self {
            evidence_type,
            date: today_string(),
            test: test.into(),
            detail: detail.into(),
            source_id: source_id.into(),
            source_reputation,
        })
    }
}

// ── Parsing helpers ────────────────────────────────────────────────

fn parse_node_kind(s: &str) -> StoreResult<NodeKind> {
    match s.to_lowercase().trim() {
        "person" => Ok(NodeKind::Person),
        "project" => Ok(NodeKind::Project),
        "server" => Ok(NodeKind::Server),
        "tool" => Ok(NodeKind::Tool),
        "concept" => Ok(NodeKind::Concept),
        "decision" => Ok(NodeKind::Decision),
        "event" => Ok(NodeKind::Event),
        "fact" => Ok(NodeKind::Fact),
        "theory" => Ok(NodeKind::Theory),
        "mechanism" => Ok(NodeKind::Mechanism),
        "principle" => Ok(NodeKind::Principle),
        "constraint" => Ok(NodeKind::Constraint),
        "problem" => Ok(NodeKind::Problem),
        "claim" => Ok(NodeKind::Claim),
        "open_question" => Ok(NodeKind::OpenQuestion),
        "implementation" => Ok(NodeKind::Implementation),
        "entity" => Ok(NodeKind::Entity),
        "spec" => Ok(NodeKind::Spec),
        "repo" => Ok(NodeKind::Repo),
        "platform" => Ok(NodeKind::Platform),
        "framework" => Ok(NodeKind::Framework),
        other => Err(StoreError::Validation(format!(
            "unknown node kind '{}'. Valid kinds: person, project, server, tool, concept, \
             decision, event, fact, theory, mechanism, principle, constraint, problem, \
             claim, open_question, implementation, entity, spec, repo, platform, framework",
            other
        ))),
    }
}

fn parse_basis(s: &str) -> StoreResult<Basis> {
    match s.to_lowercase().trim() {
        "observed" => Ok(Basis::Observed),
        "told" => Ok(Basis::Told),
        "inferred" => Ok(Basis::Inferred),
        "assumed" => Ok(Basis::Assumed),
        other => Err(StoreError::Validation(format!(
            "unknown basis '{}'. Valid values: observed, told, inferred, assumed",
            other
        ))),
    }
}

fn parse_edge_view(s: &str) -> StoreResult<EdgeView> {
    match s.to_lowercase().trim() {
        "semantic" => Ok(EdgeView::Semantic),
        "temporal" => Ok(EdgeView::Temporal),
        "causal" => Ok(EdgeView::Causal),
        "entity" | "" => Ok(EdgeView::Entity),
        other => Err(StoreError::Validation(format!(
            "unknown edge view '{}'. Valid values: semantic, temporal, causal, entity",
            other
        ))),
    }
}

fn parse_evidence_type(s: &str) -> StoreResult<EvidenceType> {
    match s.to_lowercase().trim() {
        "corroboration" => Ok(EvidenceType::Corroboration),
        "contradiction" => Ok(EvidenceType::Contradiction),
        "refutation_survived" => Ok(EvidenceType::RefutationSurvived),
        "refutation_failed" => Ok(EvidenceType::RefutationFailed),
        "human_attestation" => Ok(EvidenceType::HumanAttestation),
        "consistency" => Ok(EvidenceType::Consistency),
        "inconsistency" => Ok(EvidenceType::Inconsistency),
        "synthesis" => Ok(EvidenceType::Synthesis),
        "competition_won" => Ok(EvidenceType::CompetitionWon),
        "competition_lost" => Ok(EvidenceType::CompetitionLost),
        "pattern_transfer" => Ok(EvidenceType::PatternTransfer),
        "inconsequential_search" => Ok(EvidenceType::InconsequentialSearch),
        other => Err(StoreError::Validation(format!(
            "unknown evidence type '{}'. Valid types: corroboration, contradiction, \
             refutation_survived, refutation_failed, human_attestation, consistency, \
             inconsistency, synthesis, competition_won, competition_lost, pattern_transfer, \
             inconsequential_search",
            other
        ))),
    }
}

fn today_string() -> String {
    crate::dateutil::today_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_node_accepts_good_input() {
        let node = ValidatedNode::new("Test Node", "concept", "A test", vec![]);
        assert!(node.is_ok());
    }

    #[test]
    fn empty_label_rejected() {
        let node = ValidatedNode::new("", "concept", "A test", vec![]);
        assert!(matches!(node, Err(StoreError::Validation(_))));
    }

    #[test]
    fn invalid_kind_rejected_with_helpful_message() {
        let node = ValidatedNode::new("Test", "research", "A test", vec![]);
        assert!(matches!(node, Err(StoreError::Validation(msg)) if msg.contains("research") && msg.contains("Valid kinds")));
    }

    #[test]
    fn valid_edge_accepts_good_input() {
        let edge = ValidatedEdge::new("A", "B", "relates_to", "context", "observed", "entity", "test", 0.0);
        assert!(edge.is_ok());
    }

    #[test]
    fn invalid_basis_rejected() {
        let edge = ValidatedEdge::new("A", "B", "relates_to", "", "research", "entity", "", 0.0);
        assert!(matches!(edge, Err(StoreError::Validation(msg)) if msg.contains("research")));
    }

    #[test]
    fn invalid_evidence_type_rejected() {
        let ev = ValidatedEvidence::new("made_up_type", "test", "detail", "src", 0.5);
        assert!(matches!(ev, Err(StoreError::Validation(msg)) if msg.contains("made_up_type")));
    }

    #[test]
    fn empty_evidence_test_rejected() {
        let ev = ValidatedEvidence::new("corroboration", "", "detail", "src", 0.5);
        assert!(matches!(ev, Err(StoreError::Validation(_))));
    }
}
