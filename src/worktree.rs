use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct WorktreeManager {
    pub project_root: PathBuf,
}

impl WorktreeManager {
    pub fn new(project_root: PathBuf) -> Self {
        Self { project_root }
    }

    pub fn worktree_path(&self, config: &crate::config::Config, slug: &str) -> PathBuf {
        self.project_root
            .join(&config.worktree_dir)
            .join(slug)
    }

    pub fn create(&self, path: &Path, branch: &str) -> Result<()> {
        // Try to create branch from HEAD; if it exists already, reuse it
        let branch_exists = Command::new("git")
            .args(["branch", "--list", branch])
            .current_dir(&self.project_root)
            .output()
            .context("failed to run git branch --list")?
            .stdout
            .iter()
            .any(|&b| b != b'\n');

        let status = if branch_exists {
            Command::new("git")
                .args(["worktree", "add"])
                .arg(path)
                .arg(branch)
                .current_dir(&self.project_root)
                .status()
                .context("failed to run git worktree add")?
        } else {
            Command::new("git")
                .args(["worktree", "add", "-b"])
                .arg(branch)
                .arg(path)
                .current_dir(&self.project_root)
                .status()
                .context("failed to run git worktree add -b")?
        };

        if !status.success() {
            return Err(anyhow::anyhow!(
                "git worktree add failed with exit code {}",
                status.code().unwrap_or(-1)
            ));
        }
        Ok(())
    }

    pub fn remove(&self, path: &Path) -> Result<()> {
        let status = Command::new("git")
            .args(["worktree", "remove", "--force"])
            .arg(path)
            .current_dir(&self.project_root)
            .status()
            .context("failed to run git worktree remove")?;

        if !status.success() {
            // Fallback: prune
            let _ = Command::new("git")
                .args(["worktree", "prune"])
                .current_dir(&self.project_root)
                .status();
        }
        Ok(())
    }

    pub fn verify_git_repo(root: &Path) -> Result<()> {
        let status = Command::new("git")
            .args(["rev-parse", "--git-dir"])
            .current_dir(root)
            .status()
            .context("failed to run git rev-parse")?;
        if !status.success() {
            return Err(crate::error::EcluseError::NotAGitRepo.into());
        }
        Ok(())
    }

    #[allow(dead_code)]
    pub fn repo_name(root: &Path) -> String {
        root.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("ecluse")
            .to_string()
    }
}
