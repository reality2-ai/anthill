//! JSON file storage backend.
//!
//! Wraps the current file-based load/save logic including lenient parsing,
//! git checkout recovery, and atomic writes with fsync.
//!
//! TODO: Implement StorageBackend trait.
//! For now this is a stub to keep the crate compiling.

use std::path::PathBuf;
use crate::store::{StorageBackend, StoreResult, StoreError, CommitInfo};
use crate::knowledge::GraphData;

pub struct JsonFileBackend {
    /// Root memory directory (contains knowledge.json and graphs/).
    memory_dir: PathBuf,
}

impl JsonFileBackend {
    pub fn new(memory_dir: PathBuf) -> Self {
        Self { memory_dir }
    }

    fn meta_path(&self) -> PathBuf {
        self.memory_dir.join("knowledge.json")
    }

    fn graph_path(&self, name: &str) -> PathBuf {
        if name == "meta" || name.is_empty() {
            self.meta_path()
        } else {
            self.memory_dir.join("graphs").join(format!("{}.json", name))
        }
    }
}

impl StorageBackend for JsonFileBackend {
    fn load_graph(&self, name: &str) -> StoreResult<Option<GraphData>> {
        let path = self.graph_path(name);
        if !path.exists() {
            return Ok(None);
        }
        // Delegate to KnowledgeGraph's existing load logic for now.
        // This will be replaced with direct file reading + lenient parse.
        let kg = crate::knowledge::KnowledgeGraph::load(&path);
        Ok(Some(kg.to_graph_data()))
    }

    fn save_graph(&self, name: &str, data: &GraphData) -> StoreResult<()> {
        let path = self.graph_path(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| StoreError::Storage(e.to_string()))?;
        }
        let json = serde_json::to_string_pretty(data)
            .map_err(|e| StoreError::Storage(e.to_string()))?;
        let tmp = path.with_extension("json.tmp");
        let write_result = (|| -> std::io::Result<()> {
            use std::io::Write;
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(json.as_bytes())?;
            f.sync_all()?;
            Ok(())
        })();
        write_result.map_err(|e| StoreError::Storage(e.to_string()))?;
        std::fs::rename(&tmp, &path).map_err(|e| StoreError::Storage(e.to_string()))?;
        Ok(())
    }

    fn list_graphs(&self) -> StoreResult<Vec<String>> {
        let mut names = vec!["meta".to_string()];
        let graphs_dir = self.memory_dir.join("graphs");
        if graphs_dir.exists() {
            if let Ok(entries) = std::fs::read_dir(&graphs_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    let filename = path.file_name()
                        .map(|f| f.to_string_lossy().to_string())
                        .unwrap_or_default();
                    if filename.ends_with(".json")
                        && !filename.contains("-archive")
                        && !filename.ends_with(".corrupted")
                        && !filename.ends_with(".tmp")
                    {
                        if let Some(stem) = path.file_stem() {
                            names.push(stem.to_string_lossy().to_string());
                        }
                    }
                }
            }
        }
        names.sort();
        Ok(names)
    }

    fn delete_graph(&self, name: &str) -> StoreResult<()> {
        let path = self.graph_path(name);
        if path.exists() {
            std::fs::remove_file(&path).map_err(|e| StoreError::Storage(e.to_string()))?;
        }
        Ok(())
    }

    fn commit(&self, message: &str) -> StoreResult<String> {
        let working_dir = self.memory_dir.parent().unwrap_or(&self.memory_dir);

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
            // Get the commit hash.
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

    fn history(&self, name: &str, limit: usize) -> StoreResult<Vec<CommitInfo>> {
        let path = self.graph_path(name);
        let working_dir = self.memory_dir.parent().unwrap_or(&self.memory_dir);

        let output = std::process::Command::new("git")
            .args([
                "log",
                &format!("--max-count={}", limit),
                "--pretty=format:%h|%s|%ci",
                "--",
            ])
            .arg(&path)
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
