//! GraphEngine — in-memory graph with queries, consolidation, and rendering.
//!
//! Extracted from knowledge.rs. This module owns the petgraph StableGraph
//! and all operations on it. It does NOT handle file I/O — that's the
//! StorageBackend's job.
//!
//! Currently delegates to KnowledgeGraph. The plan is to lift query,
//! consolidation, and rendering logic here from knowledge.rs.
