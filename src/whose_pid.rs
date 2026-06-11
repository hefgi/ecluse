use std::path::Path;
use std::process::Command;

use crate::process::ProcessManager;
use crate::state::Session;

/// Result of a PID → session lookup.
#[derive(Debug, Clone, PartialEq)]
pub struct PidOwner {
    pub slug: String,
    pub slot: u8,
    pub service: Option<String>,
    pub port: Option<u16>,
}

/// Resolve `pid` to the ecluse session that owns it, if any.
///
/// Lookup strategy, in order:
/// 1. Match against `.ecluse/pids/<slug>/<service>.pid` files written by sync or nohup spawn.
/// 2. For tmux-managed sessions, match against pane PIDs and their descendants.
///
/// Returns `None` if the PID is not owned by any tracked ecluse session.
pub fn resolve(root: &Path, sessions: &[Session], pid: u32) -> Option<PidOwner> {
    for session in sessions {
        if let Some(owner) = match_pid_files(root, session, pid) {
            return Some(owner);
        }
        if matches!(session.process_manager, Some(ProcessManager::Tmux)) {
            if let Some(owner) = match_tmux_session(session, pid) {
                return Some(owner);
            }
        }
    }
    None
}

fn match_pid_files(root: &Path, session: &Session, pid: u32) -> Option<PidOwner> {
    let pid_dir = root.join(".ecluse").join("pids").join(&session.slug);
    if !pid_dir.exists() {
        return None;
    }
    let entries = std::fs::read_dir(&pid_dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("pid") {
            continue;
        }
        let Some((tracked_pid, token)) = crate::process::read_pid_file(&path) else {
            continue;
        };
        // A live PID whose start token no longer matches was recycled by an
        // unrelated process — it must not be attributed to this session.
        if crate::process::pid_alive(tracked_pid)
            && !crate::process::pid_file_alive(tracked_pid, &token)
        {
            continue;
        }
        // Match either the tracked PID directly or any descendant of it.
        if tracked_pid == pid || is_descendant(tracked_pid, pid) {
            let service = path.file_stem().and_then(|s| s.to_str()).map(String::from);
            let port = service
                .as_ref()
                .and_then(|s| session.port_overrides.get(s).copied());
            return Some(PidOwner {
                slug: session.slug.clone(),
                slot: session.slot,
                service,
                port,
            });
        }
    }
    None
}

fn match_tmux_session(session: &Session, pid: u32) -> Option<PidOwner> {
    let tmux_name = session.tmux_session.as_ref()?;
    let output = Command::new("tmux")
        .args([
            "list-panes",
            "-t",
            tmux_name,
            "-aF",
            "#{window_name} #{pane_pid}",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let mut parts = line.split_whitespace();
        let window = parts.next()?.to_string();
        let pane_pid: u32 = parts.next()?.parse().ok()?;
        if pane_pid == pid || is_descendant(pane_pid, pid) {
            let port = session.port_overrides.get(&window).copied();
            return Some(PidOwner {
                slug: session.slug.clone(),
                slot: session.slot,
                service: Some(window),
                port,
            });
        }
    }
    None
}

/// True iff `descendant` is a transitive child of `ancestor` (up to 5 levels deep).
fn is_descendant(ancestor: u32, descendant: u32) -> bool {
    if ancestor == descendant {
        return false;
    }
    let mut frontier = vec![ancestor];
    for _ in 0..5 {
        let mut next = Vec::new();
        for pid in &frontier {
            for child in child_pids(*pid) {
                if child == descendant {
                    return true;
                }
                next.push(child);
            }
        }
        if next.is_empty() {
            return false;
        }
        frontier = next;
    }
    false
}

fn child_pids(pid: u32) -> Vec<u32> {
    let output = match Command::new("pgrep")
        .args(["-P", &pid.to_string()])
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return vec![],
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|l| l.trim().parse().ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Mode;
    use std::collections::HashMap;
    use tempfile::TempDir;

    fn make_session(slug: &str, slot: u8) -> Session {
        Session {
            slug: slug.to_string(),
            mode: Mode::Host,
            slot,
            branch: format!("branch/{}", slug),
            worktree_path: format!("/tmp/{}", slug),
            status: crate::state::SessionStatus::Active,
            pending_op: None,
            compose_project: None,
            overlay_file: None,
            overlay_files: vec![],
            compose_overlays: vec![],
            app_port: None,
            started_at: "2026-01-01T00:00:00Z".into(),
            port_overrides: HashMap::new(),
            process_manager: Some(ProcessManager::Nohup),
            tmux_session: None,
            pid_files: vec![],
            log_dir: None,
            services_subset: None,
        }
    }

    fn write_pid_file(root: &Path, slug: &str, service: &str, pid: u32) {
        let pid_dir = root.join(".ecluse").join("pids").join(slug);
        std::fs::create_dir_all(&pid_dir).unwrap();
        std::fs::write(pid_dir.join(format!("{service}.pid")), pid.to_string()).unwrap();
    }

    #[test]
    fn resolve_returns_none_when_no_sessions() {
        let dir = TempDir::new().unwrap();
        let result = resolve(dir.path(), &[], 12345);
        assert!(result.is_none());
    }

    #[test]
    fn resolve_returns_none_when_pid_not_tracked() {
        let dir = TempDir::new().unwrap();
        let session = make_session("alpha", 1);
        let result = resolve(dir.path(), &[session], 99999);
        assert!(result.is_none());
    }

    #[test]
    fn resolve_matches_direct_pid_file() {
        let dir = TempDir::new().unwrap();
        let my_pid = std::process::id();
        write_pid_file(dir.path(), "alpha", "api", my_pid);
        let session = make_session("alpha", 2);
        let result = resolve(dir.path(), &[session], my_pid).unwrap();
        assert_eq!(result.slug, "alpha");
        assert_eq!(result.slot, 2);
        assert_eq!(result.service.as_deref(), Some("api"));
    }

    #[test]
    fn resolve_includes_port_when_recorded() {
        let dir = TempDir::new().unwrap();
        let my_pid = std::process::id();
        write_pid_file(dir.path(), "beta", "api", my_pid);
        let mut session = make_session("beta", 3);
        session.port_overrides.insert("api".into(), 3003);
        let result = resolve(dir.path(), &[session], my_pid).unwrap();
        assert_eq!(result.port, Some(3003));
    }

    #[test]
    fn resolve_skips_session_with_no_pid_dir() {
        let dir = TempDir::new().unwrap();
        // No pid file written.
        let session = make_session("gamma", 1);
        let result = resolve(dir.path(), &[session], 12345);
        assert!(result.is_none());
    }

    #[test]
    fn resolve_returns_first_matching_session() {
        let dir = TempDir::new().unwrap();
        let my_pid = std::process::id();
        write_pid_file(dir.path(), "alpha", "api", my_pid);
        write_pid_file(dir.path(), "beta", "api", my_pid);
        let sessions = vec![make_session("alpha", 1), make_session("beta", 2)];
        let result = resolve(dir.path(), &sessions, my_pid).unwrap();
        assert_eq!(result.slug, "alpha");
    }

    #[test]
    fn is_descendant_false_for_unrelated_pids() {
        assert!(!is_descendant(1, 2));
        assert!(!is_descendant(99999, 99998));
    }

    #[test]
    fn is_descendant_false_for_self() {
        let my_pid = std::process::id();
        assert!(!is_descendant(my_pid, my_pid));
    }
}
