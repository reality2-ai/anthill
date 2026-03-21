//! Thurisaz-compliant Bayesian epistemic engine.
//!
//! Pure functions implementing TH-WEAVE's sequential Bayesian updating:
//! - Log-odds representation for numerical stability
//! - Reputation-weighted evidence (source reliability modulates strength)
//! - Fading foundations (beliefs decay toward uncertainty without fresh evidence)
//! - Typed evidence with predefined Bayes factors
//!
//! Reference: TH-WEAVE §4.1, §7, §8.3, §10.1

use serde::{Deserialize, Serialize};

// ── Core Bayesian math ──────────────────────────────────────────────

/// Convert probability to log-odds.
pub fn to_log_odds(p: f64) -> f64 {
    let p = p.clamp(0.001, 0.999);
    (p / (1.0 - p)).ln()
}

/// Convert log-odds to probability (sigmoid).
pub fn to_probability(log_odds: f64) -> f64 {
    1.0 / (1.0 + (-log_odds).exp())
}

/// Sequential Bayesian update in log-odds space.
/// Clamps to [0.001, 0.999] in probability space (±6.9 log-odds).
pub fn bayesian_update(log_odds: f64, bayes_factor: f64) -> f64 {
    if bayes_factor <= 0.0 {
        return log_odds; // Invalid BF — no update
    }
    (log_odds + bayes_factor.ln()).clamp(-6.9, 6.9)
}

/// Reputation-weighted Bayes factor (TH-WEAVE §7.1).
///
/// w(r) = 0.5 + 0.5 × r, so:
/// - r=0 (untrusted): BF is square-rooted (dampened)
/// - r=0.5 (neutral): BF^0.75
/// - r=1.0 (fully trusted): full BF
///
/// Reputation can only attenuate, never amplify beyond base BF.
pub fn reputation_adjusted_bf(bf_base: f64, reputation: f64) -> f64 {
    let r = reputation.clamp(0.0, 1.0);
    let w = 0.5 + 0.5 * r;
    bf_base.powf(w)
}

/// Fading foundations: belief decays toward p=0.5 (log-odds=0) over time.
/// log_odds(t) = log_odds(t_last) × 2^(-elapsed / half_life)
///
/// This resolves Agrippa's trilemma: you don't need a foundation if
/// foundations fade (Peijnenburg & Atkinson, 2017).
pub fn decay(log_odds: f64, elapsed_secs: f64, half_life_secs: f64) -> f64 {
    if half_life_secs <= 0.0 || elapsed_secs <= 0.0 {
        return log_odds;
    }
    log_odds * 2.0_f64.powf(-elapsed_secs / half_life_secs)
}

// ── Evidence types ──────────────────────────────────────────────────

/// Typed evidence for Bayesian updates.
/// Each type has a predefined base Bayes factor from TH-WEAVE §3.2,
/// adapted for Anthill's AI-knowledge context.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceType {
    /// Found supporting evidence in another source. BF = 2.0 × r
    Corroboration,
    /// Found contradicting evidence. BF = 0.3 / r
    Contradiction,
    /// Actively tried to disprove, claim held. BF = 2.5
    RefutationSurvived,
    /// Actively tried to disprove, claim failed. BF = 0.1
    RefutationFailed,
    /// User confirms or corrects. BF = 1.5 × r
    HumanAttestation,
    /// Consistent with existing graph. BF = 1.5
    Consistency,
    /// Inconsistent with existing graph. BF = 0.4
    Inconsistency,
    /// Transitive inference from two strong edges. BF = 1.2 (weak positive)
    Synthesis,
    /// Won a competition against a rival hypothesis. BF = 2.0
    CompetitionWon,
    /// Lost a competition against a rival hypothesis. BF = 0.3
    CompetitionLost,
    /// Cross-domain pattern transfer: insight from one domain strengthens this. BF = 1.8
    PatternTransfer,
    /// Searched for counter-evidence but found nothing relevant. BF = 1.0 (no change).
    /// Absence of evidence is NOT evidence of absence. Only active, failed refutation
    /// strengthens a belief — merely not finding anything proves nothing.
    InconsequentialSearch,
}

