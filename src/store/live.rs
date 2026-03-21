//! LiveKnowledgeStore — the primary KnowledgeStore implementation.
//!
//! Wraps GraphEngine + StorageBackend behind the KnowledgeStore trait.
//! Handles caching, validation, and git integration.
//!
//! TODO: Implement the full KnowledgeStore trait.
//! For now this is a structural stub to verify the module compiles.

use std::path::PathBuf;
use crate::store::json_backend::JsonFileBackend;

/// The primary implementation of KnowledgeStore.
pub struct LiveKnowledgeStore {
    backend: JsonFileBackend,
    #[allow(dead_code)]
    memory_dir: PathBuf,
}

impl LiveKnowledgeStore {
    /// Create a new store backed by JSON files.
    pub fn new(memory_dir: PathBuf) -> Self {
        let backend = JsonFileBackend::new(memory_dir.clone());
        Self { backend, memory_dir }
    }

    /// Get a reference to the backend (for direct operations during migration).
    #[allow(dead_code)]
    pub fn backend(&self) -> &JsonFileBackend {
        &self.backend
    }
}

// TODO: impl KnowledgeStore for LiveKnowledgeStore { ... }
// This will be filled in as we refactor consumers to use the trait.
// For now, consumers still use KnowledgeGraph directly.
