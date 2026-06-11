use anyhow::{Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::config::Mode;
use crate::process::{ProcessManager, SpawnResult};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct State {
    #[serde(default = "default_version")]
    pub version: u8,
    #[serde(default)]
    pub sessions: Vec<Session>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            version: 1,
            sessions: Vec::new(),
        }
    }
}

fn default_version() -> u8 {
    1
}

/// A compose file together with the overlay ecluse generated for it.
/// Persisted as a pair so teardown never has to reconstruct which compose
/// file an overlay belongs to from its filename.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct ComposeOverlay {
    pub compose: String,
    pub overlay: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Session {
    pub slug: String,
    pub mode: Mode,
    pub slot: u8,
    pub branch: String,
    pub worktree_path: String,
    pub compose_project: Option<String>,
    /// Legacy: primary overlay path. Still written for older binaries;
    /// teardown prefers `compose_overlays`.
    pub overlay_file: Option<String>,
    /// Legacy: extra overlay paths (monorepo). Still written for older
    /// binaries; teardown prefers `compose_overlays`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub overlay_files: Vec<String>,
    /// (compose, overlay) pairs recorded at bring_up. The authoritative
    /// source for teardown; empty for sessions written by older versions,
    /// which fall back to the legacy fields above.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub compose_overlays: Vec<ComposeOverlay>,
    pub app_port: Option<u16>,
    pub started_at: String,
    /// Actual allocated ports (may differ from nominal if auto-bump kicked in).
    /// Stored so `ecluse env` always reflects what was really assigned.
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub port_overrides: std::collections::HashMap<String, u16>,
    /// Process manager used to spawn native service commands, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_manager: Option<ProcessManager>,
    /// tmux session name when process_manager = tmux.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tmux_session: Option<String>,
    /// PID files written when process_manager = nohup.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pid_files: Vec<PathBuf>,
    /// Log directory for nohup-managed services.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_dir: Option<PathBuf>,
    /// Services subset requested via --services; None means all services were started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub services_subset: Option<Vec<String>>,
}

impl Session {
    /// Reconstitute a SpawnResult from persisted session fields.
    pub fn spawn_result(&self) -> SpawnResult {
        SpawnResult {
            tmux_session: self.tmux_session.clone(),
            pid_files: self.pid_files.clone(),
            log_dir: self.log_dir.clone(),
        }
    }
}

impl State {
    pub fn find_session(&self, slug: &str) -> Option<&Session> {
        self.sessions.iter().find(|s| s.slug == slug)
    }

    pub fn add_session(&mut self, session: Session) {
        self.sessions.push(session);
    }

    pub fn remove_session(&mut self, slug: &str) -> Option<Session> {
        if let Some(pos) = self.sessions.iter().position(|s| s.slug == slug) {
            Some(self.sessions.remove(pos))
        } else {
            None
        }
    }

    pub fn used_slots(&self) -> Vec<u8> {
        self.sessions.iter().map(|s| s.slot).collect()
    }
}

pub struct StateGuard {
    pub state: State,
    state_path: PathBuf,
    _lock_file: File,
}

