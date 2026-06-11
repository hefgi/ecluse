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
        let status = if self.local_branch_exists(branch)? {
            // Reuse the existing local branch.
            Command::new("git")
                .args(["worktree", "add"])
                .arg(path)
                .arg(branch)
                .current_dir(&self.project_root)
                .status()
                .context("failed to run git worktree add")?
        } else if self.remote_branch_exists(branch) {
            // The branch exists on origin but not locally — create a tracking
            // branch from it. Forking a same-named branch off HEAD here would
            // silently put the session on the wrong base.
            Command::new("git")
                .args(["worktree", "add", "--track", "-b", branch])
                .arg(path)
                .arg(format!("origin/{}", branch))
                .current_dir(&self.project_root)
                .status()
                .context("failed to run git worktree add --track")?
        } else {
            // Brand-new branch from HEAD.
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

    fn local_branch_exists(&self, branch: &str) -> Result<bool> {
        let output = Command::new("git")
            .args(["branch", "--list", branch])
            .current_dir(&self.project_root)
            .output()
            .context("failed to run git branch --list")?;
        Ok(!String::from_utf8_lossy(&output.stdout).trim().is_empty())
    }

    /// True when origin/<branch> exists as a local remote-tracking ref
    /// (no network access — uses whatever the last fetch brought in).
    fn remote_branch_exists(&self, branch: &str) -> bool {
        Command::new("git")
            .args([
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("refs/remotes/origin/{}", branch),
            ])
            .current_dir(&self.project_root)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    pub fn remove(&self, path: &Path) -> Result<()> {
        let output = Command::new("git")
            .args(["worktree", "remove", "--force"])
            .arg(path)
            .current_dir(&self.project_root)
            .output()
            .context("failed to run git worktree remove")?;

        if output.status.success() {
            return Ok(());
        }

        // `remove` fails when the directory was already deleted by hand; prune
        // cleans up the leftover administrative data. Prune never deletes a
        // directory that still exists, so verify before reporting success.
        let _ = Command::new("git")
            .args(["worktree", "prune"])
            .current_dir(&self.project_root)
            .status();

        if path.exists() {
            return Err(anyhow::anyhow!(
                "git worktree remove failed for {}: {}; remove it manually with `git worktree remove --force {}`",
                path.display(),
                String::from_utf8_lossy(&output.stderr).trim(),
                path.display()
            ));
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

/// Returns the root directory of the git worktree that contains `cwd`.
/// Uses `git rev-parse --show-toplevel` which outputs the worktree root, not the repo root.
pub fn git_worktree_root(cwd: &Path) -> anyhow::Result<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(cwd)
        .output()
        .context("failed to run git rev-parse --show-toplevel")?;
    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "git rev-parse --show-toplevel failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(PathBuf::from(
        String::from_utf8_lossy(&output.stdout).trim(),
    ))
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

/// Materialize each `inherit_env` entry from `root` into `worktree_path`.
/// Files absent in root are silently skipped.
///
/// Per entry mode:
/// - `Symlink`: the worktree file is a symlink to root. Idempotent: correct symlinks
///   are left alone, stale/broken symlinks are replaced, real files are never clobbered.
/// - `Copy`: the worktree file is a real file copied from root, but only when no entry
///   currently exists at the destination. Once a copy exists (or the user has placed a
///   real file there), `ecluse up` re-runs leave it untouched so per-worktree edits stick.
///   Stale symlinks left over from a previous `symlink` configuration are replaced with
///   a fresh copy (treated as first-time init).
pub fn inherit_env_files(
    root: &Path,
    worktree_path: &Path,
    entries: &[crate::config::InheritEnvEntry],
    log: &crate::log::StepLogger,
) -> anyhow::Result<()> {
    use crate::config::InheritEnvMode;

    for entry in entries {
        let name = &entry.file;
        let src = root.join(name);
        if !src.exists() {
            continue;
        }
        let dst = worktree_path.join(name);

        match entry.mode {
            InheritEnvMode::Symlink => {
                if let Ok(target) = std::fs::read_link(&dst) {
                    if target == src {
                        // Already a correct symlink — nothing to do.
                        continue;
                    }
                    // Broken or stale symlink — replace it.
                    std::fs::remove_file(&dst).with_context(|| {
                        format!("failed to remove stale symlink {}", dst.display())
                    })?;
                } else if dst.exists() {
                    // Real file owned by the user — leave it alone.
                    log.detail(&format!("skipped {} (real file exists in worktree)", name));
                    continue;
                }
                std::os::unix::fs::symlink(&src, &dst)
                    .with_context(|| format!("failed to symlink {} into worktree", name))?;
                log.detail(&format!("symlinked {}", name));
            }
            InheritEnvMode::Copy => {
                // If a real file exists, preserve the user's per-worktree edits.
                let is_symlink = std::fs::symlink_metadata(&dst)
                    .map(|m| m.file_type().is_symlink())
                    .unwrap_or(false);
                if is_symlink {
                    // Previously symlinked under an older config — replace with a fresh copy.
                    std::fs::remove_file(&dst).with_context(|| {
                        format!("failed to remove stale symlink {}", dst.display())
                    })?;
                } else if dst.exists() {
                    log.detail(&format!("skipped {} (already in worktree)", name));
                    continue;
                }
                std::fs::copy(&src, &dst)
                    .with_context(|| format!("failed to copy {} into worktree", name))?;
                log.detail(&format!("copied {}", name));
            }
        }
    }
    Ok(())
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
/// Returns the user's choice. Returns Stop immediately when stdin is not a
/// terminal (CI, piped shells, Claude Code Bash tool).
pub fn prompt_worktree_removal(path: &Path) -> WorktreeRemovalChoice {
    // Non-interactive: stdin is a pipe, not a terminal. Return Stop so the
    // caller can emit an actionable error rather than blocking forever.
    if !is_tty_interactive() {
        return WorktreeRemovalChoice::Stop;
    }

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

/// Returns true only when stdin is connected to a terminal (i.e. a human is present).
/// False in CI, piped shells, and Claude Code's Bash tool (where stdin is a pipe).
fn is_tty_interactive() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal()
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
        // Disable signing: fixture commits must work without any signing setup.
        Command::new("git")
            .args(["-c", "commit.gpgsign=false"])
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
            slot_stride: 1,
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

    // remove(): directory already deleted by hand → prune fallback cleans the
    // administrative data and remove() reports success.
    #[test]
    fn remove_succeeds_when_directory_already_deleted() {
        let dir = TempDir::new().unwrap();
        setup_git_repo(dir.path());
        let wt = WorktreeManager::new(dir.path().to_owned());
        let config = make_config_for_worktree();
        let path = wt.worktree_path(&config, "gone");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        wt.create(&path, "ecluse/gone").unwrap();

        std::fs::remove_dir_all(&path).unwrap();
        wt.remove(&path).unwrap();
        assert!(!path.exists());
    }

    // remove(): a locked worktree cannot be removed — must error with the git
    // stderr, not report success while the directory survives on disk.
    #[test]
    fn remove_errors_when_worktree_is_locked() {
        let dir = TempDir::new().unwrap();
        setup_git_repo(dir.path());
        let wt = WorktreeManager::new(dir.path().to_owned());
        let config = make_config_for_worktree();
        let path = wt.worktree_path(&config, "locked");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        wt.create(&path, "ecluse/locked").unwrap();

        Command::new("git")
            .args(["worktree", "lock"])
            .arg(&path)
            .current_dir(dir.path())
            .output()
            .unwrap();

        let err = wt.remove(&path).unwrap_err();
        assert!(path.exists(), "locked worktree must not be deleted");
        assert!(err.to_string().contains("manually"), "got: {}", err);

        Command::new("git")
            .args(["worktree", "unlock"])
            .arg(&path)
            .current_dir(dir.path())
            .output()
            .unwrap();
        wt.remove(&path).unwrap();
    }

    /// Run git with signing disabled and a fixed identity; panic on failure
    /// so fixture problems surface as the real error, not a later assert.
    fn git_ok(dir: &std::path::Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "test")
            .env("GIT_AUTHOR_EMAIL", "test@test.com")
            .env("GIT_COMMITTER_NAME", "test")
            .env("GIT_COMMITTER_EMAIL", "test@test.com")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    // A branch that exists only on origin must be checked out as a tracking
    // branch, not forked from HEAD under the same name.
    #[test]
    fn create_tracks_remote_only_branch() {
        let upstream = TempDir::new().unwrap();
        git_ok(upstream.path(), &["init"]);
        git_ok(upstream.path(), &["commit", "--allow-empty", "-m", "init"]);
        // feat-remote points at the initial commit; the default branch then
        // moves ahead, so the clone's HEAD differs from origin/feat-remote.
        git_ok(upstream.path(), &["branch", "feat-remote"]);
        git_ok(
            upstream.path(),
            &["commit", "--allow-empty", "-m", "default moves ahead"],
        );

        let clone_parent = TempDir::new().unwrap();
        let clone_path = clone_parent.path().join("clone");
        git_ok(
            clone_parent.path(),
            &["clone", upstream.path().to_str().unwrap(), "clone"],
        );

        // feat-remote exists on origin only — not as a local branch.
        let wt = WorktreeManager::new(clone_path.clone());
        assert!(!wt.local_branch_exists("feat-remote").unwrap());
        assert!(wt.remote_branch_exists("feat-remote"));

        let config = make_config_for_worktree();
        let path = wt.worktree_path(&config, "feat-remote");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        wt.create(&path, "feat-remote").unwrap();

        // The worktree's branch must track origin/feat-remote and point at
        // the same commit.
        let upstream_ref = Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "feat-remote@{upstream}"])
            .current_dir(&path)
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&upstream_ref.stdout).trim(),
            "origin/feat-remote"
        );
        let local_head = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&path)
            .output()
            .unwrap();
        let remote_head = Command::new("git")
            .args(["rev-parse", "origin/feat-remote"])
            .current_dir(&clone_path)
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&local_head.stdout).trim(),
            String::from_utf8_lossy(&remote_head.stdout).trim(),
            "worktree must start at the remote branch tip, not at HEAD"
        );

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

    // ── inherit_env_files ─────────────────────────────────────────────────────

    use crate::config::InheritEnvEntry;

    fn setup_inherit_test(name: &str) -> (TempDir, std::path::PathBuf) {
        let dir = TempDir::new().unwrap();
        let wt_path = dir.path().join("worktree");
        std::fs::create_dir_all(&wt_path).unwrap();
        std::fs::write(dir.path().join(name), "ROOT=1\n").unwrap();
        (dir, wt_path)
    }

    // ── symlink mode ──────────────────────────────────────────────────────────

    // Fresh worktree (ecluse-created or user-created): file absent → create symlink.
    #[test]
    fn inherit_env_files_creates_symlink_when_dst_missing() {
        let (dir, wt_path) = setup_inherit_test(".env");
        let log = crate::log::StepLogger::new(true);
        inherit_env_files(
            dir.path(),
            &wt_path,
            &[InheritEnvEntry::symlink(".env")],
            &log,
        )
        .unwrap();

        let dst = wt_path.join(".env");
        assert!(dst.symlink_metadata().is_ok(), "symlink should exist");
        assert_eq!(
            std::fs::read_link(&dst).unwrap(),
            dir.path().join(".env"),
            "symlink should point to root"
        );
        assert_eq!(std::fs::read_to_string(&dst).unwrap(), "ROOT=1\n");
    }

    // Idempotent: already the correct symlink → leave unchanged, no error.
    #[test]
    fn inherit_env_files_skips_when_correct_symlink_exists() {
        let (dir, wt_path) = setup_inherit_test(".env");
        let src = dir.path().join(".env");
        let dst = wt_path.join(".env");
        std::os::unix::fs::symlink(&src, &dst).unwrap();

        let log = crate::log::StepLogger::new(true);
        inherit_env_files(
            dir.path(),
            &wt_path,
            &[InheritEnvEntry::symlink(".env")],
            &log,
        )
        .unwrap();

        assert_eq!(std::fs::read_link(&dst).unwrap(), src);
    }

    // Restart / reuse: stale symlink pointing elsewhere → replaced with correct symlink.
    #[test]
    fn inherit_env_files_replaces_stale_symlink() {
        let (dir, wt_path) = setup_inherit_test(".env");
        let dst = wt_path.join(".env");
        std::os::unix::fs::symlink("/tmp/does-not-exist-ecluse-test", &dst).unwrap();

        let log = crate::log::StepLogger::new(true);
        inherit_env_files(
            dir.path(),
            &wt_path,
            &[InheritEnvEntry::symlink(".env")],
            &log,
        )
        .unwrap();

        assert_eq!(
            std::fs::read_link(&dst).unwrap(),
            dir.path().join(".env"),
            "stale symlink should be replaced"
        );
    }

    // User-owned worktree: real file present → leave it alone, no clobber.
    #[test]
    fn inherit_env_files_symlink_skips_real_file_in_worktree() {
        let (dir, wt_path) = setup_inherit_test(".env");
        let dst = wt_path.join(".env");
        std::fs::write(&dst, "USER=custom\n").unwrap();

        let log = crate::log::StepLogger::new(true);
        inherit_env_files(
            dir.path(),
            &wt_path,
            &[InheritEnvEntry::symlink(".env")],
            &log,
        )
        .unwrap();

        assert!(
            std::fs::read_link(&dst).is_err(),
            "must not be converted to symlink"
        );
        assert_eq!(std::fs::read_to_string(&dst).unwrap(), "USER=custom\n");
    }

    // Source absent in root → no symlink created (file simply doesn't exist yet).
    #[test]
    fn inherit_env_files_skips_missing_src() {
        let dir = TempDir::new().unwrap();
        let wt_path = dir.path().join("worktree");
        std::fs::create_dir_all(&wt_path).unwrap();

        let log = crate::log::StepLogger::new(true);
        inherit_env_files(
            dir.path(),
            &wt_path,
            &[InheritEnvEntry::symlink(".env")],
            &log,
        )
        .unwrap();

        assert!(
            !wt_path.join(".env").exists(),
            "no symlink should be created"
        );
    }

    // Multiple symlink-mode files: each handled independently.
    #[test]
    fn inherit_env_files_handles_multiple_files_independently() {
        let dir = TempDir::new().unwrap();
        let wt_path = dir.path().join("worktree");
        std::fs::create_dir_all(&wt_path).unwrap();

        std::fs::write(dir.path().join(".env"), "A=1\n").unwrap();
        std::fs::write(dir.path().join(".env.local"), "B=2\n").unwrap();
        // .env.local already exists as a real file in the worktree
        std::fs::write(wt_path.join(".env.local"), "USER=local\n").unwrap();

        let log = crate::log::StepLogger::new(true);
        inherit_env_files(
            dir.path(),
            &wt_path,
            &[
                InheritEnvEntry::symlink(".env"),
                InheritEnvEntry::symlink(".env.local"),
            ],
            &log,
        )
        .unwrap();

        assert!(std::fs::read_link(wt_path.join(".env")).is_ok());
        assert!(std::fs::read_link(wt_path.join(".env.local")).is_err());
        assert_eq!(
            std::fs::read_to_string(wt_path.join(".env.local")).unwrap(),
            "USER=local\n"
        );
    }

    // ── copy mode ─────────────────────────────────────────────────────────────

    // Fresh worktree: file absent → copy from root, real file (not a symlink).
    #[test]
    fn inherit_env_files_copies_when_dst_missing() {
        let (dir, wt_path) = setup_inherit_test(".env.local");
        let log = crate::log::StepLogger::new(true);
        inherit_env_files(
            dir.path(),
            &wt_path,
            &[InheritEnvEntry::copy(".env.local")],
            &log,
        )
        .unwrap();

        let dst = wt_path.join(".env.local");
        assert!(dst.exists(), "copy should exist");
        assert!(
            std::fs::read_link(&dst).is_err(),
            "copy must not be a symlink"
        );
        assert_eq!(std::fs::read_to_string(&dst).unwrap(), "ROOT=1\n");
    }

    // Per-worktree edits are preserved: existing real file at dst → never re-copy.
    #[test]
    fn inherit_env_files_copy_skips_when_dst_exists() {
        let (dir, wt_path) = setup_inherit_test(".env.local");
        let dst = wt_path.join(".env.local");
        std::fs::write(&dst, "AUTH_ENABLED=false\n").unwrap();

        let log = crate::log::StepLogger::new(true);
        inherit_env_files(
            dir.path(),
            &wt_path,
            &[InheritEnvEntry::copy(".env.local")],
            &log,
        )
        .unwrap();

        assert_eq!(
            std::fs::read_to_string(&dst).unwrap(),
            "AUTH_ENABLED=false\n",
            "per-worktree edits must be preserved"
        );
    }

    // Mode migration: was symlinked under an older config; now configured as copy.
    // The stale symlink is replaced with a fresh copy (real file).
    #[test]
    fn inherit_env_files_copy_replaces_existing_symlink() {
        let (dir, wt_path) = setup_inherit_test(".env.local");
        let src = dir.path().join(".env.local");
        let dst = wt_path.join(".env.local");
        std::os::unix::fs::symlink(&src, &dst).unwrap();

        let log = crate::log::StepLogger::new(true);
        inherit_env_files(
            dir.path(),
            &wt_path,
            &[InheritEnvEntry::copy(".env.local")],
            &log,
        )
        .unwrap();

        assert!(
            std::fs::read_link(&dst).is_err(),
            "symlink should have been replaced with a copy"
        );
        assert_eq!(std::fs::read_to_string(&dst).unwrap(), "ROOT=1\n");
    }

    // Edits to root after copy do NOT propagate to existing worktree copies.
    #[test]
    fn inherit_env_files_copy_isolates_root_edits() {
        let (dir, wt_path) = setup_inherit_test(".env.local");
        let log = crate::log::StepLogger::new(true);

        // First ecluse up: copy created.
        inherit_env_files(
            dir.path(),
            &wt_path,
            &[InheritEnvEntry::copy(".env.local")],
            &log,
        )
        .unwrap();

        // Root file is edited later.
        std::fs::write(dir.path().join(".env.local"), "ROOT=2\n").unwrap();

        // Second ecluse up: copy must remain untouched (still ROOT=1).
        inherit_env_files(
            dir.path(),
            &wt_path,
            &[InheritEnvEntry::copy(".env.local")],
            &log,
        )
        .unwrap();

        assert_eq!(
            std::fs::read_to_string(wt_path.join(".env.local")).unwrap(),
            "ROOT=1\n",
            "worktree copy must be isolated from root edits"
        );
    }

    // Mixed modes in a single call: symlink and copy entries handled independently.
    #[test]
    fn inherit_env_files_mixed_modes_handled_independently() {
        let dir = TempDir::new().unwrap();
        let wt_path = dir.path().join("worktree");
        std::fs::create_dir_all(&wt_path).unwrap();

        std::fs::write(dir.path().join(".env"), "SHARED=secret\n").unwrap();
        std::fs::write(dir.path().join(".env.local"), "AUTH=true\n").unwrap();

        let log = crate::log::StepLogger::new(true);
        inherit_env_files(
            dir.path(),
            &wt_path,
            &[
                InheritEnvEntry::symlink(".env"),
                InheritEnvEntry::copy(".env.local"),
            ],
            &log,
        )
        .unwrap();

        // .env is a symlink to root.
        assert!(std::fs::read_link(wt_path.join(".env")).is_ok());
        assert_eq!(
            std::fs::read_to_string(wt_path.join(".env")).unwrap(),
            "SHARED=secret\n"
        );
        // .env.local is a real file (copy).
        assert!(std::fs::read_link(wt_path.join(".env.local")).is_err());
        assert_eq!(
            std::fs::read_to_string(wt_path.join(".env.local")).unwrap(),
            "AUTH=true\n"
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
