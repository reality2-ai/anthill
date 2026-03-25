//! Backend registry — indexes backends by ID and by category.

use std::collections::HashMap;
use std::sync::Arc;

use super::types::*;

/// Returns true if backend ID is a local inference backend (last-resort).
fn is_local_backend(id: &str) -> bool {
    id.starts_with("ollama") || id.starts_with("lmstudio")
}

/// Sort backends according to category semantics.
///
/// - `Local`/`CostEffective`: cost ascending, then quality descending. No local-to-end move.
/// - `Fast`: speed descending, then quality descending. Local backends to end.
/// - All others (including `None`): quality descending, then cost ascending. Local to end.
fn sort_backends(backends: &mut Vec<Arc<dyn AiBackend>>, category: Option<&EngineCategory>) {
    let local_to_end = !matches!(category, Some(EngineCategory::Local) | Some(EngineCategory::CostEffective));

    backends.sort_by(|a, b| {
        // Push local backends to end unless category is Local/CostEffective.
        if local_to_end {
            let a_local = is_local_backend(a.id());
            let b_local = is_local_backend(b.id());
            if a_local != b_local {
                return a_local.cmp(&b_local); // false < true, so non-local first
            }
        }

        match category {
            Some(EngineCategory::CostEffective) | Some(EngineCategory::Local) => {
                // Cheapest first, then best quality.
                a.tags().cost_tier.cmp(&b.tags().cost_tier)
                    .then(b.tags().quality_tier.cmp(&a.tags().quality_tier))
            }
            Some(EngineCategory::Fast) => {
                // Fastest first, then best quality.
                b.tags().speed_tier.cmp(&a.tags().speed_tier)
                    .then(b.tags().quality_tier.cmp(&a.tags().quality_tier))
            }
            _ => {
                // Best quality first, then cheapest.
                b.tags().quality_tier.cmp(&a.tags().quality_tier)
                    .then(a.tags().cost_tier.cmp(&b.tags().cost_tier))
            }
        }
    });
}

/// Central registry of all available AI backends.
///
/// Thread-safe (all interior data is behind `Arc`).  Constructed once at
/// startup and shared with every ANT's worker.
#[derive(Debug, Clone)]
pub struct BackendRegistry {
    /// ID → backend.
    backends: HashMap<String, Arc<dyn AiBackend>>,
    /// Category → ordered list of backend IDs (first = preferred).
    categories: HashMap<String, Vec<String>>,
}

