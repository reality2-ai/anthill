//! Periodic git backup for bot working directories.
//!
//! Initialises a git repo in the working dir (if not already one),
//! and periodically commits all changes with a timestamp message.
//! Optionally pushes to a remote.
//!
//! When a colony key is provided, files in memory/ and files/ are
//! encrypted (AES-256-GCM) before commit and decrypted after.
//! Git history contains only encrypted content — safe even in public repos.
//! The working directory stays plaintext for the ANT and web dashboard.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// Ensure the working directory is a git repo.
pub fn ensure_git_repo(working_dir: &Path) -> anyhow::Result<()> {
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
            redact_credentials(&String::from_utf8_lossy(&output.stderr))
        );
    }

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

/// Encrypt files in memory/ and files/ directories before git commit.
/// Returns the list of files that were encrypted (to decrypt after commit).
fn encrypt_for_backup(working_dir: &Path, credential: &str) -> Vec<(PathBuf, Vec<u8>)> {
    let mut originals = Vec::new();
    let dirs_to_encrypt = ["memory", "files"];

    for dir_name in &dirs_to_encrypt {
        let dir = working_dir.join(dir_name);
        if !dir.exists() {
            continue;
        }
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                if let Ok(plaintext) = std::fs::read(&path) {
                    // Save original content for restoration after commit.
                    originals.push((path.clone(), plaintext.clone()));

                    // Encrypt and overwrite.
                    match crate::trust::encrypt_payload(credential, &plaintext) {
                        Ok(encrypted) => {
                            let _ = std::fs::write(&path, encrypted.as_bytes());
                        }
                        Err(e) => {
                            log::warn!("Failed to encrypt {}: {}", path.display(), e);
                        }
                    }
                }
            }
        }
    }
    originals
}

/// Restore plaintext files after git commit.
fn restore_after_backup(originals: &[(PathBuf, Vec<u8>)]) {
    for (path, content) in originals {
        let _ = std::fs::write(path, content);
    }
}

/// Run a single backup: encrypt → stage → commit → decrypt → optionally push.
fn run_backup(working_dir: &Path, remote: &str, credential: &str) -> anyhow::Result<bool> {
    // Encrypt files for backup (if credential provided).
    let originals = if !credential.is_empty() {
        encrypt_for_backup(working_dir, credential)
    } else {
        Vec::new()
    };

    // Stage all changes.
    let output = Command::new("git")
        .args(["add", "-A"])
        .current_dir(working_dir)
        .output()?;
    if !output.status.success() {
        restore_after_backup(&originals);
        anyhow::bail!("git add failed");
    }

    // Check if there's anything to commit.
    let status = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(working_dir)
        .output()?;
    let status_text = String::from_utf8_lossy(&status.stdout);
    if status_text.trim().is_empty() {
        restore_after_backup(&originals);
        return Ok(false);
    }

    // Commit with timestamp.
    let timestamp = chrono_timestamp();
    let msg = if credential.is_empty() {
        format!("anthill backup — {}", timestamp)
    } else {
        format!("anthill backup (encrypted) — {}", timestamp)
    };
    let output = Command::new("git")
        .args(["commit", "-m", &msg])
        .current_dir(working_dir)
        .output()?;

    // Restore plaintext immediately after commit.
    restore_after_backup(&originals);

    if !output.status.success() {
        anyhow::bail!(
            "git commit failed: {}",
            redact_credentials(&String::from_utf8_lossy(&output.stderr))
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
            let stderr = redact_credentials(&String::from_utf8_lossy(&output.stderr));
            log::warn!("Backup push to '{}' failed: {}", remote, stderr);
        }
    }

    Ok(true)
}

/// Spawn the periodic backup task.
pub async fn backup_loop(
    working_dir: String,
    interval_hours: u32,
    remote: String,
    credential: String,
) {
    let interval = Duration::from_secs(interval_hours as u64 * 3600);
    let dir = std::path::PathBuf::from(&working_dir);

    log::info!(
        "Backup task started — every {}h, dir={}, encrypted={}",
        interval_hours, working_dir, !credential.is_empty()
    );

    loop {
        tokio::time::sleep(interval).await;

        match run_backup(&dir, &remote, &credential) {
            Ok(true) => {}
            Ok(false) => log::debug!("Backup: nothing to commit"),
            Err(e) => log::error!("Backup failed: {}", e),
        }
    }
}

/// Redact credentials from git error output (e.g. https://user:pass@host).
fn redact_credentials(text: &str) -> String {
    // Match patterns like https://user:pass@host or http://token@host
    let mut result = text.to_string();
    while let Some(start) = result.find("://") {
        let after_scheme = start + 3;
        if let Some(at_pos) = result[after_scheme..].find('@') {
            let at_abs = after_scheme + at_pos;
            // Only redact if there's something between :// and @
            if at_abs > after_scheme {
                result = format!("{}://[REDACTED]@{}", &result[..start], &result[at_abs + 1..]);
            } else {
                break;
            }
        } else {
            break;
        }
    }
    result
}

fn chrono_timestamp() -> String {
    let output = Command::new("date")
        .args(["+%Y-%m-%d %H:%M:%S"])
        .output();
    match output {
        Ok(o) => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        Err(_) => "unknown".to_string(),
    }
}
