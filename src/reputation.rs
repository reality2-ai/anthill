//! Source reputation tracking (TH-REP compliant).
//!
//! Tracks reliability of information sources using a Bayesian model:
//! - Documents: start at 0.5 (neutral)
//! - AI inference: starts at 0.5
//! - Users: start at 0.7 (benefit of the doubt)
//! - Thematic analysis: inherits from source document
//!
//! Reputation decays toward 0.5 with a 7-day half-life (lazy, on access).
//! Persisted to memory/reputation.json.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// A tracked information source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceReputation {
    /// Reputation score [0.0, 1.0].
    pub score: f64,
    /// Source category.
    pub category: SourceCategory,
    /// When this source was first seen (Unix timestamp seconds).
    pub first_seen: u64,
    /// When reputation was last updated (Unix timestamp seconds).
    pub last_updated: u64,
    /// Number of corroborations (claims confirmed by other sources).
    pub corroborations: u32,
    /// Number of contradictions (claims disproved).
    pub contradictions: u32,
    /// Human-readable description.
    #[serde(default)]
    pub description: String,
}

/// Source categories with their initial reputation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SourceCategory {
    /// A document or file being analysed.
    Document,
    /// AI inference (Claude, Gemini, etc.).
    AiInference,
    /// A human user.
    User,
    /// Thematic analysis output (inherits from source document).
    ThematicAnalysis,
    /// MCP tool call.
    Mcp,
    /// Unknown source.
    Unknown,
}

impl SourceCategory {
    /// Initial reputation for a new source of this category.
    pub fn initial_reputation(&self) -> f64 {
        match self {
            Self::Document => 0.5,
            Self::AiInference => 0.5,
            Self::User => 0.7,
            Self::ThematicAnalysis => 0.5,
            Self::Mcp => 0.6,
            Self::Unknown => 0.3,
        }
    }
}

/// Reputation decay half-life in seconds (7 days).
const REPUTATION_HALF_LIFE_SECS: f64 = 7.0 * 86400.0;

/// The reputation registry — maps source IDs to their reputation.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ReputationRegistry {
    pub sources: HashMap<String, SourceReputation>,
}

impl ReputationRegistry {
    /// Load from JSON file, or create empty.
    pub fn load(path: &Path) -> Self {
        if path.exists() {
            if let Ok(contents) = std::fs::read_to_string(path) {
                if let Ok(registry) = serde_json::from_str(&contents) {
                    return registry;
                }
            }
        }
        Self::default()
    }

    /// Save to JSON file (atomic write).
    pub fn save(&self, path: &Path) {
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let tmp = path.with_extension("json.tmp");
            if std::fs::write(&tmp, &json).is_ok() {
                let _ = std::fs::rename(&tmp, path);
            }
        }
    }

    /// Get or create a source's reputation, with lazy decay applied.
    pub fn get_reputation(&mut self, source_id: &str, category: SourceCategory) -> f64 {
        let now = current_timestamp();
        let entry = self.sources.entry(source_id.to_string()).or_insert_with(|| {
            SourceReputation {
                score: category.initial_reputation(),
                category,
                first_seen: now,
                last_updated: now,
                corroborations: 0,
                contradictions: 0,
                description: String::new(),
            }
        });

        // Apply lazy decay toward 0.5
        let elapsed = (now - entry.last_updated) as f64;
        if elapsed > 0.0 {
            entry.score = decay_toward_neutral(entry.score, elapsed);
            entry.last_updated = now;
        }

        entry.score
    }

    /// Record that a source's claim was corroborated.
    pub fn record_corroboration(&mut self, source_id: &str) {
        if let Some(entry) = self.sources.get_mut(source_id) {
            entry.corroborations += 1;
            // Small boost: move 10% of the distance toward 1.0
            entry.score = entry.score + 0.1 * (1.0 - entry.score);
            entry.score = entry.score.clamp(0.01, 0.99);
            entry.last_updated = current_timestamp();
        }
    }

    /// Record that a source's claim was contradicted.
    pub fn record_contradiction(&mut self, source_id: &str) {
        if let Some(entry) = self.sources.get_mut(source_id) {
            entry.contradictions += 1;
            // Penalty: move 15% of the distance toward 0.0
            entry.score = entry.score * 0.85;
            entry.score = entry.score.clamp(0.01, 0.99);
            entry.last_updated = current_timestamp();
        }
    }

    /// Get reputation without creating the entry (read-only peek).
    pub fn peek_reputation(&self, source_id: &str) -> Option<f64> {
        self.sources.get(source_id).map(|e| {
            let elapsed = (current_timestamp() - e.last_updated) as f64;
            if elapsed > 0.0 {
                decay_toward_neutral(e.score, elapsed)
            } else {
                e.score
            }
        })
    }

    /// Set description for a source.
    #[allow(dead_code)]
    pub fn set_description(&mut self, source_id: &str, description: &str) {
        if let Some(entry) = self.sources.get_mut(source_id) {
            entry.description = description.to_string();
        }
    }

    /// Render a summary of all sources for display.
    pub fn render_summary(&self) -> String {
        let now = current_timestamp();
        let mut lines: Vec<String> = self.sources.iter()
            .map(|(id, entry)| {
                let elapsed = (now - entry.last_updated) as f64;
                let current_score = if elapsed > 0.0 {
                    decay_toward_neutral(entry.score, elapsed)
                } else {
                    entry.score
                };
                format!(
                    "{}: {:.0}% ({} corr, {} contra) [{}]",
                    id,
                    current_score * 100.0,
                    entry.corroborations,
                    entry.contradictions,
                    if entry.description.is_empty() { format!("{:?}", entry.category) } else { entry.description.clone() }
                )
            })
            .collect();
        lines.sort();
        if lines.is_empty() {
            "No sources tracked".into()
        } else {
            lines.join("\n")
        }
    }
}

