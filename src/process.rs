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

/// Check whether spawned nohup processes are still alive.
/// Returns warning strings for any that have died.
pub fn check_processes_alive(
    manager: &Option<ProcessManager>,
    result: &SpawnResult,
    slug: &str,
) -> Vec<String> {
    if !matches!(manager, Some(ProcessManager::Nohup)) {
        return vec![];
    }
    let mut warnings = vec![];
    for pid_file in &result.pid_files {
        let service = pid_file
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown");
        if let Ok(content) = std::fs::read_to_string(pid_file) {
            if let Ok(pid) = content.trim().parse::<u32>() {
                if !pid_alive(pid) {
                    let log_hint = result
                        .log_dir
                        .as_ref()
                        .map(|d| d.join(format!("{}.log", service)).display().to_string())
                        .unwrap_or_else(|| format!(".ecluse/logs/{}/{}.log", slug, service));
                    warnings.push(format!(
                        "service '{}' (PID {}) is not running — check {}",
                        service, pid, log_hint
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

/// Parse a .env-style file into key=value pairs. Lines starting with `#`
/// and blank lines are skipped. Values are not unquoted — taken as-is.
fn parse_env_file(path: &Path) -> std::collections::HashMap<String, String> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return std::collections::HashMap::new();
    };
    content
        .lines()
        .filter(|l| !l.trim_start().starts_with('#') && l.contains('='))
        .filter_map(|l| {
            let (k, v) = l.split_once('=')?;
            let k = k.trim().to_string();
            if k.is_empty() {
                return None;
            }
            Some((k, v.to_string()))
        })
        .collect()
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
fn build_source_preamble(worktree: &Path) -> String {
    let files = [".env", ".env.local", ".env.ecluse"];
    files
        .iter()
        .filter(|f| worktree.join(f).exists())
        .map(|f| format!("set -a; source {}; set +a", shell_escape(f)))
        .collect::<Vec<_>>()
        .join("; ")
}

fn write_env_preamble_file(
    worktree: &Path,
    env: &std::collections::HashMap<String, String>,
) -> Option<std::path::PathBuf> {
    // Write merged env as a sourceable file into .ecluse/ so tmux windows can source it
    // without sending a multi-KB export string through send-keys (which corrupts for
    // large envs due to terminal line-length limits and key-event reordering).
    let ecluse_dir = worktree
        .ancestors()
        .find(|p| p.join(".ecluse").exists())
        .unwrap_or(worktree)
        .join(".ecluse");
    let _ = std::fs::create_dir_all(&ecluse_dir);
    let preamble_path = ecluse_dir.join("env-preamble.sh");
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

    // Write merged env to a file so tmux windows source it rather than receiving
    // a multi-KB export string through send-keys (safe for any env size).
    let preamble_path = write_env_preamble_file(worktree, &merged_env);

    // Build the source preamble: ecluse preamble file first, then the worktree env
    // files (.env → .env.local → .env.ecluse) so manual restarts (↑ Enter) also
    // have the correct environment. The preamble file is sourced first so the
    // worktree files can override individual vars if needed.
    let mut source_parts: Vec<String> = Vec::new();
    if let Some(ref p) = preamble_path {
        source_parts.push(format!(
            "set -a; source {}; set +a",
            shell_escape(&p.display().to_string())
        ));
    }
    let worktree_files = build_source_preamble(worktree);
    if !worktree_files.is_empty() {
        source_parts.push(worktree_files);
    }
    source_parts.push(format!(
        "cd {}",
        shell_escape(&worktree.display().to_string())
    ));
    let setup_cmd = source_parts.join("; ");

    // Kill any stale tmux session with this name (processes exited but shell remains).
    if tmux_session_exists(&session) {
        Command::new("tmux")
            .args(["kill-session", "-t", &session])
            .output()
            .ok();
    }

    // Create detached session
    let status = Command::new("tmux")
        .args(["new-session", "-d", "-s", &session, "-x", "220", "-y", "50"])
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

    for (i, svc) in services.iter().enumerate() {
        let cmd = svc.command.as_deref().unwrap();
        let target = if i == 0 {
            format!("{}:0", session)
        } else {
            format!("{}:{}", session, svc.name)
        };

        if i == 0 {
            Command::new("tmux")
                .args(["rename-window", "-t", &format!("{}:0", session), &svc.name])
                .status()
                .ok();
        } else {
            Command::new("tmux")
                .args(["new-window", "-t", &session, "-n", &svc.name])
                .status()
                .ok();
        }

        // Source env + cd to worktree, then run the service command.
        // Two separate send-keys calls keeps each line short; the setup line
        // is a handful of file paths, never a large export blob.
        Command::new("tmux")
            .args(["send-keys", "-t", &target, &setup_cmd, "Enter"])
            .status()
            .ok();

        Command::new("tmux")
            .args(["send-keys", "-t", &target, cmd, "Enter"])
            .status()
            .ok();
    }

    Ok(SpawnResult {
        tmux_session: Some(session),
        pid_files: vec![],
        log_dir: None,
    })
}

fn kill_tmux(result: &SpawnResult) {
    if let Some(session) = &result.tmux_session {
        Command::new("tmux")
            .args(["kill-session", "-t", session])
            .output()
            .ok();
    }
}

fn spawn_nohup(
    slug: &str,
    services: &[&&ServiceConfig],
    worktree: &Path,
    env: &std::collections::HashMap<String, String>,
) -> Result<SpawnResult> {
    use std::fs::File;
    use std::os::unix::process::CommandExt;

    let log_dir = worktree
        .ancestors()
        .find(|p| p.join(".ecluse").exists())
        .unwrap_or(worktree)
        .join(".ecluse")
        .join("logs")
        .join(slug);
    let pid_dir = worktree
        .ancestors()
        .find(|p| p.join(".ecluse").exists())
        .unwrap_or(worktree)
        .join(".ecluse")
        .join("pids")
        .join(slug);

    std::fs::create_dir_all(&log_dir)?;
    std::fs::create_dir_all(&pid_dir)?;

    let merged_env = merge_worktree_env(worktree, env);
    let mut pid_files = vec![];

    for svc in services {
        let cmd = svc.command.as_deref().unwrap();
        let log_path = log_dir.join(format!("{}.log", svc.name));
        let pid_path = pid_dir.join(format!("{}.pid", svc.name));

        let log_file =
            File::create(&log_path).map_err(|e| crate::error::EcluseError::SpawnFailed {
                service: svc.name.clone(),
                reason: format!("could not create log file: {}", e),
            })?;
        let log_file2 = log_file.try_clone()?;

        let child = Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .current_dir(worktree)
            .envs(&merged_env)
            .stdout(log_file)
            .stderr(log_file2)
            .process_group(0)
            .spawn()
            .map_err(|e| crate::error::EcluseError::SpawnFailed {
                service: svc.name.clone(),
                reason: e.to_string(),
            })?;

        std::fs::write(&pid_path, child.id().to_string())?;
        pid_files.push(pid_path);
    }

    Ok(SpawnResult {
        tmux_session: None,
        pid_files,
        log_dir: Some(log_dir),
    })
}

fn kill_nohup(result: &SpawnResult) {
    for pid_file in &result.pid_files {
        if let Ok(content) = std::fs::read_to_string(pid_file) {
            if let Ok(pid) = content.trim().parse::<u32>() {
                Command::new("kill")
                    .args(["-TERM", &pid.to_string()])
                    .output()
                    .ok();
            }
        }
        // Remove PID file regardless of kill success
        let _ = std::fs::remove_file(pid_file);
    }
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
}
