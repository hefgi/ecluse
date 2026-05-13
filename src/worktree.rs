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
        self.project_root.join(&config.worktree_dir).join(slug)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::TempDir;

    fn setup_git_repo(dir: &std::path::Path) {
        Command::new("git")
            .args(["init"])
            .current_dir(dir)
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "--allow-empty", "-m", "init"])
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "test")
            .env("GIT_AUTHOR_EMAIL", "test@test.com")
            .env("GIT_COMMITTER_NAME", "test")
            .env("GIT_COMMITTER_EMAIL", "test@test.com")
            .output()
            .unwrap();
    }

    fn make_config_for_worktree() -> crate::config::Config {
        crate::config::Config {
            mode: crate::config::Mode::Host,
            max_slots: 8,
            prefix: "ecluse".into(),
            worktree_dir: ".ecluse/worktrees".into(),
            app_label: "ecluse.role".into(),
            app_label_value: "app".into(),
            strict_port: false,
            port_search_range: 10,
            services: vec![],
            hooks: crate::config::HookConfig::default(),
        }
    }

    // ── verify_git_repo ───────────────────────────────────────────────────────

    #[test]
    fn verify_git_repo_succeeds_in_git_dir() {
        let dir = TempDir::new().unwrap();
        setup_git_repo(dir.path());
        assert!(WorktreeManager::verify_git_repo(dir.path()).is_ok());
    }

    #[test]
    fn verify_git_repo_fails_outside_git_dir() {
        let dir = TempDir::new().unwrap();
        let err = WorktreeManager::verify_git_repo(dir.path()).unwrap_err();
        assert!(
            err.to_string().contains("git") || err.to_string().contains("repository"),
            "got: {}",
            err
        );
    }

    // ── worktree_path ─────────────────────────────────────────────────────────

    #[test]
    fn worktree_path_constructs_correctly() {
        let dir = TempDir::new().unwrap();
        let wt = WorktreeManager::new(dir.path().to_owned());
        let config = make_config_for_worktree();
        let path = wt.worktree_path(&config, "feat-x");
        assert!(path.ends_with(".ecluse/worktrees/feat-x"));
    }

    #[test]
    fn worktree_path_uses_custom_worktree_dir() {
        let dir = TempDir::new().unwrap();
        let wt = WorktreeManager::new(dir.path().to_owned());
        let mut config = make_config_for_worktree();
        config.worktree_dir = ".wt".into();
        let path = wt.worktree_path(&config, "my-slug");
        assert!(path.ends_with(".wt/my-slug"));
    }

    // ── repo_name ─────────────────────────────────────────────────────────────

    #[test]
    fn repo_name_returns_directory_name() {
        let dir = TempDir::new().unwrap();
        // TempDir has a name like "tmp12345" — we just check it doesn't crash
        let name = WorktreeManager::repo_name(dir.path());
        assert!(!name.is_empty());
    }

    #[test]
    fn repo_name_fallback_for_root() {
        let name = WorktreeManager::repo_name(std::path::Path::new("/"));
        assert_eq!(name, "ecluse");
    }

    // ── create / remove worktree ──────────────────────────────────────────────

    #[test]
    fn create_and_remove_worktree_new_branch() {
        let dir = TempDir::new().unwrap();
        setup_git_repo(dir.path());
        let wt = WorktreeManager::new(dir.path().to_owned());
        let config = make_config_for_worktree();
        let path = wt.worktree_path(&config, "test-slug");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();

        wt.create(&path, "ecluse/test-slug").unwrap();
        assert!(path.exists(), "worktree should exist after create");

        wt.remove(&path).unwrap();
        assert!(!path.exists(), "worktree should be gone after remove");
    }

    #[test]
    fn create_worktree_existing_branch() {
        let dir = TempDir::new().unwrap();
        setup_git_repo(dir.path());

        // Create branch first
        Command::new("git")
            .args(["branch", "my-existing-branch"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        let wt = WorktreeManager::new(dir.path().to_owned());
        let config = make_config_for_worktree();
        let path = wt.worktree_path(&config, "existing");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();

        wt.create(&path, "my-existing-branch").unwrap();
        assert!(path.exists());
        wt.remove(&path).unwrap();
    }
}
