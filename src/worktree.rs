use anyhow::{Context, Result};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct WorktreeManager {
    pub project_root: PathBuf,
}

#[derive(Debug, PartialEq)]
pub enum WorktreeRemovalChoice {
    Stop,
    KeepWorktree,
    DeleteWorktree,
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
            let prune_status = Command::new("git")
                .args(["worktree", "prune"])
                .current_dir(&self.project_root)
                .status()
                .context("failed to run git worktree prune")?;
            if !prune_status.success() {
                return Err(anyhow::anyhow!(
                    "git worktree remove and git worktree prune both failed for {}; \
                     remove it manually with `git worktree remove --force {}`",
                    path.display(),
                    path.display()
                ));
            }
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

    /// Returns the path of the main (primary) worktree, regardless of where
    /// the command is run from.  git always lists the main worktree first in
    /// `git worktree list --porcelain`.
    pub fn main_worktree_root(cwd: &Path) -> Result<PathBuf> {
        let output = Command::new("git")
            .args(["worktree", "list", "--porcelain"])
            .current_dir(cwd)
            .output()
            .context("failed to run git worktree list")?;

        if !output.status.success() {
            return Err(crate::error::EcluseError::NotAGitRepo.into());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        // First non-empty "worktree <path>" line is the main worktree.
        for line in stdout.lines() {
            if let Some(path) = line.strip_prefix("worktree ") {
                return Ok(PathBuf::from(path));
            }
        }

        Err(anyhow::anyhow!(
            "could not determine main worktree path; git worktree list produced no output"
        ))
    }
}

/// Returns true if `cwd` is inside a linked git worktree (not the main worktree).
/// Detects this by checking whether `git rev-parse --git-dir` output contains
/// `/.git/worktrees/`, which git writes only for linked worktrees.
pub fn is_inside_git_worktree(cwd: &Path) -> bool {
    Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .current_dir(cwd)
        .output()
        .map(|o| {
            let out = String::from_utf8_lossy(&o.stdout);
            out.contains("/.git/worktrees/")
        })
        .unwrap_or(false)
}

#[derive(Debug, PartialEq)]
pub enum EnvConflictChoice {
    Skip,
    Overwrite,
}

/// Symlink each file in `files` from `root` into `worktree_path`.
/// Files absent in root are silently skipped. If the destination already exists
/// (file or broken symlink), the user is prompted to skip or overwrite.
pub fn symlink_env_files(
    root: &Path,
    worktree_path: &Path,
    files: &[String],
    log: &crate::log::StepLogger,
) -> anyhow::Result<()> {
    for name in files {
        let src = root.join(name);
        if !src.exists() {
            continue;
        }
        let dst = worktree_path.join(name);
        if dst.exists() || dst.symlink_metadata().is_ok() {
            match prompt_env_file_conflict(&dst, name) {
                EnvConflictChoice::Skip => {
                    log.detail(&format!("skipped {} (file already exists)", name));
                    continue;
                }
                EnvConflictChoice::Overwrite => {
                    std::fs::remove_file(&dst)
                        .with_context(|| format!("failed to remove existing {}", dst.display()))?;
                }
            }
        }
        std::os::unix::fs::symlink(&src, &dst)
            .with_context(|| format!("failed to symlink {} into worktree", name))?;
        log.detail(&format!("symlinked {}", name));
    }
    Ok(())
}

/// Prompt the user when an env file already exists in the worktree.
/// Reads from /dev/tty so the prompt works even when stdin is piped.
pub fn prompt_env_file_conflict(dst: &Path, name: &str) -> EnvConflictChoice {
    eprintln!("\n  {} already exists in worktree: {}", name, dst.display());
    eprintln!("    [1] skip      (keep existing file)");
    eprintln!("    [2] overwrite (replace with symlink to repo root)");
    eprint!("  Choice [1/2]: ");
    let _ = std::io::stderr().flush();
    let input = read_tty_line().unwrap_or_default();
    match input.trim() {
        "2" => EnvConflictChoice::Overwrite,
        _ => EnvConflictChoice::Skip,
    }
}

/// Returns true if the worktree at `path` has any uncommitted changes
/// (staged, unstaged, or untracked files).
pub fn has_uncommitted_changes(path: &Path) -> bool {
    Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(path)
        .output()
        .map(|o| o.status.success() && !o.stdout.is_empty())
        .unwrap_or(false)
}

/// Prompt the user before removing a worktree. Always asks; adds an
/// uncommitted-changes warning when dirty. Reads directly from /dev/tty
/// so it works even when stdout/stderr are piped.
///
/// Returns the user's choice. On any I/O error or EOF, returns Stop (safe default).
pub fn prompt_worktree_removal(path: &Path) -> WorktreeRemovalChoice {
    let dirty = has_uncommitted_changes(path);

    // Write prompt to stderr so it is visible even when stdout is redirected.
    if dirty {
        eprintln!("\n  ⚠  UNCOMMITTED CHANGES in worktree: {}", path.display());
    } else {
        eprintln!("\n  Worktree: {}", path.display());
    }
    eprintln!("    [1] stop  (abort — leave everything as-is)");
    eprintln!("    [2] keep  (continue, but keep the worktree on disk)");
    eprintln!("    [3] delete (continue and delete the worktree)");
    eprint!("  Choice [1/2/3]: ");
    let _ = std::io::stderr().flush();

    // Read from /dev/tty so the prompt works even when stdin is piped.
    let input = read_tty_line().unwrap_or_default();
    match input.trim() {
        "2" => WorktreeRemovalChoice::KeepWorktree,
        "3" => WorktreeRemovalChoice::DeleteWorktree,
        _ => WorktreeRemovalChoice::Stop,
    }
}

fn read_tty_line() -> std::io::Result<String> {
    use std::io::BufRead;
    let tty = std::fs::File::open("/dev/tty")?;
    let mut reader = std::io::BufReader::new(tty);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    Ok(line)
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
            inherit_env: vec![],
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

    // ── is_inside_git_worktree ────────────────────────────────────────────────

    #[test]
    fn is_inside_git_worktree_returns_false_in_main_repo() {
        let dir = TempDir::new().unwrap();
        setup_git_repo(dir.path());
        assert!(!is_inside_git_worktree(dir.path()));
    }

    #[test]
    fn is_inside_git_worktree_returns_true_in_linked_worktree() {
        let dir = TempDir::new().unwrap();
        setup_git_repo(dir.path());
        let wt = WorktreeManager::new(dir.path().to_owned());
        let config = make_config_for_worktree();
        let path = wt.worktree_path(&config, "feat");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        wt.create(&path, "ecluse/feat").unwrap();
        assert!(is_inside_git_worktree(&path));
        wt.remove(&path).unwrap();
    }

    // ── symlink_env_files ─────────────────────────────────────────────────────

    #[test]
    fn symlink_env_files_creates_symlink() {
        let dir = TempDir::new().unwrap();
        setup_git_repo(dir.path());
        let wt = WorktreeManager::new(dir.path().to_owned());
        let config = make_config_for_worktree();
        let wt_path = wt.worktree_path(&config, "feat");
        std::fs::create_dir_all(&wt_path).unwrap();

        // Create .env in root
        let src = dir.path().join(".env");
        std::fs::write(&src, "SECRET=abc\n").unwrap();

        let log = crate::log::StepLogger::new(true);
        symlink_env_files(dir.path(), &wt_path, &[".env".into()], &log).unwrap();

        let dst = wt_path.join(".env");
        assert!(dst.symlink_metadata().is_ok(), "symlink should exist");
        assert!(std::fs::read_link(&dst).is_ok(), "dst should be a symlink");
        assert_eq!(std::fs::read_to_string(&dst).unwrap(), "SECRET=abc\n");
    }

    #[test]
    fn symlink_env_files_skips_missing_src() {
        let dir = TempDir::new().unwrap();
        setup_git_repo(dir.path());
        let wt = WorktreeManager::new(dir.path().to_owned());
        let config = make_config_for_worktree();
        let wt_path = wt.worktree_path(&config, "feat");
        std::fs::create_dir_all(&wt_path).unwrap();

        // .env does NOT exist in root
        let log = crate::log::StepLogger::new(true);
        symlink_env_files(dir.path(), &wt_path, &[".env".into()], &log).unwrap();

        assert!(
            !wt_path.join(".env").exists(),
            "no symlink should be created"
        );
    }

    // ── main_worktree_root ────────────────────────────────────────────────────

    #[test]
    fn main_worktree_root_from_main_worktree() {
        let dir = TempDir::new().unwrap();
        setup_git_repo(dir.path());
        let root = WorktreeManager::main_worktree_root(dir.path()).unwrap();
        // Resolve symlinks so the comparison is stable on macOS (/var → /private/var).
        let expected = std::fs::canonicalize(dir.path()).unwrap();
        let got = std::fs::canonicalize(&root).unwrap();
        assert_eq!(got, expected);
    }

    #[test]
    fn main_worktree_root_from_linked_worktree() {
        let dir = TempDir::new().unwrap();
        setup_git_repo(dir.path());

        let wt_path = dir.path().join(".ecluse/worktrees/feat");
        std::fs::create_dir_all(&wt_path).unwrap();

        let wt = WorktreeManager::new(dir.path().to_owned());
        let config = make_config_for_worktree();
        let path = wt.worktree_path(&config, "feat");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        wt.create(&path, "ecluse/feat").unwrap();

        // When called from the linked worktree, should still return the main root.
        let root = WorktreeManager::main_worktree_root(&path).unwrap();
        let expected = std::fs::canonicalize(dir.path()).unwrap();
        let got = std::fs::canonicalize(&root).unwrap();
        assert_eq!(got, expected);

        wt.remove(&path).unwrap();
    }
}
