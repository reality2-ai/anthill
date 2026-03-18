//! Periodic git backup for bot working directories.
//!
//! Initialises a git repo in the working dir (if not already one),
//! and periodically commits all changes with a timestamp message.
//! Optionally pushes to a remote.

use std::path::Path;
use std::process::Command;
use std::time::Duration;

/// Ensure the working directory is a git repo.
/// Creates one if it doesn't exist.
pub fn ensure_git_repo(working_dir: &Path) -> anyhow::Result<()> {
    // Ensure .gitignore excludes repos (they have their own git history).
    let gitignore_path = working_dir.join(".gitignore");
    let needs_update = if gitignore_path.exists() {
        let contents = std::fs::read_to_string(&gitignore_path).unwrap_or_default();
        !contents.contains("repos/")
    } else {
        true
    };
    if needs_update {
        let mut contents = if gitignore_path.exists() {
            std::fs::read_to_string(&gitignore_path).unwrap_or_default()
        } else {
            String::new()
        };
        if !contents.is_empty() && !contents.ends_with('\n') {
            contents.push('\n');
        }
        contents.push_str("# Cloned repos have their own git history\nrepos/\n");
        let _ = std::fs::write(&gitignore_path, contents);
    }

    let git_dir = working_dir.join(".git");
    if git_dir.exists() {
        log::info!("Working dir already a git repo: {}", working_dir.display());
        return Ok(());
    }

    log::info!("Initialising git repo in {}", working_dir.display());
    let output = Command::new("git")
        .args(["init"])
        .current_dir(working_dir)
        .output()?;

    if !output.status.success() {
        anyhow::bail!(
            "git init failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    // Create initial commit.
    let _ = Command::new("git")
        .args(["add", "-A"])
        .current_dir(working_dir)
        .output();
    let _ = Command::new("git")
        .args(["commit", "-m", "Initial commit (anthill backup)"])
        .current_dir(working_dir)
        .output();

    Ok(())
}

/// Run a single backup: stage all changes, commit, optionally push.
fn run_backup(working_dir: &Path, remote: &str) -> anyhow::Result<bool> {
    // Stage all changes.
    let output = Command::new("git")
        .args(["add", "-A"])
        .current_dir(working_dir)
        .output()?;
    if !output.status.success() {
        anyhow::bail!("git add failed");
    }

    // Check if there's anything to commit.
    let status = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(working_dir)
        .output()?;
    let status_text = String::from_utf8_lossy(&status.stdout);
    if status_text.trim().is_empty() {
        return Ok(false); // Nothing to commit.
    }

    // Commit with timestamp.
    let timestamp = chrono_timestamp();
    let msg = format!("anthill backup — {}", timestamp);
    let output = Command::new("git")
        .args(["commit", "-m", &msg])
        .current_dir(working_dir)
        .output()?;
    if !output.status.success() {
        anyhow::bail!(
            "git commit failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    log::info!("Backup committed: {}", msg);

    // Push if remote is configured.
    if !remote.is_empty() {
        let output = Command::new("git")
            .args(["push", remote])
            .current_dir(working_dir)
            .output()?;
        if output.status.success() {
            log::info!("Backup pushed to remote '{}'", remote);
        } else {
            log::warn!(
                "Backup push to '{}' failed: {}",
                remote,
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    Ok(true)
}

/// Spawn the periodic backup task.
pub async fn backup_loop(
    working_dir: String,
    interval_hours: u32,
    remote: String,
) {
    let interval = Duration::from_secs(interval_hours as u64 * 3600);
    let dir = std::path::PathBuf::from(&working_dir);

    log::info!(
        "Backup task started — every {}h, dir={}",
        interval_hours, working_dir
    );

    loop {
        tokio::time::sleep(interval).await;

        match run_backup(&dir, &remote) {
            Ok(true) => {}
            Ok(false) => log::debug!("Backup: nothing to commit"),
            Err(e) => log::error!("Backup failed: {}", e),
        }
    }
}

/// Simple timestamp without pulling in chrono.
fn chrono_timestamp() -> String {
    let output = Command::new("date")
        .args(["+%Y-%m-%d %H:%M:%S"])
        .output();
    match output {
        Ok(o) => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        Err(_) => "unknown".to_string(),
    }
}