impl BackendRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            backends: HashMap::new(),
            categories: HashMap::new(),
        }
    }

    /// Register a backend.  Automatically indexes it under its declared
    /// categories.
    pub fn register(&mut self, backend: Arc<dyn AiBackend>) {
        let id = backend.id().to_string();
        for cat in &backend.tags().categories {
            self.categories
                .entry(cat.to_string())
                .or_default()
                .push(id.clone());
        }
        self.backends.insert(id, backend);
    }

    /// Look up a backend by exact ID.
    pub fn get(&self, id: &str) -> Option<Arc<dyn AiBackend>> {
        self.backends.get(id).cloned()
    }

    /// All registered backend IDs.
    pub fn ids(&self) -> Vec<String> {
        self.backends.keys().cloned().collect()
    }

    /// All registered backends, sorted: non-local first by quality desc then
    /// cost asc, local backends (ollama, lmstudio) last.
    pub fn all(&self) -> Vec<Arc<dyn AiBackend>> {
        let mut backends: Vec<_> = self.backends.values().cloned().collect();
        sort_backends(&mut backends, None);
        backends
    }

    /// Find backends belonging to a category, in preference order.
    ///
    /// Sorting adapts to the category intent:
    /// - CostEffective/Local: sort by cost ascending, then quality descending
    /// - Fast: sort by speed descending, then quality descending
    /// - All others: sort by quality descending, then cost ascending
    ///
    /// Local backends (ollama, lmstudio) are always moved to the end unless
    /// the category is `Local` or `CostEffective`.
    pub fn find_by_category(&self, category: &EngineCategory) -> Vec<Arc<dyn AiBackend>> {
        let key = category.to_string();
        self.categories
            .get(&key)
            .map(|ids| {
                let mut backends: Vec<_> = ids.iter()
                    .filter_map(|id| self.backends.get(id).cloned())
                    .collect();
                sort_backends(&mut backends, Some(category));
                backends
            })
            .unwrap_or_default()
    }

    /// Find backends belonging to a category (by string name).
    pub fn find_by_category_str(&self, category: &str) -> Vec<Arc<dyn AiBackend>> {
        if let Some(cat) = EngineCategory::parse(category) {
            self.find_by_category(&cat)
        } else {
            Vec::new()
        }
    }

    /// Resolve a backend selection.
    ///
    /// `selector` can be:
    /// - A category name ("intellectual", "fast", "local", …)
    /// - A backend ID ("claude-cli", "openai-gpt4o", …)
    /// - A comma-separated fallback list ("claude-cli,openai-gpt4o")
    ///
    /// Returns an ordered list of backends to try.
    pub fn resolve(&self, selector: &str) -> Vec<Arc<dyn AiBackend>> {
        // Try as category first.
        if let Some(cat) = EngineCategory::parse(selector) {
            let results = self.find_by_category(&cat);
            if !results.is_empty() {
                return results;
            }
        }
        // Try as comma-separated ID list.
        let ids: Vec<&str> = selector.split(',').map(|s| s.trim()).collect();
        let mut out = Vec::new();
        for id in ids {
            if let Some(b) = self.backends.get(id) {
                out.push(b.clone());
            }
        }
        out
    }

    /// Return only currently-available backends from the registry.
    pub async fn available(&self) -> Vec<Arc<dyn AiBackend>> {
        let mut available = Vec::new();
        for b in self.backends.values() {
            if b.is_available().await {
                available.push(b.clone());
            }
        }
        available
    }

    /// Summary for /backends API and doctor check.
    pub async fn status_report(&self) -> Vec<BackendStatus> {
        let mut report = Vec::new();
        for (id, b) in &self.backends {
            let avail = b.is_available().await;
            report.push(BackendStatus {
                id: id.clone(),
                name: b.name().to_string(),
                available: avail,
                tags: b.tags().clone(),
            });
        }
        report.sort_by(|a, b| a.id.cmp(&b.id));
        report
    }

    /// Number of registered backends.
    pub fn len(&self) -> usize {
        self.backends.len()
    }

    /// Is the registry empty?
    pub fn is_empty(&self) -> bool {
        self.backends.is_empty()
    }

    /// Override category ordering from config.
    ///
    /// `overrides` maps category name → ordered list of backend IDs.
    /// This replaces the auto-detected order for those categories.
    pub fn apply_category_overrides(&mut self, overrides: &HashMap<String, Vec<String>>) {
        for (cat, ids) in overrides {
            // Only keep IDs that actually exist in the registry.
            let valid: Vec<String> = ids.iter()
                .filter(|id| self.backends.contains_key(id.as_str()))
                .cloned()
                .collect();
            if !valid.is_empty() {
                self.categories.insert(cat.clone(), valid);
            }
        }
    }
}

impl Default for BackendRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Status of a single backend (for reporting).
#[derive(Debug, Clone, serde::Serialize)]
pub struct BackendStatus {
    pub id: String,
    pub name: String,
    pub available: bool,
    pub tags: EngineTags,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    /// Dummy backend for testing.
    #[derive(Debug)]
    struct DummyBackend {
        id: String,
        tags: EngineTags,
    }

    #[async_trait::async_trait]
    impl AiBackend for DummyBackend {
        fn id(&self) -> &str { &self.id }
        fn name(&self) -> &str { &self.id }
        fn tags(&self) -> &EngineTags { &self.tags }
        async fn is_available(&self) -> bool { true }
        async fn execute(&self, _req: &AiRequest, _tx: ProgressTx) -> Result<AiResponse, AiError> {
            Ok(AiResponse {
                text: "test".into(),
                backend_id: self.id.clone(),
                tokens: None,
                cost_microdollars: None,
            })
        }
    }

