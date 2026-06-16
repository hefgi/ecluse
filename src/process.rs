use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::ServiceConfig;

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum ProcessManager {
    Tmux,
    Nohup,
    #[default]
    None,
}

impl std::fmt::Display for ProcessManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProcessManager::Tmux => write!(f, "tmux"),
            ProcessManager::Nohup => write!(f, "nohup"),
            ProcessManager::None => write!(f, "none"),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct GlobalConfig {
    #[serde(default)]
    pub process_manager: ProcessManager,
}

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct SpawnResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tmux_session: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pid_files: Vec<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_dir: Option<PathBuf>,
}

pub fn binary_available(name: &str) -> bool {
    Command::new("which")
        .arg(name)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn detect_process_manager() -> ProcessManager {
    if binary_available("tmux") {
        ProcessManager::Tmux
    } else {
        ProcessManager::Nohup
    }
}

pub fn global_config_path() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .map(|h| PathBuf::from(h).join(".config/ecluse/config.toml"))
}

pub fn load_global_config() -> Result<GlobalConfig> {
    let path = match global_config_path() {
        Some(p) => p,
        None => return Ok(GlobalConfig::default()),
    };
    if !path.exists() {
        return Ok(GlobalConfig::default());
    }
    let content = std::fs::read_to_string(&path)?;
    let cfg: GlobalConfig = toml::from_str(&content)?;
    Ok(cfg)
}