impl EvidenceType {
    /// Base Bayes factor for this evidence type.
    /// For reputation-dependent types, this is the coefficient (multiply by r).
    pub fn base_bayes_factor(&self) -> f64 {
        match self {
            Self::Corroboration => 2.0,
            Self::Contradiction => 0.3,
            Self::RefutationSurvived => 2.5,
            Self::RefutationFailed => 0.1,
            Self::HumanAttestation => 1.5,
            Self::Consistency => 1.5,
            Self::Inconsistency => 0.4,
            Self::Synthesis => 1.2,
            Self::CompetitionWon => 2.0,
            Self::CompetitionLost => 0.3,
            Self::PatternTransfer => 1.8,
            Self::InconsequentialSearch => 1.0, // No change — absence proves nothing
        }
    }

    /// Whether this evidence type's BF is modulated by source reputation.
    #[allow(dead_code)]
    pub fn is_reputation_dependent(&self) -> bool {
        matches!(
            self,
            Self::Corroboration | Self::Contradiction | Self::HumanAttestation
        )
    }

    /// Compute the effective Bayes factor given source reputation.
    /// For reputation-independent types, reputation is ignored.
    pub fn effective_bayes_factor(&self, reputation: f64) -> f64 {
        let r = reputation.clamp(0.1, 1.0);
        match self {
            Self::Corroboration => reputation_adjusted_bf(2.0 * r, reputation),
            Self::Contradiction => {
                // BF = 0.3 / r — lower reputation makes contradiction weaker evidence
                let bf = (0.3 / r).clamp(0.01, 10.0);
                reputation_adjusted_bf(bf, reputation)
            }
            Self::HumanAttestation => reputation_adjusted_bf(1.5 * r, reputation),
            // Non-reputation types: apply reputation dampening to base BF
            _ => reputation_adjusted_bf(self.base_bayes_factor(), reputation),
        }
    }

    /// Human-readable description.
    pub fn description(&self) -> &'static str {
        match self {
            Self::Corroboration => "Supporting evidence found in another source",
            Self::Contradiction => "Contradicting evidence found",
            Self::RefutationSurvived => "Claim survived active disproof attempt",
            Self::RefutationFailed => "Claim failed active disproof attempt",
            Self::HumanAttestation => "User confirmed or corrected",
            Self::Consistency => "Consistent with existing knowledge graph",
            Self::Inconsistency => "Inconsistent with existing knowledge graph",
            Self::Synthesis => "Transitive inference from two strong edges",
            Self::CompetitionWon => "Won competition against a rival hypothesis",
            Self::CompetitionLost => "Lost competition against a rival hypothesis",
            Self::PatternTransfer => "Cross-domain pattern transfer strengthened this idea",
            Self::InconsequentialSearch => "Searched for counter-evidence but found nothing relevant",
        }
    }
}

/// A piece of evidence applied to an edge.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    /// When this evidence was observed.
    pub date: String,
    /// What type of evidence this is.
    pub evidence_type: EvidenceType,
    /// What was tested or observed.
    pub test: String,
    /// The evidence itself.
    pub detail: String,
    /// Source identifier (links to reputation registry).
    pub source_id: String,
    /// Source reputation at time of evidence (for audit).
    pub source_reputation: f64,
    /// The effective Bayes factor applied.
    pub bayes_factor: f64,
    /// Log-odds before this evidence.
    pub log_odds_before: f64,
    /// Log-odds after this evidence.
    pub log_odds_after: f64,
}

// ── Decay categories ────────────────────────────────────────────────