/// Decay reputation toward 0.5 (neutral).
/// Uses the same fading-foundations approach as TH-WEAVE.
fn decay_toward_neutral(score: f64, elapsed_secs: f64) -> f64 {
    // Convert score to offset from 0.5, decay, convert back
    let offset = score - 0.5;
    let decayed_offset = offset * 2.0_f64.powf(-elapsed_secs / REPUTATION_HALF_LIFE_SECS);
    (0.5 + decayed_offset).clamp(0.01, 0.99)
}

/// Current Unix timestamp in seconds.
fn current_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_reputations() {
        assert!((SourceCategory::Document.initial_reputation() - 0.5).abs() < 1e-10);
        assert!((SourceCategory::User.initial_reputation() - 0.7).abs() < 1e-10);
        assert!((SourceCategory::AiInference.initial_reputation() - 0.5).abs() < 1e-10);
    }

    #[test]
    fn decay_toward_neutral_works() {
        // Score of 0.9 should decay toward 0.5
        let decayed = decay_toward_neutral(0.9, 7.0 * 86400.0); // 1 half-life
        // offset was 0.4, after 1 half-life should be 0.2 → score 0.7
        assert!((decayed - 0.7).abs() < 0.01, "got {}", decayed);

        // Score of 0.1 should also decay toward 0.5
        let decayed = decay_toward_neutral(0.1, 7.0 * 86400.0);
        assert!((decayed - 0.3).abs() < 0.01, "got {}", decayed);
    }

    #[test]
    fn neutral_stays_neutral() {
        let decayed = decay_toward_neutral(0.5, 30.0 * 86400.0);
        assert!((decayed - 0.5).abs() < 1e-10);
    }

    #[test]
    fn corroboration_boosts() {
        let mut registry = ReputationRegistry::default();
        registry.sources.insert("test".into(), SourceReputation {
            score: 0.5,
            category: SourceCategory::Document,
            first_seen: current_timestamp(),
            last_updated: current_timestamp(),
            corroborations: 0,
            contradictions: 0,
            description: String::new(),
        });
        registry.record_corroboration("test");
        assert!(registry.sources["test"].score > 0.5);
        assert_eq!(registry.sources["test"].corroborations, 1);
    }

    #[test]
    fn contradiction_penalises() {
        let mut registry = ReputationRegistry::default();
        registry.sources.insert("test".into(), SourceReputation {
            score: 0.8,
            category: SourceCategory::Document,
            first_seen: current_timestamp(),
            last_updated: current_timestamp(),
            corroborations: 0,
            contradictions: 0,
            description: String::new(),
        });
        registry.record_contradiction("test");
        assert!(registry.sources["test"].score < 0.8);
        assert_eq!(registry.sources["test"].contradictions, 1);
    }
}