impl StateGuard {
    pub fn acquire(root: &Path) -> Result<Self> {
        let ecluse_dir = root.join(".ecluse");
        std::fs::create_dir_all(&ecluse_dir)
            .with_context(|| format!("failed to create {}", ecluse_dir.display()))?;

        let lock_path = ecluse_dir.join("state.lock");
        let lock_file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
            .with_context(|| format!("failed to open lock file {}", lock_path.display()))?;

        // Try to acquire with a timeout by polling
        let start = std::time::Instant::now();
        loop {
            match lock_file.try_lock_exclusive() {
                Ok(()) => break,
                Err(_) => {
                    if start.elapsed() >= Duration::from_secs(10) {
                        return Err(crate::error::EcluseError::LockTimeout.into());
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        }

        let state_path = ecluse_dir.join("state.json");

        // Clean up any stale .tmp left by a previous crash between write and rename.
        let tmp_path = state_path.with_extension("json.tmp");
        if tmp_path.exists() {
            let _ = std::fs::remove_file(&tmp_path);
        }

        let state = if state_path.exists() {
            let content =
                std::fs::read_to_string(&state_path).context("failed to read state.json")?;
            serde_json::from_str(&content)
                .with_context(|| crate::error::EcluseError::StateCorrupt(content.clone()))?
        } else {
            State::default()
        };

        Ok(StateGuard {
            state,
            state_path,
            _lock_file: lock_file,
        })
    }

    /// Acquire a shared (read-only) lock. Multiple readers can hold this simultaneously.
    /// Use for commands that only read state: status, ls, env, shell.
    pub fn acquire_shared(root: &Path) -> Result<Self> {
        let ecluse_dir = root.join(".ecluse");
        std::fs::create_dir_all(&ecluse_dir)
            .with_context(|| format!("failed to create {}", ecluse_dir.display()))?;

        // Create the lock file if missing. state.json may still exist (e.g.
        // the lock file was removed by hand) — never report an empty state
        // just because the lock file is gone.
        let lock_path = ecluse_dir.join("state.lock");
        let lock_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .with_context(|| format!("failed to open lock file {}", lock_path.display()))?;

        let start = std::time::Instant::now();
        loop {
            match lock_file.try_lock_shared() {
                Ok(()) => break,
                Err(_) => {
                    if start.elapsed() >= Duration::from_secs(10) {
                        return Err(crate::error::EcluseError::LockTimeout.into());
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        }

        let state_path = ecluse_dir.join("state.json");
        let state = if state_path.exists() {
            let content =
                std::fs::read_to_string(&state_path).context("failed to read state.json")?;
            serde_json::from_str(&content)
                .with_context(|| crate::error::EcluseError::StateCorrupt(content.clone()))?
        } else {
            State::default()
        };

        Ok(StateGuard {
            state,
            state_path,
            _lock_file: lock_file,
        })
    }

    pub fn commit(&self) -> Result<()> {
        let tmp = self.state_path.with_extension("json.tmp");
        let data =
            serde_json::to_string_pretty(&self.state).context("failed to serialize state")?;
        std::fs::write(&tmp, &data).context("failed to write state.json.tmp")?;
        std::fs::rename(&tmp, &self.state_path)
            .context("failed to atomically update state.json")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Mode;
    use tempfile::TempDir;

    fn make_session(slug: &str, slot: u8) -> Session {
        Session {
            slug: slug.to_string(),
            mode: Mode::Host,
            slot,
            branch: format!("branch/{}", slug),
            worktree_path: format!("/tmp/{}", slug),
            compose_project: None,
            overlay_file: None,
            overlay_files: vec![],
            compose_overlays: vec![],
            app_port: None,
            started_at: "2026-01-01T00:00:00Z".into(),
            port_overrides: std::collections::HashMap::new(),
            process_manager: None,
            tmux_session: None,
            pid_files: vec![],
            log_dir: None,
            services_subset: None,
        }
    }

    // ── State methods ─────────────────────────────────────────────────────────

    #[test]
    fn find_session_returns_existing() {
        let mut state = State::default();
        state.add_session(make_session("alpha", 1));
        assert!(state.find_session("alpha").is_some());
    }

    #[test]
    fn find_session_returns_none_when_missing() {
        let state = State::default();
        assert!(state.find_session("ghost").is_none());
    }

    #[test]
    fn add_session_increases_count() {
        let mut state = State::default();
        assert_eq!(state.sessions.len(), 0);
        state.add_session(make_session("alpha", 1));
        assert_eq!(state.sessions.len(), 1);
    }

    #[test]
    fn remove_session_returns_removed_session() {
        let mut state = State::default();
        state.add_session(make_session("alpha", 1));
        let removed = state.remove_session("alpha");
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().slug, "alpha");
        assert!(state.sessions.is_empty());
    }

    #[test]
    fn remove_session_returns_none_when_absent() {
        let mut state = State::default();
        assert!(state.remove_session("ghost").is_none());
    }

    #[test]
    fn used_slots_reflects_all_sessions() {
        let mut state = State::default();
        state.add_session(make_session("a", 1));
        state.add_session(make_session("b", 3));
        state.add_session(make_session("c", 5));
        let slots = state.used_slots();
        assert!(slots.contains(&1));
        assert!(slots.contains(&3));
        assert!(slots.contains(&5));
        assert_eq!(slots.len(), 3);
    }

    #[test]
    fn used_slots_empty_when_no_sessions() {
        let state = State::default();
        assert!(state.used_slots().is_empty());
    }

    #[test]
    fn state_default_has_version_1() {
        let state = State::default();
        assert_eq!(state.version, 1);
    }

    // ── StateGuard acquire / commit ───────────────────────────────────────────

    #[test]
    fn acquire_creates_ecluse_dir_and_empty_state() {
        let dir = TempDir::new().unwrap();
        let guard = StateGuard::acquire(dir.path()).unwrap();
        assert!(dir.path().join(".ecluse").exists());
        assert!(guard.state.sessions.is_empty());
    }

    #[test]
    fn acquire_reads_existing_state_json() {
        let dir = TempDir::new().unwrap();
        let ecluse = dir.path().join(".ecluse");
        std::fs::create_dir_all(&ecluse).unwrap();
        let state = State {
            version: 1,
            sessions: vec![make_session("alpha", 1)],
        };
        let json = serde_json::to_string_pretty(&state).unwrap();
        std::fs::write(ecluse.join("state.json"), json).unwrap();

        let guard = StateGuard::acquire(dir.path()).unwrap();
        assert_eq!(guard.state.sessions.len(), 1);
        assert_eq!(guard.state.sessions[0].slug, "alpha");
    }

    #[test]
    fn acquire_errors_on_corrupt_state_json() {
        let dir = TempDir::new().unwrap();
        let ecluse = dir.path().join(".ecluse");
        std::fs::create_dir_all(&ecluse).unwrap();
        std::fs::write(ecluse.join("state.json"), "{ not valid json {{").unwrap();

        match StateGuard::acquire(dir.path()) {
            Ok(_) => panic!("expected error for corrupt state"),
            Err(e) => {
                let msg = e.to_string();
                assert!(
                    msg.contains("corrupt") || msg.contains("state"),
                    "got: {}",
                    msg
                );
            }
        }
    }

    #[test]
    fn commit_writes_and_sessions_persist() {
        let dir = TempDir::new().unwrap();
        {
            let mut guard = StateGuard::acquire(dir.path()).unwrap();
            guard.state.add_session(make_session("beta", 2));
            guard.commit().unwrap();
            // guard dropped here, releasing lock
        }
        let guard2 = StateGuard::acquire(dir.path()).unwrap();
        assert_eq!(guard2.state.sessions.len(), 1);
        assert_eq!(guard2.state.sessions[0].slug, "beta");
    }

    #[test]
    fn commit_is_atomic_tmp_file_gone_after() {
        let dir = TempDir::new().unwrap();
        let mut guard = StateGuard::acquire(dir.path()).unwrap();
        guard.state.add_session(make_session("gamma", 1));
        guard.commit().unwrap();

        let tmp = dir.path().join(".ecluse").join("state.json.tmp");
        assert!(
            !tmp.exists(),
            ".json.tmp should be removed after atomic rename"
        );
    }

    #[test]
    fn multiple_sessions_roundtrip() {
        let dir = TempDir::new().unwrap();
        {
            let mut guard = StateGuard::acquire(dir.path()).unwrap();
            guard.state.add_session(make_session("a", 1));
            guard.state.add_session(make_session("b", 2));
            guard.state.add_session(make_session("c", 3));
            guard.commit().unwrap();
        }
        let guard2 = StateGuard::acquire(dir.path()).unwrap();
        assert_eq!(guard2.state.sessions.len(), 3);
    }

    #[test]
    fn session_with_process_manager_roundtrips() {
        let dir = TempDir::new().unwrap();
        {
            let mut guard = StateGuard::acquire(dir.path()).unwrap();
            guard.state.add_session(Session {
                slug: "pm-sess".into(),
                mode: Mode::Host,
                slot: 1,
                branch: "branch/pm-sess".into(),
                worktree_path: "/tmp/pm-sess".into(),
                compose_project: None,
                overlay_file: None,
                overlay_files: vec![],
                compose_overlays: vec![],
                app_port: Some(3001),
                started_at: "2026-01-01T00:00:00Z".into(),
                port_overrides: std::collections::HashMap::new(),
                process_manager: Some(crate::process::ProcessManager::Tmux),
                tmux_session: Some("ecluse-pm-sess".into()),
                pid_files: vec![],
                log_dir: None,
                services_subset: None,
            });
            guard.commit().unwrap();
        }
        let guard2 = StateGuard::acquire(dir.path()).unwrap();
        let s = &guard2.state.sessions[0];
        assert_eq!(
            s.process_manager,
            Some(crate::process::ProcessManager::Tmux)
        );
        assert_eq!(s.tmux_session.as_deref(), Some("ecluse-pm-sess"));
    }

    #[test]
    fn session_pid_files_roundtrip() {
        let dir = TempDir::new().unwrap();
        {
            let mut guard = StateGuard::acquire(dir.path()).unwrap();
            guard.state.add_session(Session {
                slug: "nohup-sess".into(),
                mode: Mode::Host,
                slot: 2,
                branch: "branch/nohup-sess".into(),
                worktree_path: "/tmp/nohup-sess".into(),
                compose_project: None,
                overlay_file: None,
                overlay_files: vec![],
                compose_overlays: vec![],
                app_port: None,
                started_at: "2026-01-01T00:00:00Z".into(),
                port_overrides: std::collections::HashMap::new(),
                process_manager: Some(crate::process::ProcessManager::Nohup),
                tmux_session: None,
                pid_files: vec![PathBuf::from("/tmp/api.pid")],
                log_dir: Some(PathBuf::from("/tmp/logs")),
                services_subset: None,
            });
            guard.commit().unwrap();
        }
        let guard2 = StateGuard::acquire(dir.path()).unwrap();
        let s = &guard2.state.sessions[0];
        assert_eq!(s.pid_files, vec![PathBuf::from("/tmp/api.pid")]);
        assert_eq!(s.log_dir, Some(PathBuf::from("/tmp/logs")));
    }

    #[test]
    fn session_with_compose_fields_roundtrips() {
        let dir = TempDir::new().unwrap();
        {
            let mut guard = StateGuard::acquire(dir.path()).unwrap();
            guard.state.add_session(Session {
                slug: "compose-sess".into(),
                mode: Mode::Container,
                slot: 1,
                branch: "branch/compose-sess".into(),
                worktree_path: "/tmp/wt".into(),
                compose_project: Some("ecluse_compose-sess".into()),
                overlay_file: Some("/tmp/overlay.yml".into()),
                overlay_files: vec![],
                compose_overlays: vec![],
                app_port: Some(3001),
                started_at: "2026-01-01T00:00:00Z".into(),
                port_overrides: std::collections::HashMap::new(),
                process_manager: None,
                tmux_session: None,
                pid_files: vec![],
                log_dir: None,
                services_subset: None,
            });
            guard.commit().unwrap();
        }
        let guard2 = StateGuard::acquire(dir.path()).unwrap();
        let s = &guard2.state.sessions[0];
        assert_eq!(s.compose_project.as_deref(), Some("ecluse_compose-sess"));
        assert_eq!(s.app_port, Some(3001));
    }

    #[test]
    fn session_services_subset_roundtrips() {
        let dir = TempDir::new().unwrap();
        {
            let mut guard = StateGuard::acquire(dir.path()).unwrap();
            let mut s = make_session("sub-sess", 1);
            s.services_subset = Some(vec!["api".into(), "postgres".into()]);
            guard.state.add_session(s);
            guard.commit().unwrap();
        }
        let guard2 = StateGuard::acquire(dir.path()).unwrap();
        let s = &guard2.state.sessions[0];
        assert_eq!(
            s.services_subset,
            Some(vec!["api".into(), "postgres".into()])
        );
    }

    #[test]
    fn session_services_subset_none_not_serialized() {
        let s = make_session("no-sub", 1);
        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains("services_subset"), "got: {json}");
    }

    // The lock file can go missing while state.json survives (manual cleanup,
    // partial flush). Shared acquisition must recreate the lock and read the
    // real state, not silently report "no sessions".
    #[test]
    fn acquire_shared_reads_state_when_lock_file_missing() {
        let dir = TempDir::new().unwrap();
        {
            let mut guard = StateGuard::acquire(dir.path()).unwrap();
            guard.state.add_session(make_session("survivor", 1));
            guard.commit().unwrap();
        }
        std::fs::remove_file(dir.path().join(".ecluse/state.lock")).unwrap();

        let guard = StateGuard::acquire_shared(dir.path()).unwrap();
        assert_eq!(guard.state.sessions.len(), 1);
        assert_eq!(guard.state.sessions[0].slug, "survivor");
    }

    // ── compose_overlays ──────────────────────────────────────────────────────

    #[test]
    fn compose_overlays_roundtrip() {
        let mut s = make_session("pairs", 1);
        s.compose_overlays = vec![ComposeOverlay {
            compose: "/repo/docker-compose.yml".into(),
            overlay: "/repo/.ecluse/overlays/pairs.yml".into(),
        }];
        let json = serde_json::to_string(&s).unwrap();
        let back: Session = serde_json::from_str(&json).unwrap();
        assert_eq!(back.compose_overlays, s.compose_overlays);
    }

    #[test]
    fn legacy_state_without_compose_overlays_defaults_to_empty() {
        // state.json written by an older ecluse has no compose_overlays field.
        let s = make_session("old", 1);
        let mut json = serde_json::to_value(&s).unwrap();
        json.as_object_mut().unwrap().remove("compose_overlays");
        let back: Session = serde_json::from_value(json).unwrap();
        assert!(back.compose_overlays.is_empty());
    }
}