pub fn save_global_config(cfg: &GlobalConfig) -> Result<()> {
    let path = global_config_path()
        .ok_or_else(|| anyhow::anyhow!("could not determine home directory"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = toml::to_string_pretty(cfg)?;
    std::fs::write(&path, content)?;
    Ok(())
}

pub fn spawn_services(
    manager: &ProcessManager,
    slug: &str,
    services: &[&ServiceConfig],
    worktree: &Path,
    env: &std::collections::HashMap<String, String>,
) -> Result<SpawnResult> {
    let with_commands: Vec<&&ServiceConfig> =
        services.iter().filter(|s| s.command.is_some()).collect();

    if with_commands.is_empty() || matches!(manager, ProcessManager::None) {
        return Ok(SpawnResult::default());
    }

    match manager {
        ProcessManager::Tmux => spawn_tmux(slug, &with_commands, worktree, env),
        ProcessManager::Nohup => spawn_nohup(slug, &with_commands, worktree, env),
        ProcessManager::None => Ok(SpawnResult::default()),
    }
}

pub fn kill_services(manager: &ProcessManager, result: &SpawnResult) {
    match manager {
        ProcessManager::Tmux => kill_tmux(result),
        ProcessManager::Nohup => kill_nohup(result),
        ProcessManager::None => {}
    }
}

/// Check whether spawned services are still alive via their pid files
/// (written for both nohup- and tmux-managed sessions).
/// Returns warning strings for any that have died.
pub fn check_processes_alive(
    manager: &Option<ProcessManager>,
    result: &SpawnResult,
    slug: &str,
) -> Vec<String> {
    if !matches!(
        manager,
        Some(ProcessManager::Nohup) | Some(ProcessManager::Tmux)
    ) {
        return vec![];
    }
    let mut warnings = vec![];
    for pid_file in &result.pid_files {
        let service = pid_file
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown");
        let inspect_hint = match &result.tmux_session {
            Some(session) => format!("inspect with `tmux attach -t {}`", session),
            None => {
                let log = result
                    .log_dir
                    .as_ref()
                    .map(|d| d.join(format!("{}.log", service)).display().to_string())
                    .unwrap_or_else(|| format!(".ecluse/logs/{}/{}.log", slug, service));
                format!("check {}", log)
            }
        };
        match read_pid_file(pid_file) {
            None => warnings.push(format!(
                "service '{}' has no pid file (likely killed); run `ecluse up` to restart it",
                service
            )),
            Some((pid, token)) => {
                if !pid_file_alive(pid, &token) {
                    warnings.push(format!(
                        "service '{}' (PID {}) is not running — {}",
                        service, pid, inspect_hint
                    ));
                }
            }
        }
    }
    warnings
}

pub fn pid_alive(pid: u32) -> bool {
    // kill -0 sends no signal but checks if the process exists
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Opaque start-time token for a PID (`ps -o lstart=`). Two processes that
/// recycle the same PID get different tokens, so a stored (pid, token) pair
/// identifies exactly one process incarnation.
pub fn pid_start_token(pid: u32) -> Option<String> {
    let out = Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "lstart="])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// Write `<pid>` plus its start token. Readers treat a mismatched token as a
/// stale file (the PID was recycled by an unrelated process since the write).
pub fn write_pid_file_with_token(path: &Path, pid: u32) -> std::io::Result<()> {
    let token = pid_start_token(pid).unwrap_or_default();
    std::fs::write(path, format!("{pid}\n{token}\n"))
}

/// Parse a pid file: first line pid, optional second line start token
/// (absent in files written by older versions).
pub fn read_pid_file(path: &Path) -> Option<(u32, Option<String>)> {
    let content = std::fs::read_to_string(path).ok()?;
    let mut lines = content.lines();
    let pid = lines.next()?.trim().parse().ok()?;
    let token = lines
        .next()
        .map(|l| l.trim().to_string())
        .filter(|t| !t.is_empty());
    Some((pid, token))
}

/// True when the PID exists AND — if a token was recorded — it is still the
/// same process incarnation the pid file was written for.
pub fn pid_file_alive(pid: u32, token: &Option<String>) -> bool {
    if !pid_alive(pid) {
        return false;
    }
    match token {
        None => true, // legacy pid file without token
        Some(t) => pid_start_token(pid).as_deref() == Some(t.as_str()),
    }
}

fn shell_escape(s: &str) -> String {
    // Wrap in single quotes, escaping any single quotes in the value
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn tmux_session_name(slug: &str) -> String {
    format!("ecluse-{}", slug)
}

fn tmux_session_exists(session: &str) -> bool {
    Command::new("tmux")
        .args(["has-session", "-t", session])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Parse a .env-style file into key=value pairs.
/// Delegates to the shared parser so spawning, `shell`, `env`, and
/// `up --json` all read env files identically.
fn parse_env_file(path: &Path) -> std::collections::HashMap<String, String> {
    crate::env::parse_env_file(path).into_iter().collect()
}

/// Merge .env → .env.local → .env.ecluse from `worktree` into `base`,
/// with later files winning on overlap. Returns the merged map.
/// Files that don't exist are silently skipped.
pub fn merge_worktree_env(
    worktree: &Path,
    base: &std::collections::HashMap<String, String>,
) -> std::collections::HashMap<String, String> {
    let mut merged = base.clone();
    for file in [".env", ".env.local", ".env.ecluse"] {
        let path = worktree.join(file);
        if path.exists() {
            merged.extend(parse_env_file(&path));
        }
    }
    merged
}

/// Build a shell one-liner that sources env files present in `worktree`,
/// in order: .env → .env.local → .env.ecluse (ecluse vars win on overlap).
/// Files that don't exist are silently skipped.
/// Sent as a separate command before the service command so that
/// manual restarts inside the tmux window (`↑ Enter`) also have the env.
///
/// Paths are emitted with an explicit `./` prefix because zsh's POSIX `.`
/// builtin does NOT search the current directory unless `.` is in `$PATH` —
/// `. .env` silently fails as "no such file or directory" under zsh
/// (the default macOS shell) while working in bash. `./` works in both.
fn build_source_preamble(worktree: &Path) -> String {
    let files = [".env", ".env.local", ".env.ecluse"];
    files
        .iter()
        .filter(|f| worktree.join(f).exists())
        .map(|f| format!("set -a; . {}; set +a", shell_escape(&format!("./{}", f))))
        .collect::<Vec<_>>()
        .join("; ")
}

/// Root `.ecluse` directory for a worktree: nearest ancestor containing one,
/// falling back to the worktree itself (externally-registered worktrees).
fn ecluse_dir_for(worktree: &Path) -> PathBuf {
    worktree
        .ancestors()
        .find(|p| p.join(".ecluse").exists())
        .unwrap_or(worktree)
        .join(".ecluse")
}

/// Path of the per-session env preamble sourced by tmux windows.
/// Namespaced by slug — a shared file would be overwritten by the next
/// session's spawn and leak its ports into manual restarts here.
pub fn env_preamble_path(worktree: &Path, slug: &str) -> PathBuf {
    ecluse_dir_for(worktree)
        .join("preambles")
        .join(format!("{}.sh", slug))
}

/// Best-effort removal of a session's env preamble at teardown.
pub fn remove_env_preamble(worktree: &Path, slug: &str) {
    let _ = std::fs::remove_file(env_preamble_path(worktree, slug));
}

fn write_env_preamble_file(
    worktree: &Path,
    slug: &str,
    env: &std::collections::HashMap<String, String>,
) -> Option<std::path::PathBuf> {
    // Write merged env as a sourceable file so tmux windows can source it
    // without sending a multi-KB export string through send-keys (which corrupts for
    // large envs due to terminal line-length limits and key-event reordering).
    let preamble_path = env_preamble_path(worktree, slug);
    if let Some(parent) = preamble_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut lines: Vec<String> = env
        .iter()
        .map(|(k, v)| format!("export {}={}", k, shell_escape(v)))
        .collect();
    lines.sort();
    lines.push(String::new());
    let content = lines.join("\n");
    std::fs::write(&preamble_path, content).ok()?;
    Some(preamble_path)
}

fn spawn_tmux(
    slug: &str,
    services: &[&&ServiceConfig],
    worktree: &Path,
    env: &std::collections::HashMap<String, String>,
) -> Result<SpawnResult> {
    let session = tmux_session_name(slug);
    let merged_env = merge_worktree_env(worktree, env);

    // Window 0 stays a plain shell (handy for `ecluse shell` attaches); each
    // service runs as its own window's process — no send-keys typing, so a
    // command that fails to start is a detectable pane death, not a silently
    // ignored keystroke.
    let preamble_path = write_env_preamble_file(worktree, slug, &merged_env);

    let mut source_parts: Vec<String> = vec![format!(
        "cd {}",
        shell_escape(&worktree.display().to_string())
    )];
    if let Some(ref p) = preamble_path {
        source_parts.push(format!(
            "set -a; . {}; set +a",
            shell_escape(&p.display().to_string())
        ));
    }
    let worktree_files = build_source_preamble(worktree);
    if !worktree_files.is_empty() {
        source_parts.push(worktree_files);
    }
    let setup_cmd = source_parts.join("; ");

    // Kill any stale tmux session with this name (processes exited but shell remains).
    if tmux_session_exists(&session) {
        Command::new("tmux")
            .args(["kill-session", "-t", &session])
            .output()
            .ok();
    }

    let status = Command::new("tmux")
        .args([
            "new-session",
            "-d",
            "-s",
            &session,
            "-n",
            "shell",
            "-x",
            "220",
            "-y",
            "50",
        ])
        .status()
        .map_err(|e| crate::error::EcluseError::SpawnFailed {
            service: "tmux".into(),
            reason: e.to_string(),
        })?;
    if !status.success() {
        return Err(crate::error::EcluseError::SpawnFailed {
            service: "tmux".into(),
            reason: "tmux new-session failed".into(),
        }
        .into());
    }

    // Keep dead panes around so a crashed service's output stays inspectable
    // (and so the death is observable below instead of closing the window).
    // A session-scoped hook applies the window option at creation time —
    // setting it after new-window would race an instantly-dying command.
    Command::new("tmux")
        .args([
            "set-hook",
            "-t",
            &session,
            "after-new-window",
            "set-option -w remain-on-exit on",
        ])
        .output()
        .ok();
    // The shell window should have the session env + worktree cwd too.
    Command::new("tmux")
        .args([
            "send-keys",
            "-t",
            &format!("{}:shell", session),
            &setup_cmd,
            "Enter",
        ])
        .output()
        .ok();

    let ecluse_dir = ecluse_dir_for(worktree);
    let pid_dir = ecluse_dir.join("pids").join(slug);
    std::fs::create_dir_all(&pid_dir)?;

    let cleanup = |pid_files: &[PathBuf]| {
        let _ = Command::new("tmux")
            .args(["kill-session", "-t", &session])
            .output();
        for pf in pid_files {
            let _ = std::fs::remove_file(pf);
        }
    };

    let mut pid_files: Vec<PathBuf> = vec![];
    for svc in services {
        let cmd = svc.command.as_deref().unwrap();
        let full_cmd = format!("{}; exec sh -c {}", setup_cmd, shell_escape(cmd));
        let created = Command::new("tmux")
            .args(["new-window", "-t", &session, "-n", &svc.name, &full_cmd])
            .status()
            .map(|st| st.success())
            .unwrap_or(false);
        if !created {
            cleanup(&pid_files);
            return Err(crate::error::EcluseError::SpawnFailed {
                service: svc.name.clone(),
                reason: "tmux new-window failed".into(),
            }
            .into());
        }

        if let Some(pane_pid) = tmux_pane_pid_for(&session, &svc.name) {
            let pid_path = pid_dir.join(format!("{}.pid", svc.name));
            write_pid_file_with_token(&pid_path, pane_pid)?;
            pid_files.push(pid_path);
        }
    }

    // Catch instant failures: a service command that dies within the grace
    // window (typo, missing binary, port conflict) fails the spawn instead of
    // reporting a "ready" session with dead services.
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(1500);
    loop {
        if let Some((window, status)) = first_dead_pane(&session) {
            let output = tmux_pane_tail(&session, &window, 5);
            cleanup(&pid_files);
            return Err(crate::error::EcluseError::SpawnFailed {
                service: window,
                reason: format!(
                    "command exited immediately (status {}); last output:\n{}",
                    status,
                    output.trim_end()
                ),
            }
            .into());
        }
        if std::time::Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
    }

    Ok(SpawnResult {
        tmux_session: Some(session),
        pid_files,
        log_dir: None,
    })
}

/// Pane PID of the named window in `session`.
fn tmux_pane_pid_for(session: &str, window: &str) -> Option<u32> {
    let out = Command::new("tmux")
        .args([
            "list-panes",
            "-t",
            &format!("{}:{}", session, window),
            "-F",
            "#{pane_pid}",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()?
        .trim()
        .parse()
        .ok()
}

/// First service pane that has died, as (window_name, exit_status).
/// The plain "shell" window is exempt.
fn first_dead_pane(session: &str) -> Option<(String, String)> {
    let out = Command::new("tmux")
        .args([
            "list-panes",
            "-s",
            "-t",
            session,
            "-F",
            "#{window_name}|#{pane_dead}|#{pane_dead_status}",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    for line in stdout.lines() {
        let mut parts = line.split('|');
        let window = parts.next()?.to_string();
        let dead = parts.next()? == "1";
        let status = parts.next().unwrap_or("").to_string();
        if dead && window != "shell" {
            return Some((window, status));
        }
    }
    None
}

/// Last `n` lines of a window's pane output (best effort).
fn tmux_pane_tail(session: &str, window: &str, n: usize) -> String {
    let out = Command::new("tmux")
        .args([
            "capture-pane",
            "-p",
            "-t",
            &format!("{}:{}", session, window),
        ])
        .output();
    match out {
        Ok(o) if o.status.success() => {
            let text = String::from_utf8_lossy(&o.stdout);
            let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
            lines
                .iter()
                .rev()
                .take(n)
                .rev()
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        }
        _ => String::new(),
    }
}

fn kill_tmux(result: &SpawnResult) {
    if let Some(session) = &result.tmux_session {
        // Bare `tmux kill-session` only delivers SIGHUP to each pane's foreground
        // process. Wrapper chains like sh → pnpm → node → vite reparent and survive,
        // holding ports across `ecluse down`. Group-kill every pane PID first (same
        // TERM→KILL grace as the nohup path), then kill the now-empty session.
        for pid in tmux_session_pane_pids(session) {
            kill_process_group(pid);
        }
        Command::new("tmux")
            .args(["kill-session", "-t", session])
            .output()
            .ok();
    }
    for pid_file in &result.pid_files {
        let _ = std::fs::remove_file(pid_file);
    }
}

/// All pane PIDs across all windows of `session`. Empty on any tmux failure.
fn tmux_session_pane_pids(session: &str) -> Vec<u32> {
    let Ok(out) = Command::new("tmux")
        .args(["list-panes", "-s", "-t", session, "-F", "#{pane_pid}"])
        .output()
    else {
        return vec![];
    };
    if !out.status.success() {
        return vec![];
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.trim().parse::<u32>().ok())
        .collect()
}

fn spawn_nohup(
    slug: &str,
    services: &[&&ServiceConfig],
    worktree: &Path,
    env: &std::collections::HashMap<String, String>,
) -> Result<SpawnResult> {
    let ecluse_dir = ecluse_dir_for(worktree);
    let log_dir = ecluse_dir.join("logs").join(slug);
    let pid_dir = ecluse_dir.join("pids").join(slug);

    std::fs::create_dir_all(&log_dir)?;
    std::fs::create_dir_all(&pid_dir)?;

    let merged_env = merge_worktree_env(worktree, env);
    let mut pid_files: Vec<PathBuf> = vec![];

    for svc in services {
        match spawn_one_nohup(svc, worktree, &merged_env, &log_dir, &pid_dir) {
            Ok(pid_path) => pid_files.push(pid_path),
            Err(e) => {
                // A partial spawn must not leave orphans: kill what already started.
                kill_nohup(&SpawnResult {
                    tmux_session: None,
                    pid_files,
                    log_dir: Some(log_dir.clone()),
                });
                return Err(e);
            }
        }
    }

    Ok(SpawnResult {
        tmux_session: None,
        pid_files,
        log_dir: Some(log_dir),
    })
}

fn spawn_one_nohup(
    svc: &ServiceConfig,
    worktree: &Path,
    env: &std::collections::HashMap<String, String>,
    log_dir: &Path,
    pid_dir: &Path,
) -> Result<PathBuf> {
    use std::fs::File;
    use std::os::unix::process::CommandExt;

    let cmd = svc.command.as_deref().unwrap();
    let log_path = log_dir.join(format!("{}.log", svc.name));
    let pid_path = pid_dir.join(format!("{}.pid", svc.name));

    let log_file = File::create(&log_path).map_err(|e| crate::error::EcluseError::SpawnFailed {
        service: svc.name.clone(),
        reason: format!("could not create log file: {}", e),
    })?;
    let log_file2 = log_file.try_clone()?;

    let child = Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .current_dir(worktree)
        .envs(env)
        .stdout(log_file)
        .stderr(log_file2)
        .process_group(0)
        .spawn()
        .map_err(|e| crate::error::EcluseError::SpawnFailed {
            service: svc.name.clone(),
            reason: e.to_string(),
        })?;

    write_pid_file_with_token(&pid_path, child.id())?;
    Ok(pid_path)
}

fn kill_nohup(result: &SpawnResult) {
    for pid_file in &result.pid_files {
        if let Some((pid, token)) = read_pid_file(pid_file) {
            let leader_alive = pid_alive(pid);
            if leader_alive && !pid_file_alive(pid, &token) {
                // The PID was recycled by an unrelated process — never signal
                // it (or its group, which now belongs to that process).
                tracing::warn!(
                    "PID {} from {} was recycled by another process; not signaling it",
                    pid,
                    pid_file.display()
                );
            } else if leader_alive || target_alive(&format!("-{}", pid)) {
                // Leader verified, or leader gone but its group still has our
                // orphaned children (a pgid stays allocated while any member
                // lives, so it cannot have been recycled).
                kill_process_group(pid);
            }
        }
        // Remove PID file regardless of kill success
        let _ = std::fs::remove_file(pid_file);
    }
}

/// TERM an entire process group, escalating to KILL if anything survives the
/// grace period. spawn_nohup runs each service in its own group
/// (process_group(0), pgid == leader pid); signaling only the leader would
/// orphan the service's children — the `sh -c` wrapper dies while the actual
/// server keeps running and holds the port.
fn kill_process_group(pgid: u32) {
    signal_with_grace(&format!("-{}", pgid));
}

/// TERM a single process, escalating to KILL after the grace period.
pub fn kill_pid_with_grace(pid: u32) {
    signal_with_grace(&pid.to_string());
}

/// SIGTERM `target` (a pid, or "-pgid" for a whole group), poll for it to
/// disappear, and SIGKILL whatever survives the 2s grace period.
fn signal_with_grace(target: &str) {
    let _ = Command::new("kill").args(["-TERM", "--", target]).output();

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while target_alive(target) {
        if std::time::Instant::now() >= deadline {
            let _ = Command::new("kill").args(["-KILL", "--", target]).output();
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// True while the target (pid or process group) still exists (kill -0).
fn target_alive(target: &str) -> bool {
    Command::new("kill")
        .args(["-0", "--", target])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn binary_available_returns_true_for_sh() {
        assert!(binary_available("sh"));
    }

    #[test]
    fn binary_available_returns_false_for_nonexistent() {
        assert!(!binary_available("ecluse-nonexistent-xyz"));
    }

    #[test]
    fn detect_process_manager_returns_valid_variant() {
        let pm = detect_process_manager();
        assert!(matches!(pm, ProcessManager::Tmux | ProcessManager::Nohup));
    }

    #[test]
    fn load_global_config_returns_default_when_missing() {
        // Read from a path that doesn't exist to verify default is returned.
        // We can't mutate HOME safely in multithreaded tests — use the internal
        // path-based loader directly.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".config/ecluse/config.toml");
        assert!(!path.exists());
        // Simulate what load_global_config does when the file is absent:
        let cfg: GlobalConfig = toml::from_str("").unwrap();
        assert_eq!(cfg.process_manager, ProcessManager::None);
    }

    #[test]
    fn save_and_load_global_config_roundtrips() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".config/ecluse/config.toml");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let original = GlobalConfig {
            process_manager: ProcessManager::Tmux,
        };
        let content = toml::to_string_pretty(&original).unwrap();
        std::fs::write(&path, &content).unwrap();
        let loaded: GlobalConfig =
            toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(loaded.process_manager, ProcessManager::Tmux);
    }

    #[test]
    fn spawn_result_default_is_empty() {
        let r = SpawnResult::default();
        assert!(r.tmux_session.is_none());
        assert!(r.pid_files.is_empty());
        assert!(r.log_dir.is_none());
    }

    #[test]
    fn spawn_services_none_manager_returns_empty_result() {
        let dir = TempDir::new().unwrap();
        let svc = crate::config::ServiceConfig {
            name: "api".into(),
            base_port: 3000,
            run: crate::config::ServiceRun::Native,
            compose: None,
            command: Some("echo hello".into()),
            port_env: vec![],
            debug_port: None,
            extra_ports: vec![],
            publish_primary: None,
            host_port: None,
        };
        let result = spawn_services(
            &ProcessManager::None,
            "test-slug",
            &[&svc],
            dir.path(),
            &std::collections::HashMap::new(),
        )
        .unwrap();
        assert!(result.tmux_session.is_none());
        assert!(result.pid_files.is_empty());
    }

    #[test]
    fn kill_services_none_manager_is_noop() {
        let result = SpawnResult::default();
        kill_services(&ProcessManager::None, &result);
    }

    #[test]
    fn check_processes_alive_empty_when_no_pid_files() {
        let result = SpawnResult::default();
        let warnings = check_processes_alive(&Some(ProcessManager::Nohup), &result, "slug");
        assert!(warnings.is_empty());
    }

    #[test]
    fn check_processes_alive_warns_for_dead_pid() {
        let dir = TempDir::new().unwrap();
        let pid_file = dir.path().join("api.pid");
        // PID 99999999 is almost certainly not running
        std::fs::write(&pid_file, "99999999").unwrap();
        let result = SpawnResult {
            tmux_session: None,
            pid_files: vec![pid_file],
            log_dir: Some(dir.path().to_owned()),
        };
        let warnings = check_processes_alive(&Some(ProcessManager::Nohup), &result, "slug");
        assert!(!warnings.is_empty());
        assert!(warnings[0].contains("api"));
    }

    #[test]
    fn check_processes_alive_tmux_returns_empty() {
        let result = SpawnResult {
            tmux_session: Some("ecluse-foo".into()),
            pid_files: vec![],
            log_dir: None,
        };
        let warnings = check_processes_alive(&Some(ProcessManager::Tmux), &result, "slug");
        assert!(warnings.is_empty());
    }

    #[test]
    fn spawn_nohup_creates_log_and_pid_files() {
        if !binary_available("nohup") && !binary_available("sh") {
            return;
        }
        let dir = TempDir::new().unwrap();
        // Create a .ecluse dir so the path resolution works
        std::fs::create_dir_all(dir.path().join(".ecluse")).unwrap();
        let svc = crate::config::ServiceConfig {
            name: "api".into(),
            base_port: 3000,
            run: crate::config::ServiceRun::Native,
            compose: None,
            command: Some("sleep 60".into()),
            port_env: vec![],
            debug_port: None,
            extra_ports: vec![],
            publish_primary: None,
            host_port: None,
        };
        let result = spawn_services(
            &ProcessManager::Nohup,
            "test-slug",
            &[&svc],
            dir.path(),
            &std::collections::HashMap::new(),
        )
        .unwrap();
        assert_eq!(result.pid_files.len(), 1);
        assert!(result.pid_files[0].exists());
        // Clean up the spawned process
        kill_services(&ProcessManager::Nohup, &result);
    }

    #[test]
    fn parse_env_file_reads_key_value_pairs() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".env");
        std::fs::write(&path, "FOO=bar\nBAZ=qux\n").unwrap();
        let map = parse_env_file(&path);
        assert_eq!(map.get("FOO").map(String::as_str), Some("bar"));
        assert_eq!(map.get("BAZ").map(String::as_str), Some("qux"));
    }

    #[test]
    fn parse_env_file_skips_comments_and_blanks() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".env");
        std::fs::write(&path, "# comment\n\nFOO=bar\n").unwrap();
        let map = parse_env_file(&path);
        assert_eq!(map.len(), 1);
        assert_eq!(map.get("FOO").map(String::as_str), Some("bar"));
    }

    #[test]
    fn parse_env_file_returns_empty_for_missing_file() {
        let map = parse_env_file(std::path::Path::new("/nonexistent/.env"));
        assert!(map.is_empty());
    }

    #[test]
    fn merge_worktree_env_ecluse_wins_on_overlap() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join(".env"), "PORT=3000\nSHARED=from-env\n").unwrap();
        std::fs::write(dir.path().join(".env.ecluse"), "PORT=3001\nECLUSE_SLOT=2\n").unwrap();
        let base = std::collections::HashMap::new();
        let merged = merge_worktree_env(dir.path(), &base);
        // .env.ecluse wins over .env for PORT
        assert_eq!(merged.get("PORT").map(String::as_str), Some("3001"));
        assert_eq!(merged.get("ECLUSE_SLOT").map(String::as_str), Some("2"));
        assert_eq!(merged.get("SHARED").map(String::as_str), Some("from-env"));
    }

    #[test]
    fn merge_worktree_env_base_preserved_when_no_files() {
        let dir = TempDir::new().unwrap();
        let mut base = std::collections::HashMap::new();
        base.insert("EXISTING".to_string(), "value".to_string());
        let merged = merge_worktree_env(dir.path(), &base);
        assert_eq!(merged.get("EXISTING").map(String::as_str), Some("value"));
    }

    #[test]
    fn merge_worktree_env_env_local_overrides_env() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join(".env"), "KEY=from-env\n").unwrap();
        std::fs::write(dir.path().join(".env.local"), "KEY=from-local\n").unwrap();
        let base = std::collections::HashMap::new();
        let merged = merge_worktree_env(dir.path(), &base);
        assert_eq!(merged.get("KEY").map(String::as_str), Some("from-local"));
    }

    fn native_svc(name: &str, command: &str) -> crate::config::ServiceConfig {
        crate::config::ServiceConfig {
            name: name.into(),
            base_port: 3000,
            run: crate::config::ServiceRun::Native,
            compose: None,
            command: Some(command.into()),
            port_env: vec![],
            debug_port: None,
            extra_ports: vec![],
            publish_primary: None,
            host_port: None,
        }
    }

    fn wait_until(timeout: std::time::Duration, mut cond: impl FnMut() -> bool) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        while std::time::Instant::now() < deadline {
            if cond() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        cond()
    }

    // ── pid start tokens ──────────────────────────────────────────────────────

    #[test]
    fn pid_token_roundtrip_for_live_process() {
        let dir = TempDir::new().unwrap();
        let my_pid = std::process::id();
        let path = dir.path().join("self.pid");
        write_pid_file_with_token(&path, my_pid).unwrap();
        let (pid, token) = read_pid_file(&path).unwrap();
        assert_eq!(pid, my_pid);
        assert!(token.is_some(), "live process must get a start token");
        assert!(pid_file_alive(pid, &token));
    }

    #[test]
    fn pid_file_alive_rejects_recycled_pid() {
        // Same PID, forged token from "another incarnation".
        let my_pid = std::process::id();
        let forged = Some("Wed Jan  1 00:00:00 1986".to_string());
        assert!(!pid_file_alive(my_pid, &forged));
    }

    #[test]
    fn pid_file_alive_accepts_legacy_tokenless_files() {
        let my_pid = std::process::id();
        assert!(pid_file_alive(my_pid, &None));
    }

    #[test]
    fn read_pid_file_parses_legacy_single_line_format() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("legacy.pid");
        std::fs::write(&path, "4242").unwrap();
        let (pid, token) = read_pid_file(&path).unwrap();
        assert_eq!(pid, 4242);
        assert!(token.is_none());
    }

    // The service command spawns a child; killing the session must take the
    // whole process group down, not just the `sh -c` group leader.
    #[test]
    fn kill_nohup_kills_whole_process_group() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".ecluse")).unwrap();
        let child_pid_file = dir.path().join("child.pid");
        let svc = native_svc(
            "bg",
            &format!("sleep 30 & echo $! > {}; wait", child_pid_file.display()),
        );
        let result = spawn_services(
            &ProcessManager::Nohup,
            "pg-test",
            &[&svc],
            dir.path(),
            &std::collections::HashMap::new(),
        )
        .unwrap();

        assert!(
            wait_until(std::time::Duration::from_secs(5), || child_pid_file
                .exists()),
            "child pid file never appeared"
        );
        let child_pid: u32 = std::fs::read_to_string(&child_pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!(pid_alive(child_pid), "background child should be running");

        kill_services(&ProcessManager::Nohup, &result);

        assert!(
            wait_until(std::time::Duration::from_secs(5), || !pid_alive(child_pid)),
            "background child must die with the process group"
        );
    }

    // Preamble files are per-slug; parallel sessions must never share one.
    #[test]
    fn env_preamble_file_is_namespaced_per_slug() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".ecluse")).unwrap();

        let mut env_a = std::collections::HashMap::new();
        env_a.insert("PORT".to_string(), "3001".to_string());
        let path_a = write_env_preamble_file(dir.path(), "sess-a", &env_a).unwrap();

        let mut env_b = std::collections::HashMap::new();
        env_b.insert("PORT".to_string(), "3002".to_string());
        let path_b = write_env_preamble_file(dir.path(), "sess-b", &env_b).unwrap();

        assert_ne!(path_a, path_b);
        assert!(std::fs::read_to_string(&path_a).unwrap().contains("3001"));
        assert!(std::fs::read_to_string(&path_b).unwrap().contains("3002"));
    }

    #[test]
    fn remove_env_preamble_deletes_only_that_slug() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".ecluse")).unwrap();
        let mut env = std::collections::HashMap::new();
        env.insert("PORT".to_string(), "3001".to_string());
        let path_a = write_env_preamble_file(dir.path(), "sess-a", &env).unwrap();
        let path_b = write_env_preamble_file(dir.path(), "sess-b", &env).unwrap();

        remove_env_preamble(dir.path(), "sess-a");
        assert!(!path_a.exists());
        assert!(path_b.exists());
    }

    // If service N fails to spawn, services 1..N-1 must be killed, not orphaned.
    #[test]
    fn spawn_nohup_partial_failure_cleans_up_already_spawned() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".ecluse")).unwrap();
        // Make the second service's log file uncreatable: a directory in its place.
        std::fs::create_dir_all(dir.path().join(".ecluse/logs/part/two.log")).unwrap();

        let one = native_svc("one", "sleep 30");
        let two = native_svc("two", "sleep 30");
        let err = spawn_services(
            &ProcessManager::Nohup,
            "part",
            &[&one, &two],
            dir.path(),
            &std::collections::HashMap::new(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("two"), "got: {}", err);

        // kill_nohup removes pid files after killing — service one must be cleaned up.
        let one_pid = dir.path().join(".ecluse/pids/part/one.pid");
        assert!(
            !one_pid.exists(),
            "service one's pid file should be removed by partial-spawn cleanup"
        );
    }

    // ── build_source_preamble ─────────────────────────────────────────────────

    #[test]
    fn build_source_preamble_prefixes_relative_paths_with_dot_slash() {
        // zsh's POSIX `.` builtin does NOT search cwd unless `.` is in $PATH.
        // The emitted command must use `./<name>` so it works in both bash and zsh.
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join(".env"), "FOO=bar").unwrap();
        std::fs::write(dir.path().join(".env.local"), "BAR=baz").unwrap();
        let cmd = build_source_preamble(dir.path());
        assert!(
            cmd.contains(". './.env'"),
            "expected `. './.env'` in: {}",
            cmd
        );
        assert!(
            cmd.contains(". './.env.local'"),
            "expected `. './.env.local'` in: {}",
            cmd
        );
        // Never emit the bare `. '.env'` form — that's the regression we're fixing.
        assert!(
            !cmd.contains(". '.env'"),
            "must not emit zsh-incompatible `. '.env'`: {}",
            cmd
        );
    }

    #[test]
    fn build_source_preamble_skips_missing_files() {
        let dir = TempDir::new().unwrap();
        // Only .env present.
        std::fs::write(dir.path().join(".env"), "FOO=bar").unwrap();
        let cmd = build_source_preamble(dir.path());
        assert!(cmd.contains(". './.env'"));
        assert!(!cmd.contains(".env.local"));
        assert!(!cmd.contains(".env.ecluse"));
    }

    #[test]
    fn build_source_preamble_empty_when_no_files() {
        let dir = TempDir::new().unwrap();
        assert_eq!(build_source_preamble(dir.path()), "");
    }
}