/// Decay category determines how quickly beliefs fade without fresh evidence.
/// Adapted from TH-WEAVE §8.3 for knowledge (not IoT).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DecayCategory {
    /// Stable facts: "Anthill is written in Rust". Half-life: 30 days.
    #[default]
    Fact,
    /// Decisions: "We chose petgraph over SurrealDB". Half-life: 14 days.
    Decision,
    /// Observations: "Alfred is running v0.4.0". Half-life: 7 days.
    Observation,
    /// Inferences: "This architecture seems scalable". Half-life: 3 days.
    Inference,
    /// Assumptions: "The user probably wants X". Half-life: 1 day.
    Assumed,
}

impl DecayCategory {
    /// Half-life in seconds.
    pub fn half_life_secs(&self) -> f64 {
        match self {
            Self::Fact => 30.0 * 86400.0,        // 30 days
            Self::Decision => 14.0 * 86400.0,     // 14 days
            Self::Observation => 7.0 * 86400.0,   // 7 days
            Self::Inference => 3.0 * 86400.0,     // 3 days
            Self::Assumed => 1.0 * 86400.0,        // 1 day
        }
    }

    /// Half-life in days (for display).
    pub fn half_life_days(&self) -> f64 {
        self.half_life_secs() / 86400.0
    }

    /// Infer decay category from edge basis and node kind.
    pub fn from_basis(basis: &str) -> Self {
        match basis {
            "observed" => Self::Observation,
            "told" => Self::Fact,
            "inferred" => Self::Inference,
            "assumed" => Self::Assumed,
            _ => Self::Fact,
        }
    }
}

// ── Justificatory chains ────────────────────────────────────────────