    fn make_dummy(id: &str, cats: Vec<EngineCategory>) -> Arc<dyn AiBackend> {
        Arc::new(DummyBackend {
            id: id.into(),
            tags: EngineTags {
                categories: cats,
                capabilities: vec![],
                cost_tier: 3,
                speed_tier: 3,
                quality_tier: 3,
            },
        })
    }

    #[derive(Debug)]
    struct QualityDummy {
        id: String,
        tags: EngineTags,
    }

    #[async_trait::async_trait]
    impl AiBackend for QualityDummy {
        fn id(&self) -> &str { &self.id }
        fn name(&self) -> &str { &self.id }
        fn tags(&self) -> &EngineTags { &self.tags }
        async fn is_available(&self) -> bool { true }
        async fn execute(&self, _req: &AiRequest, _tx: ProgressTx) -> Result<AiResponse, AiError> {
            Ok(AiResponse {
                text: "test".into(),
                backend_id: self.id.clone(),
                tokens: None,
                cost_microdollars: None,
            })
        }
    }

    #[test]
    fn register_and_lookup() {
        let mut reg = BackendRegistry::new();
        reg.register(make_dummy("a", vec![EngineCategory::Fast]));
        reg.register(make_dummy("b", vec![EngineCategory::Fast, EngineCategory::Balanced]));

        assert_eq!(reg.len(), 2);
        assert!(reg.get("a").is_some());
        assert!(reg.get("c").is_none());
    }

    #[test]
    fn find_by_category() {
        let mut reg = BackendRegistry::new();
        reg.register(make_dummy("a", vec![EngineCategory::Fast]));
        reg.register(make_dummy("b", vec![EngineCategory::Fast, EngineCategory::Balanced]));
        reg.register(make_dummy("c", vec![EngineCategory::Intellectual]));

        let fast = reg.find_by_category(&EngineCategory::Fast);
        assert_eq!(fast.len(), 2);
        let intellectual = reg.find_by_category(&EngineCategory::Intellectual);
        assert_eq!(intellectual.len(), 1);
        assert_eq!(intellectual[0].id(), "c");
    }

    #[test]
    fn find_by_category_sorted_by_quality() {
        let mut reg = BackendRegistry::new();
        let high = Arc::new(QualityDummy {
            id: "high".into(),
            tags: EngineTags {
                categories: vec![EngineCategory::Fast],
                capabilities: vec![],
                cost_tier: 4,
                speed_tier: 3,
                quality_tier: 5,
            },
        });
        let low = Arc::new(QualityDummy {
            id: "low".into(),
            tags: EngineTags {
                categories: vec![EngineCategory::Fast],
                capabilities: vec![],
                cost_tier: 1,
                speed_tier: 3,
                quality_tier: 2,
            },
        });
        let medium = Arc::new(QualityDummy {
            id: "medium".into(),
            tags: EngineTags {
                categories: vec![EngineCategory::Fast],
                capabilities: vec![],
                cost_tier: 2,
                speed_tier: 3,
                quality_tier: 3,
            },
        });

        reg.register(low.clone());
        reg.register(high.clone());
        reg.register(medium.clone());

        let fast = reg.find_by_category(&EngineCategory::Fast);
        assert_eq!(fast.len(), 3);
        assert_eq!(fast[0].id(), "high");
        assert_eq!(fast[1].id(), "medium");
        assert_eq!(fast[2].id(), "low");
    }

    #[test]
    fn resolve_by_category_string() {
        let mut reg = BackendRegistry::new();
        reg.register(make_dummy("a", vec![EngineCategory::Fast]));
        reg.register(make_dummy("b", vec![EngineCategory::Intellectual]));

        let results = reg.resolve("fast");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id(), "a");