#[cfg(test)]
mod tmux_tests {
    use super::*;
    use tempfile::TempDir;

    fn tmux_available() -> bool {
        binary_available("tmux")
    }

    fn native_svc(name: &str, command: &str) -> crate::config::ServiceConfig {
        crate::config::ServiceConfig {
            name: name.into(),
            base_port: 3000,
            run: crate::config::ServiceRun::Native,
            compose: None,
            command: Some(command.into()),
            port_env: vec![],
            debug_port: None,
            extra_ports: vec![],
            publish_primary: None,
            host_port: None,
        }
    }

    // A command that dies instantly must fail the spawn (and clean up the
    // session) instead of reporting a "ready" session with dead services.
    #[test]
    fn spawn_tmux_detects_instantly_failing_command() {
        if !tmux_available() {
            return;
        }
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".ecluse")).unwrap();
        let svc = native_svc("broken", "definitely-not-a-binary-xyz");
        let err = spawn_services(
            &ProcessManager::Tmux,
            "tmux-fail-test",
            &[&svc],
            dir.path(),
            &std::collections::HashMap::new(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("broken"), "got: {}", err);
        assert!(
            !tmux_session_exists("ecluse-tmux-fail-test"),
            "failed spawn must not leave the tmux session behind"
        );
    }

    // A long-running service gets a recorded, token-verified pane PID, and
    // kill_services takes the whole session (and pid files) down.
    #[test]
    fn spawn_tmux_records_pane_pids_and_kills_cleanly() {
        if !tmux_available() {
            return;
        }
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".ecluse")).unwrap();
        let svc = native_svc("sleeper", "sleep 300");
        let result = spawn_services(
            &ProcessManager::Tmux,
            "tmux-ok-test",
            &[&svc],
            dir.path(),
            &std::collections::HashMap::new(),
        )
        .unwrap();

        assert_eq!(result.tmux_session.as_deref(), Some("ecluse-tmux-ok-test"));
        assert_eq!(result.pid_files.len(), 1, "pane pid must be recorded");
        let (pid, token) = read_pid_file(&result.pid_files[0]).unwrap();
        assert!(pid_file_alive(pid, &token), "pane process must be running");

        kill_services(&ProcessManager::Tmux, &result);
        assert!(!tmux_session_exists("ecluse-tmux-ok-test"));
        assert!(
            !result.pid_files[0].exists(),
            "pid file must be removed on kill"
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while pid_alive(pid) && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(!pid_alive(pid), "pane process must die with the session");
    }

    fn wait_until(timeout: std::time::Duration, mut cond: impl FnMut() -> bool) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        while std::time::Instant::now() < deadline {
            if cond() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        cond()
    }

    // The pane command spawns a background child (the common pnpm → node → vite
    // case); kill_services must take the whole process group, not just the pane.
    #[test]
    fn kill_tmux_kills_whole_process_group() {
        if !tmux_available() {
            return;
        }
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".ecluse")).unwrap();
        let child_pid_file = dir.path().join("child.pid");
        let svc = native_svc(
            "bg",
            &format!("sleep 300 & echo $! > {}; wait", child_pid_file.display()),
        );
        let result = spawn_services(
            &ProcessManager::Tmux,
            "tmux-pg-test",
            &[&svc],
            dir.path(),
            &std::collections::HashMap::new(),
        )
        .unwrap();

        assert!(
            wait_until(std::time::Duration::from_secs(5), || child_pid_file
                .exists()),
            "background child pid file never appeared"
        );
        let child_pid: u32 = std::fs::read_to_string(&child_pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!(pid_alive(child_pid), "background child should be running");

        kill_services(&ProcessManager::Tmux, &result);

        assert!(
            wait_until(std::time::Duration::from_secs(5), || !pid_alive(child_pid)),
            "background child must die with the tmux pane's process group"
        );
        assert!(!tmux_session_exists("ecluse-tmux-pg-test"));
    }
}
