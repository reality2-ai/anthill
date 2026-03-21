//! GraphEngine — in-memory graph with queries, consolidation, and rendering.
//!
//! Extracted from knowledge.rs. This module owns the petgraph StableGraph
//! and all operations on it. It does NOT handle file I/O — that's the
//! StorageBackend's job.
//!
//! TODO: Extract query/consolidation/rendering logic from KnowledgeGraph.
//! For now, this re-exports KnowledgeGraph to keep things compiling
//! during the incremental refactor.

// Phase 1 stub: re-export KnowledgeGraph as the engine.
// This will be replaced with a proper GraphEngine struct that owns
// the graph without file I/O concerns.
pub use crate::knowledge::KnowledgeGraph;