        let results = reg.resolve("intellectual");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id(), "b");
    }

    #[test]
    fn resolve_by_id_list() {
        let mut reg = BackendRegistry::new();
        reg.register(make_dummy("x", vec![EngineCategory::Fast]));
        reg.register(make_dummy("y", vec![EngineCategory::Fast]));

        let results = reg.resolve("y,x");
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].id(), "y");
        assert_eq!(results[1].id(), "x");
    }

    #[test]
    fn resolve_by_exact_id() {
        let mut reg = BackendRegistry::new();
        reg.register(make_dummy("claude-cli", vec![EngineCategory::Intellectual]));

        let results = reg.resolve("claude-cli");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id(), "claude-cli");
    }

    #[test]
    fn category_overrides() {
        let mut reg = BackendRegistry::new();
        reg.register(make_dummy("a", vec![EngineCategory::Fast]));
        reg.register(make_dummy("b", vec![EngineCategory::Fast]));

        // Default order: a, b
        let fast = reg.find_by_category(&EngineCategory::Fast);
        assert_eq!(fast[0].id(), "a");

        // Override: b first
        let mut overrides = HashMap::new();
        overrides.insert("fast".into(), vec!["b".into(), "a".into()]);
        reg.apply_category_overrides(&overrides);

        let fast = reg.find_by_category(&EngineCategory::Fast);
        assert_eq!(fast[0].id(), "b");
    }

    #[test]
    fn parse_categories() {
        assert_eq!(EngineCategory::parse("fast"), Some(EngineCategory::Fast));
        assert_eq!(EngineCategory::parse("INTELLECTUAL"), Some(EngineCategory::Intellectual));
        assert_eq!(EngineCategory::parse("cost-effective"), Some(EngineCategory::CostEffective));
        assert_eq!(EngineCategory::parse("cheap"), Some(EngineCategory::CostEffective));
        assert_eq!(EngineCategory::parse("local"), Some(EngineCategory::Local));
        assert_eq!(EngineCategory::parse("specialized:coding"),
            Some(EngineCategory::Specialized("coding".into())));
        assert_eq!(EngineCategory::parse("garbage"), None);
    }

    // ── Audit fix test vectors ───────────────────────────────────────

    /// Fix #16: CostEffective category sorts by cost ascending (cheapest first).
    #[test]
    fn cost_effective_sorts_by_cost_first() {
        let mut reg = BackendRegistry::new();
        let expensive_high_quality = Arc::new(QualityDummy {
            id: "cloud-api".into(),
            tags: EngineTags {
                categories: vec![EngineCategory::CostEffective],
                capabilities: vec![],
                cost_tier: 5, speed_tier: 3, quality_tier: 5,
            },
        });
        let cheap_low_quality = Arc::new(QualityDummy {
            id: "local-cheap".into(),
            tags: EngineTags {
                categories: vec![EngineCategory::CostEffective],
                capabilities: vec![],
                cost_tier: 1, speed_tier: 3, quality_tier: 2,
            },
        });
        reg.register(expensive_high_quality);
        reg.register(cheap_low_quality);

        let results = reg.find_by_category(&EngineCategory::CostEffective);
        assert_eq!(results[0].id(), "local-cheap", "CostEffective should sort cheapest first");
        assert_eq!(results[1].id(), "cloud-api");
    }

    /// Fix #16: Fast category sorts by speed descending (fastest first).
    #[test]
    fn fast_sorts_by_speed_first() {
        let mut reg = BackendRegistry::new();
        let slow = Arc::new(QualityDummy {
            id: "slow-but-smart".into(),
            tags: EngineTags {
                categories: vec![EngineCategory::Fast],
                capabilities: vec![],
                cost_tier: 3, speed_tier: 2, quality_tier: 5,
            },
        });
        let fast = Arc::new(QualityDummy {
            id: "fast-local".into(),
            tags: EngineTags {
                categories: vec![EngineCategory::Fast],
                capabilities: vec![],
                cost_tier: 1, speed_tier: 5, quality_tier: 3,
            },
        });
        reg.register(slow);
        reg.register(fast);

        let results = reg.find_by_category(&EngineCategory::Fast);
        assert_eq!(results[0].id(), "fast-local", "Fast should sort fastest first");
        assert_eq!(results[1].id(), "slow-but-smart");
    }

    /// Fix #19: Ollama/LMStudio always last in non-Local categories.
    #[test]
    fn ollama_lmstudio_always_last_in_quality_categories() {
        let mut reg = BackendRegistry::new();
        let ollama = Arc::new(QualityDummy {
            id: "ollama-llama".into(),
            tags: EngineTags {
                categories: vec![EngineCategory::Intellectual],
                capabilities: vec![],
                cost_tier: 1, speed_tier: 4, quality_tier: 3,
            },
        });
        let lms = Arc::new(QualityDummy {
            id: "lmstudio-qwen".into(),
            tags: EngineTags {
                categories: vec![EngineCategory::Intellectual],
                capabilities: vec![],
                cost_tier: 1, speed_tier: 4, quality_tier: 3,
            },
        });
        let claude = Arc::new(QualityDummy {
            id: "claude-cli".into(),
            tags: EngineTags {
                categories: vec![EngineCategory::Intellectual],
                capabilities: vec![],
                cost_tier: 4, speed_tier: 3, quality_tier: 5,
            },
        });

        reg.register(ollama);
        reg.register(lms);
        reg.register(claude);

        let results = reg.find_by_category(&EngineCategory::Intellectual);
        assert_eq!(results[0].id(), "claude-cli", "Claude should be first (non-local)");
        assert!(results[1].id().starts_with("ollama") || results[1].id().starts_with("lmstudio"),
            "Local backends should be at the end");
    }

    /// Fix #19: Local category does NOT push ollama/lmstudio to end.
    #[test]
    fn local_category_keeps_ollama_at_natural_position() {
        let mut reg = BackendRegistry::new();
        let ollama = Arc::new(QualityDummy {
            id: "ollama-llama".into(),
            tags: EngineTags {
                categories: vec![EngineCategory::Local],
                capabilities: vec![],
                cost_tier: 1, speed_tier: 4, quality_tier: 3,
            },
        });
        let lms_expensive = Arc::new(QualityDummy {
            id: "lmstudio-big".into(),
            tags: EngineTags {
                categories: vec![EngineCategory::Local],
                capabilities: vec![],
                cost_tier: 2, speed_tier: 3, quality_tier: 4,
            },
        });

        reg.register(lms_expensive);
        reg.register(ollama);

        let results = reg.find_by_category(&EngineCategory::Local);
        assert_eq!(results[0].id(), "ollama-llama",
            "Local category should sort by cost (cheapest first), not push ollama to end");
    }

    /// Fix #19: all() returns non-local first, local last, sorted by quality.
    #[test]
    fn all_backends_sorted_correctly() {
        let mut reg = BackendRegistry::new();
        let ollama = Arc::new(QualityDummy {
            id: "ollama-x".into(),
            tags: EngineTags {
                categories: vec![EngineCategory::Local],
                capabilities: vec![],
                cost_tier: 1, speed_tier: 4, quality_tier: 3,
            },
        });
        let gemini = Arc::new(QualityDummy {
            id: "gemini-cli".into(),
            tags: EngineTags {
                categories: vec![EngineCategory::Fast],
                capabilities: vec![],
                cost_tier: 3, speed_tier: 4, quality_tier: 4,
            },
        });
        let claude = Arc::new(QualityDummy {
            id: "claude-cli".into(),
            tags: EngineTags {
                categories: vec![EngineCategory::Intellectual],
                capabilities: vec![],
                cost_tier: 4, speed_tier: 3, quality_tier: 5,
            },
        });

        reg.register(ollama);
        reg.register(gemini);
        reg.register(claude);

        let all = reg.all();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].id(), "claude-cli", "Best quality non-local first");
        assert_eq!(all[1].id(), "gemini-cli", "Next quality non-local");
        assert_eq!(all[2].id(), "ollama-x", "Local backend last");
    }
}
