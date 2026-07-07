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

/// Lifecycle state of a session entry.
///
/// `Pending` reserves the slug + slot while an `up`/`down` runs *without*
/// holding the state lock — provisioning can take minutes (image pulls,
/// hooks) and must not block every other ecluse command. A `Pending` entry
/// that never transitions back means the operation crashed; `ecluse down
/// <slug>` cleans it up.
///
/// `Stopped` means `ecluse down --keep-worktree` completed: services are
/// down and the worktree is on disk. The entry stays in state so that the
/// next `ecluse up` from inside that worktree resumes at the same slot
/// instead of allocating a new one (which would change all ports).
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
    #[default]
    Active,
    Pending,
    Stopped,
}

fn is_active(status: &SessionStatus) -> bool {
    *status == SessionStatus::Active
}

/// Identity of the in-flight operation that marked a session Pending.
///
/// `id` lets the owning command verify nothing took the session over while it
/// worked without holding the lock — a finalize that has lost ownership must
/// not write state (it would resurrect an entry another command deleted).
/// `since` lets `ls` flag entries whose owning operation likely crashed.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct PendingOp {
    pub id: String,
    pub since: String,
}

/// Fresh operation id: unique enough to distinguish two concurrent commands.
///
/// `pid` disambiguates across processes; the process-local counter guarantees
/// two ids minted in the same process (even within one nanosecond) never
/// collide, and the timestamp keeps ids human-readable / ordered.
pub fn new_op_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    format!(
        "{}-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Session {
    pub slug: String,
    pub mode: Mode,
    pub slot: u8,
    pub branch: String,
    pub worktree_path: String,
    /// `Active` during normal operation; `Pending` while an up/down is in
    /// flight; `Stopped` after `ecluse down --keep-worktree` (slot reserved
    /// until the worktree is revived via `ecluse up`). Defaults to Active and
    /// is omitted from JSON when Active, so state.json files written by older
    /// versions load unchanged and stay byte-compatible.
    #[serde(default, skip_serializing_if = "is_active")]
    pub status: SessionStatus,
    /// Present iff status == Pending: identifies the operation that owns this
    /// entry. Maintained by `State::mark_pending` / the finalize paths.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_op: Option<PendingOp>,
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

    pub fn find_session_mut(&mut self, slug: &str) -> Option<&mut Session> {
        self.sessions.iter_mut().find(|s| s.slug == slug)
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

    /// Mark `slug` Pending under a fresh operation id, taking ownership of the
    /// entry (including from a previous operation that crashed mid-flight).
    /// Returns the session as it was before marking plus the op id the caller
    /// must present to `still_owned` when finalizing.
    pub fn mark_pending(&mut self, slug: &str) -> Option<(Session, String)> {
        let pos = self.sessions.iter().position(|s| s.slug == slug)?;
        let original = self.sessions[pos].clone();
        let op_id = new_op_id();
        self.sessions[pos].status = SessionStatus::Pending;
        self.sessions[pos].pending_op = Some(PendingOp {
            id: op_id.clone(),
            since: chrono::Utc::now().to_rfc3339(),
        });
        Some((original, op_id))
    }

    /// Transition `slug` to `Stopped`, clearing all service runtime state so a
    /// later `ecluse up` from inside the kept worktree resumes at the same slot.
    /// Returns an error if the slug is absent — callers reach this only after
    /// `still_owned` confirmed the entry, so a missing slug is a broken
    /// invariant that must surface loudly rather than silently skip the update.
    pub fn mark_stopped(&mut self, slug: &str) -> Result<()> {
        let s = self.find_session_mut(slug).ok_or_else(|| {
            anyhow::anyhow!("mark_stopped: session '{slug}' not found (state invariant broken)")
        })?;
        s.status = SessionStatus::Stopped;
        s.pending_op = None;
        s.tmux_session = None;
        s.pid_files.clear();
        s.log_dir = None;
        s.port_overrides.clear();
        s.app_port = None;
        Ok(())
    }

    /// True while the Pending entry written under `op_id` is still in place —
    /// i.e. no other command removed or took over the session in the meantime.
    pub fn still_owned(&self, slug: &str, op_id: &str) -> bool {
        self.find_session(slug)
            .and_then(|s| s.pending_op.as_ref())
            .is_some_and(|op| op.id == op_id)
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
            status: SessionStatus::Active,
            pending_op: None,
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
                status: SessionStatus::Active,
                pending_op: None,
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
                status: SessionStatus::Active,
                pending_op: None,
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
                status: SessionStatus::Active,
                pending_op: None,
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

    // ── SessionStatus ─────────────────────────────────────────────────────────

    #[test]
    fn active_status_not_serialized() {
        // Keeps state.json byte-compatible with older versions for active sessions.
        let s = make_session("plain", 1);
        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains("status"), "got: {json}");
    }

    #[test]
    fn pending_status_roundtrips() {
        let mut s = make_session("busy", 1);
        s.status = SessionStatus::Pending;
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("pending"), "got: {json}");
        let back: Session = serde_json::from_str(&json).unwrap();
        assert_eq!(back.status, SessionStatus::Pending);
    }

    #[test]
    fn session_without_status_field_defaults_to_active() {
        // state.json written by an older ecluse has no status field.
        let s = make_session("old", 1);
        let mut json: serde_json::Value = serde_json::to_value(&s).unwrap();
        json.as_object_mut().unwrap().remove("status");
        let back: Session = serde_json::from_value(json).unwrap();
        assert_eq!(back.status, SessionStatus::Active);
    }

    // ── mark_pending / still_owned ────────────────────────────────────────────

    #[test]
    fn mark_pending_sets_status_and_op() {
        let mut state = State::default();
        state.add_session(make_session("busy", 1));
        let (original, op_id) = state.mark_pending("busy").unwrap();
        assert_eq!(original.status, SessionStatus::Active);
        let s = state.find_session("busy").unwrap();
        assert_eq!(s.status, SessionStatus::Pending);
        assert_eq!(s.pending_op.as_ref().unwrap().id, op_id);
        assert!(state.still_owned("busy", &op_id));
    }

    // mark_pending mutates the entry in place — it does NOT remove it. A restore
    // path must therefore remove-then-add, or it duplicates the session. This
    // pins that invariant so the teardown-failure restore can't silently regress.
    #[test]
    fn mark_pending_keeps_a_single_entry() {
        let mut state = State::default();
        state.add_session(make_session("busy", 1));
        state.mark_pending("busy").unwrap();
        assert_eq!(
            state.sessions.iter().filter(|s| s.slug == "busy").count(),
            1
        );
        // Restore pattern: remove the Pending entry, then re-add — still one.
        let original = state.remove_session("busy").unwrap();
        state.add_session(original);
        assert_eq!(
            state.sessions.iter().filter(|s| s.slug == "busy").count(),
            1
        );
    }

    #[test]
    fn mark_pending_missing_session_returns_none() {
        let mut state = State::default();
        assert!(state.mark_pending("ghost").is_none());
    }

    // A second mark_pending takes the entry over: the first operation's
    // finalize must stand down instead of resurrecting a deleted session.
    #[test]
    fn second_mark_pending_takes_over_ownership() {
        let mut state = State::default();
        state.add_session(make_session("busy", 1));
        let (_, first_op) = state.mark_pending("busy").unwrap();
        let (taken_over, second_op) = state.mark_pending("busy").unwrap();
        assert_eq!(taken_over.status, SessionStatus::Pending);
        assert!(!state.still_owned("busy", &first_op));
        assert!(state.still_owned("busy", &second_op));
    }

    #[test]
    fn still_owned_false_after_removal() {
        let mut state = State::default();
        state.add_session(make_session("busy", 1));
        let (_, op_id) = state.mark_pending("busy").unwrap();
        state.remove_session("busy");
        assert!(!state.still_owned("busy", &op_id));
    }

    #[test]
    fn new_op_ids_are_unique() {
        assert_ne!(new_op_id(), new_op_id());
    }

    #[test]
    fn pending_op_roundtrips_in_state_json() {
        let mut state = State::default();
        state.add_session(make_session("busy", 1));
        let (_, op_id) = state.mark_pending("busy").unwrap();
        let json = serde_json::to_string(&state).unwrap();
        let back: State = serde_json::from_str(&json).unwrap();
        assert!(back.still_owned("busy", &op_id));
    }

    #[test]
    fn pending_sessions_still_reserve_slots() {
        let mut state = State::default();
        let mut s = make_session("busy", 3);
        s.status = SessionStatus::Pending;
        state.add_session(s);
        assert!(state.used_slots().contains(&3));
    }

    // ── Stopped status ────────────────────────────────────────────────────────

    #[test]
    fn stopped_status_serializes_and_deserializes() {
        let mut s = make_session("kept", 3);
        s.status = SessionStatus::Stopped;
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"status\":\"stopped\""), "got: {json}");
        let back: Session = serde_json::from_str(&json).unwrap();
        assert_eq!(back.status, SessionStatus::Stopped);
    }

    #[test]
    fn stopped_slot_is_counted_as_used() {
        let mut state = State::default();
        let mut s = make_session("kept", 3);
        s.status = SessionStatus::Stopped;
        state.add_session(s);
        assert!(state.used_slots().contains(&3));
    }

    #[test]
    fn find_session_mut_updates_status() {
        let mut state = State::default();
        state.add_session(make_session("x", 1));
        state.find_session_mut("x").unwrap().status = SessionStatus::Stopped;
        assert_eq!(
            state.find_session("x").unwrap().status,
            SessionStatus::Stopped
        );
    }

    #[test]
    fn mark_stopped_transitions_and_clears_runtime_state() {
        let mut state = State::default();
        let mut s = make_session("kept", 2);
        s.status = SessionStatus::Pending;
        s.pending_op = Some(PendingOp {
            id: "op".into(),
            since: "now".into(),
        });
        s.tmux_session = Some("sess".into());
        s.pid_files = vec![PathBuf::from("/tmp/pid")];
        s.log_dir = Some(PathBuf::from("/tmp/log"));
        s.port_overrides.insert("web".into(), 3002);
        s.app_port = Some(3002);
        state.add_session(s);

        state.mark_stopped("kept").unwrap();

        let back = state.find_session("kept").unwrap();
        assert_eq!(back.status, SessionStatus::Stopped);
        assert!(back.pending_op.is_none());
        assert!(back.tmux_session.is_none());
        assert!(back.pid_files.is_empty());
        assert!(back.log_dir.is_none());
        assert!(back.port_overrides.is_empty());
        assert!(back.app_port.is_none());
        // Identity fields must survive so `ecluse up` resumes the same worktree.
        assert_eq!(back.slug, "kept");
        assert_eq!(back.slot, 2);
        assert_eq!(back.worktree_path, "/tmp/kept");
        assert_eq!(back.branch, "branch/kept");
        assert_eq!(back.mode, Mode::Host);
        // Slot stays reserved so `ecluse up` resumes at the same slot.
        assert!(state.used_slots().contains(&2));
    }

    #[test]
    fn mark_stopped_missing_session_errors() {
        let mut state = State::default();
        assert!(state.mark_stopped("ghost").is_err());
    }
}