/// A step in the justificatory chain — provenance for "why do I believe this?"
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JustificationStep {
    /// Step number in the chain.
    pub step: u32,
    /// What process produced this evidence.
    pub process: String,
    /// Confidence at this point in the chain.
    pub confidence: f64,
    /// Source identifier (e.g. "document:README.md", "ai:inference", "user:roy").
    pub source: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_odds_roundtrip() {
        for p in [0.1, 0.25, 0.5, 0.75, 0.9] {
            let lo = to_log_odds(p);
            let p2 = to_probability(lo);
            assert!((p - p2).abs() < 1e-10, "roundtrip failed for p={}", p);
        }
    }

    #[test]
    fn log_odds_at_half() {
        assert!((to_log_odds(0.5)).abs() < 1e-10, "p=0.5 should give log_odds=0");
    }

    #[test]
    fn bayesian_update_with_bf_1() {
        // BF=1 means no change
        let lo = to_log_odds(0.7);
        let updated = bayesian_update(lo, 1.0);
        assert!((lo - updated).abs() < 1e-10);
    }

    #[test]
    fn bayesian_update_strengthens() {
        let lo = to_log_odds(0.5);
        let updated = bayesian_update(lo, 3.0);
        let p = to_probability(updated);
        assert!(p > 0.5, "BF>1 should increase probability");
        // p = 3/(3+1) = 0.75 from prior of 0.5
        assert!((p - 0.75).abs() < 0.01);
    }

    #[test]
    fn bayesian_update_weakens() {
        let lo = to_log_odds(0.5);
        let updated = bayesian_update(lo, 0.3);
        let p = to_probability(updated);
        assert!(p < 0.5, "BF<1 should decrease probability");
    }

    /// TH-WEAVE §10.1 worked example: relay claim weaving.
    #[test]
    fn th_weave_section_10_1_relay_example() {
        // Step 1: Prior p=0.5
        let mut lo = to_log_odds(0.5);
        assert!((lo).abs() < 1e-10);

        // Step 2: X sends breadcrumb (BF=3.0, rep r=0.5)
        // w(0.5) = 0.75, BF_adj = 3.0^0.75 ≈ 2.28
        let bf_adj = reputation_adjusted_bf(3.0, 0.5);
        assert!((bf_adj - 2.28).abs() < 0.1, "BF_adj={}", bf_adj);
        lo = bayesian_update(lo, bf_adj);
        let p = to_probability(lo);
        assert!((p - 0.69).abs() < 0.02, "After breadcrumb: p={}", p);

        // Step 3: B confirms delivery (BF=3.0, r=0.7)
        // w(0.7) = 0.85, BF_adj = 3.0^0.85 ≈ 2.63
        let bf_adj = reputation_adjusted_bf(3.0, 0.7);
        assert!((bf_adj - 2.63).abs() < 0.1, "BF_adj={}", bf_adj);
        lo = bayesian_update(lo, bf_adj);
        let p = to_probability(lo);
        assert!((p - 0.86).abs() < 0.02, "After delivery confirm: p={}", p);

        // Step 4: Refutation test succeeds (BF=2.5)
        lo = bayesian_update(lo, 2.5);
        let p = to_probability(lo);
        assert!((p - 0.94).abs() < 0.02, "After refutation survived: p={}", p);

        // Step 5: After 2 hours decay (τ=1 hour → 2 half-lives)
        lo = decay(lo, 7200.0, 3600.0);
        let p = to_probability(lo);
        assert!((p - 0.66).abs() < 0.05, "After 2hr decay: p={}", p);
    }

    #[test]
    fn reputation_dampening() {
        let bf = 3.0;
        // r=0: √BF
        let dampened = reputation_adjusted_bf(bf, 0.0);
        assert!((dampened - bf.sqrt()).abs() < 0.01);
        // r=1: full BF
        let full = reputation_adjusted_bf(bf, 1.0);
        assert!((full - bf).abs() < 0.01);
        // monotonic: higher reputation → higher adjusted BF
        let mid = reputation_adjusted_bf(bf, 0.5);
        assert!(dampened < mid && mid < full);
    }

    #[test]
    fn decay_toward_uncertainty() {
        let lo = to_log_odds(0.9);
        // After many half-lives, should approach 0 (p=0.5)
        let decayed = decay(lo, 100.0 * 86400.0, 1.0 * 86400.0);
        let p = to_probability(decayed);
        assert!((p - 0.5).abs() < 0.01, "Should decay to ~0.5, got {}", p);
    }

    #[test]
    fn decay_no_effect_when_no_time() {
        let lo = to_log_odds(0.8);
        let decayed = decay(lo, 0.0, 86400.0);
        assert!((lo - decayed).abs() < 1e-10);
    }

    #[test]
    fn evidence_type_bayes_factors() {
        // Sanity checks
        assert!(EvidenceType::Corroboration.base_bayes_factor() > 1.0);
        assert!(EvidenceType::Contradiction.base_bayes_factor() < 1.0);
        assert!(EvidenceType::RefutationSurvived.base_bayes_factor() > 1.0);
        assert!(EvidenceType::RefutationFailed.base_bayes_factor() < 1.0);
        assert!(EvidenceType::Consistency.base_bayes_factor() > 1.0);
        assert!(EvidenceType::Inconsistency.base_bayes_factor() < 1.0);
    }

    #[test]
    fn decay_category_ordering() {
        // Facts should decay slowest, assumptions fastest
        assert!(DecayCategory::Fact.half_life_secs() > DecayCategory::Decision.half_life_secs());
        assert!(DecayCategory::Decision.half_life_secs() > DecayCategory::Observation.half_life_secs());
        assert!(DecayCategory::Observation.half_life_secs() > DecayCategory::Inference.half_life_secs());
        assert!(DecayCategory::Inference.half_life_secs() > DecayCategory::Assumed.half_life_secs());
    }

    #[test]
    fn clamp_prevents_certainty() {
        // Even with very strong evidence, should never reach 0 or 1
        let lo = to_log_odds(0.5);
        let updated = bayesian_update(lo, 1000000.0);
        let p = to_probability(updated);
        assert!(p < 1.0 && p > 0.99, "Expected p > 0.99, got {}", p);

        let updated = bayesian_update(lo, 0.000001);
        let p = to_probability(updated);
        assert!(p > 0.0 && p < 0.01, "Expected p < 0.01, got {}", p);
    }
}
