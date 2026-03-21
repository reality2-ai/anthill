//! CBOR + Git storage backend for knowledge graphs.
//!
//! Stores each graph as a single CBOR file (compact binary, serde-compatible
//! via ciborium). Every save auto-commits to the local git repo with a
//! descriptive message. The git history becomes the knowledge evolution journal.
//!
//! File layout:
//!   memory/knowledge.cbor          — meta-graph
//!   memory/graphs/<topic>.cbor     — topic graphs
//!
//! Also reads legacy .json files for backward compatibility during migration.

use std::path::{Path, PathBuf};
use crate::knowledge::GraphData;
use crate::store::{StorageBackend, StoreResult, StoreError, CommitInfo};

pub struct CborGitBackend {
    memory_dir: PathBuf,
    /// Whether to auto-commit after each save.
    auto_commit: bool,
    /// When true, commits are deferred until end_thought() is called.
    /// This batches multiple changes into one atomic "thought" commit.
    in_thought: std::sync::atomic::AtomicBool,
}

impl CborGitBackend {
    pub fn new(memory_dir: PathBuf) -> Self {
        Self {
            memory_dir,
            auto_commit: true,
            in_thought: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Create without auto-commit (for migration/batch operations).
    #[allow(dead_code)]
    pub fn without_auto_commit(memory_dir: PathBuf) -> Self {
        Self {
            memory_dir,
            auto_commit: false,
            in_thought: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Begin a thought — subsequent saves are batched until end_thought().
    /// Use this to group related changes into one atomic commit.
    pub fn begin_thought(&self) {
        self.in_thought.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// End a thought — commit all batched changes with a descriptive message.
    /// The message should describe what was thought about, not what changed.
    pub fn end_thought(&self, message: &str) -> StoreResult<String> {
        self.in_thought.store(false, std::sync::atomic::Ordering::Relaxed);
        self.git_commit(message)
    }

    /// Check if we're inside a thought transaction.
    pub fn is_in_thought(&self) -> bool {
        self.in_thought.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn cbor_path(&self, name: &str) -> PathBuf {
        if name == "meta" || name.is_empty() {
            self.memory_dir.join("knowledge.cbor")
        } else {
            self.memory_dir.join("graphs").join(format!("{}.cbor", name))
        }
    }

    /// Legacy JSON path — for backward-compatible reading.
    fn json_path(&self, name: &str) -> PathBuf {
        if name == "meta" || name.is_empty() {
            self.memory_dir.join("knowledge.json")
        } else {
            self.memory_dir.join("graphs").join(format!("{}.json", name))
        }
    }

    fn git_commit(&self, message: &str) -> StoreResult<String> {
        if !self.auto_commit { return Ok("auto-commit disabled".into()); }

        let working_dir = self.memory_dir.parent().unwrap_or(&self.memory_dir);

        // Stage all memory changes.
        let _ = std::process::Command::new("git")
            .args(["add", "memory/"])
            .current_dir(working_dir)
            .output();

        // Check if there's anything to commit.
        let has_changes = std::process::Command::new("git")
            .args(["diff", "--cached", "--quiet"])
            .current_dir(working_dir)
            .output()
            .map(|o| !o.status.success())
            .unwrap_or(false);

        if !has_changes {
            return Ok("no changes".into());
        }

        let output = std::process::Command::new("git")
            .args(["commit", "-m", message])
            .current_dir(working_dir)
            .output()
            .map_err(|e| StoreError::Git(e.to_string()))?;

        if output.status.success() {
            let hash = std::process::Command::new("git")
                .args(["rev-parse", "--short", "HEAD"])
                .current_dir(working_dir)
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .unwrap_or_default();
            Ok(hash)
        } else {
            Err(StoreError::Git(String::from_utf8_lossy(&output.stderr).to_string()))
        }
    }

    // ── Branch management (thought branches) ──

    /// Create a new thought branch and switch to it.
    /// Returns the branch name.
    pub fn create_branch(&self, name: &str) -> StoreResult<String> {
        let working_dir = self.memory_dir.parent().unwrap_or(&self.memory_dir);
        let branch = format!("thought/{}", name);

        // Commit any pending changes first.
        let _ = self.git_commit(&format!("checkpoint before branching to {}", branch));

        let output = std::process::Command::new("git")
            .args(["checkout", "-b", &branch])
            .current_dir(working_dir)
            .output()
            .map_err(|e| StoreError::Git(e.to_string()))?;

        if output.status.success() {
            log::info!("Created thought branch: {}", branch);
            Ok(branch)
        } else {
            Err(StoreError::Git(format!("Failed to create branch {}: {}",
                branch, String::from_utf8_lossy(&output.stderr))))
        }
    }

    /// Switch back to main branch.
    pub fn switch_to_main(&self) -> StoreResult<()> {
        let working_dir = self.memory_dir.parent().unwrap_or(&self.memory_dir);

        // Commit any pending changes on the current branch.
        let _ = self.git_commit("checkpoint before switching to main");

        let output = std::process::Command::new("git")
            .args(["checkout", "main"])
            .current_dir(working_dir)
            .output()
            .or_else(|_| {
                // Try "master" if "main" doesn't exist.
                std::process::Command::new("git")
                    .args(["checkout", "master"])
                    .current_dir(working_dir)
                    .output()
            })
            .map_err(|e| StoreError::Git(e.to_string()))?;

        if output.status.success() { Ok(()) }
        else { Err(StoreError::Git(String::from_utf8_lossy(&output.stderr).to_string())) }
    }

    /// Merge a thought branch into main if the exploration was fruitful.
    /// Returns true if merge succeeded.
    pub fn merge_branch(&self, branch: &str) -> StoreResult<bool> {
        let working_dir = self.memory_dir.parent().unwrap_or(&self.memory_dir);

        // Make sure we're on main.
        self.switch_to_main()?;

        let output = std::process::Command::new("git")
            .args(["merge", branch, "--no-ff", "-m",
                &format!("Merge thought branch '{}' — ideas survived evaluation", branch)])
            .current_dir(working_dir)
            .output()
            .map_err(|e| StoreError::Git(e.to_string()))?;

        if output.status.success() {
            log::info!("Merged thought branch: {}", branch);
            // Delete the branch after successful merge.
            let _ = std::process::Command::new("git")
                .args(["branch", "-d", branch])
                .current_dir(working_dir)
                .output();
            Ok(true)
        } else {
            // Merge conflict or failure — abort and stay on main.
            let _ = std::process::Command::new("git")
                .args(["merge", "--abort"])
                .current_dir(working_dir)
                .output();
            log::warn!("Failed to merge thought branch {}: conflict", branch);
            Ok(false)
        }
    }

    /// Abandon a thought branch — the exploration was a dead end.
    pub fn abandon_branch(&self, branch: &str) -> StoreResult<()> {
        let working_dir = self.memory_dir.parent().unwrap_or(&self.memory_dir);

        self.switch_to_main()?;

        // Delete the branch (force — it was never merged).
        let _ = std::process::Command::new("git")
            .args(["branch", "-D", branch])
            .current_dir(working_dir)
            .output();

        log::info!("Abandoned thought branch: {}", branch);
        Ok(())
    }

    /// List all thought branches.
    pub fn list_branches(&self) -> StoreResult<Vec<String>> {
        let working_dir = self.memory_dir.parent().unwrap_or(&self.memory_dir);

        let output = std::process::Command::new("git")
            .args(["branch", "--list", "thought/*"])
            .current_dir(working_dir)
            .output()
            .map_err(|e| StoreError::Git(e.to_string()))?;

        let text = String::from_utf8_lossy(&output.stdout);
        let branches: Vec<String> = text.lines()
            .map(|l| l.trim().trim_start_matches("* ").to_string())
            .filter(|l| !l.is_empty())
            .collect();

        Ok(branches)
    }

    /// Get the current branch name.
    pub fn current_branch(&self) -> StoreResult<String> {
        let working_dir = self.memory_dir.parent().unwrap_or(&self.memory_dir);

        let output = std::process::Command::new("git")
            .args(["branch", "--show-current"])
            .current_dir(working_dir)
            .output()
            .map_err(|e| StoreError::Git(e.to_string()))?;

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
}

impl StorageBackend for CborGitBackend {
    fn load_graph(&self, name: &str) -> StoreResult<Option<GraphData>> {
        let cbor_path = self.cbor_path(name);

        // Try CBOR first.
        if cbor_path.exists() {
            let bytes = std::fs::read(&cbor_path)
                .map_err(|e| StoreError::Storage(format!("read {}: {}", cbor_path.display(), e)))?;
            let data: GraphData = ciborium::de::from_reader(&bytes[..])
                .map_err(|e| StoreError::Storage(format!("CBOR decode {}: {}", cbor_path.display(), e)))?;
            return Ok(Some(data));
        }

        // Fall back to legacy JSON.
        let json_path = self.json_path(name);
        if json_path.exists() {
            let contents = std::fs::read_to_string(&json_path)
                .map_err(|e| StoreError::Storage(format!("read {}: {}", json_path.display(), e)))?;

            // Try strict parse first.
            if let Ok(data) = serde_json::from_str::<GraphData>(&contents) {
                return Ok(Some(data));
            }

            // Lenient parse — load through KnowledgeGraph which handles recovery.
            let kg = crate::knowledge::KnowledgeGraph::load(&json_path);
            if kg.node_count() > 0 {
                return Ok(Some(kg.to_graph_data()));
            }
        }

        Ok(None)
    }

    fn save_graph(&self, name: &str, data: &GraphData) -> StoreResult<()> {
        let path = self.cbor_path(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| StoreError::Storage(e.to_string()))?;
        }

        // Atomic write: CBOR to tmp, fsync, rename.
        let tmp = path.with_extension("cbor.tmp");
        let write_result = (|| -> std::io::Result<()> {
            let mut buf = Vec::new();
            ciborium::ser::into_writer(data, &mut buf)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;

            use std::io::Write;
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(&buf)?;
            f.sync_all()?;
            Ok(())
        })();
        write_result.map_err(|e| StoreError::Storage(e.to_string()))?;
        std::fs::rename(&tmp, &path)
            .map_err(|e| StoreError::Storage(e.to_string()))?;

        // Auto-commit to git (unless inside a thought transaction).
        if !self.is_in_thought() {
            let msg = format!("knowledge: update {}", name);
            let _ = self.git_commit(&msg);
        }

        Ok(())
    }

    fn list_graphs(&self) -> StoreResult<Vec<String>> {
        let mut names = Vec::new();

        // Check for meta-graph (CBOR or JSON).
        if self.cbor_path("meta").exists() || self.json_path("meta").exists() {
            names.push("meta".to_string());
        }

        let graphs_dir = self.memory_dir.join("graphs");
        if graphs_dir.exists() {
            if let Ok(entries) = std::fs::read_dir(&graphs_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    let filename = path.file_name()
                        .map(|f| f.to_string_lossy().to_string())
                        .unwrap_or_default();

                    // Skip archive, corrupted, tmp files.
                    if filename.contains("-archive") || filename.ends_with(".corrupted")
                        || filename.ends_with(".tmp") || filename.ends_with(".bak")
                    {
                        continue;
                    }

                    let is_cbor = filename.ends_with(".cbor");
                    let is_json = filename.ends_with(".json");

                    if !is_cbor && !is_json { continue; }

                    // For JSON files, check they're actually graphs.
                    if is_json {
                        let is_graph = std::fs::read_to_string(&path)
                            .map(|c| c.contains("\"nodes\""))
                            .unwrap_or(false);
                        if !is_graph { continue; }
                    }

                    if let Some(stem) = path.file_stem() {
                        let name = stem.to_string_lossy().to_string();
                        // Don't add duplicates (if both .json and .cbor exist).
                        if !names.contains(&name) {
                            names.push(name);
                        }
                    }
                }
            }
        }

        names.sort();
        Ok(names)
    }

    fn delete_graph(&self, name: &str) -> StoreResult<()> {
        let cbor = self.cbor_path(name);
        let json = self.json_path(name);
        if cbor.exists() {
            std::fs::remove_file(&cbor).map_err(|e| StoreError::Storage(e.to_string()))?;
        }
        if json.exists() {
            std::fs::remove_file(&json).map_err(|e| StoreError::Storage(e.to_string()))?;
        }
        Ok(())
    }

    fn commit(&self, message: &str) -> StoreResult<String> {
        self.git_commit(message)
    }

    fn history(&self, name: &str, limit: usize) -> StoreResult<Vec<CommitInfo>> {
        // Check both CBOR and JSON paths for history.
        let cbor_path = self.cbor_path(name);
        let json_path = self.json_path(name);
        let working_dir = self.memory_dir.parent().unwrap_or(&self.memory_dir);

        // Use the path that exists (prefer CBOR).
        let track_path = if cbor_path.exists() { &cbor_path } else { &json_path };

        let output = std::process::Command::new("git")
            .args([
                "log",
                &format!("--max-count={}", limit),
                "--pretty=format:%h|%s|%ci",
                "--",
            ])
            .arg(track_path)
            .current_dir(working_dir)
            .output()
            .map_err(|e| StoreError::Git(e.to_string()))?;

        let text = String::from_utf8_lossy(&output.stdout);
        let commits = text.lines()
            .filter_map(|line| {
                let parts: Vec<&str> = line.splitn(3, '|').collect();
                if parts.len() == 3 {
                    Some(CommitInfo {
                        hash: parts[0].to_string(),
                        message: parts[1].to_string(),
                        timestamp: parts[2].to_string(),
                    })
                } else {
                    None
                }
            })
            .collect();

        Ok(commits)
    }
}

#[allow(dead_code)]
/// Migrate a JSON graph to CBOR format.
/// Reads the JSON file, converts, writes CBOR, optionally removes JSON.
pub fn migrate_json_to_cbor(json_path: &Path, cbor_path: &Path, remove_json: bool) -> StoreResult<()> {
    // Load via KnowledgeGraph to get lenient parsing + recovery.
    let kg = crate::knowledge::KnowledgeGraph::load(json_path);
    if kg.node_count() == 0 {
        return Err(StoreError::Storage(format!("empty graph at {}", json_path.display())));
    }

    let data = kg.to_graph_data();

    // Write CBOR.
    let mut buf = Vec::new();
    ciborium::ser::into_writer(&data, &mut buf)
        .map_err(|e| StoreError::Storage(format!("CBOR encode: {}", e)))?;

    if let Some(parent) = cbor_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| StoreError::Storage(e.to_string()))?;
    }

    use std::io::Write;
    let tmp = cbor_path.with_extension("cbor.tmp");
    let mut f = std::fs::File::create(&tmp)
        .map_err(|e| StoreError::Storage(e.to_string()))?;
    f.write_all(&buf).map_err(|e| StoreError::Storage(e.to_string()))?;
    f.sync_all().map_err(|e| StoreError::Storage(e.to_string()))?;
    std::fs::rename(&tmp, cbor_path)
        .map_err(|e| StoreError::Storage(e.to_string()))?;

    if remove_json {
        let _ = std::fs::remove_file(json_path);
    }

    Ok(())
}
