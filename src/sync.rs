use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::ServiceConfig;
use crate::docker;

/// A process discovered running with its cwd inside the worktree.
#[derive(Debug, Clone)]
pub struct DiscoveredProcess {
    pub pid: u32,
    pub cmdline: String,
    pub listening_ports: Vec<u16>,
}

/// A service matched to a discovered process.
#[derive(Debug, Clone)]
pub struct ServiceMatch {
    pub service_name: String,
    pub pid: u32,
    pub port: Option<u16>,
}

/// Find all processes whose cwd is inside `worktree`.
///
/// Uses `lsof +d <path>` to list open-file records for the directory, extracts
/// unique PIDs, then resolves each PID's cmdline and listening ports.
pub fn find_processes_in_worktree(worktree: &Path) -> Vec<DiscoveredProcess> {
    let pids = pids_in_directory(worktree);
    let mut result = Vec::new();
    for pid in pids {
        let cmdline = process_cmdline(pid).unwrap_or_default();
        let listening_ports = pid_listening_ports(pid);
        result.push(DiscoveredProcess {
            pid,
            cmdline,
            listening_ports,
        });
    }
    result
}

/// Match native services to discovered processes.
///
/// For each service with a `command`, we strip generic launcher prefixes and
/// search processes whose cmdline contains the meaningful tokens. If the matched
/// root process has no listening ports, we walk its child process subtree (up to
/// 3 levels deep) to find a descendant that is listening.
pub fn match_services(
    services: &[&ServiceConfig],
    processes: &[DiscoveredProcess],
) -> Vec<ServiceMatch> {
    let mut matches = Vec::new();

    for svc in services {
        let command = match &svc.command {
            Some(c) => c,
            None => continue,
        };

        let tokens = meaningful_tokens(command);
        if tokens.is_empty() {
            continue;
        }

        let root = processes
            .iter()
            .find(|p| cmdline_matches(&p.cmdline, &tokens));

        let (pid, port) = match root {
            Some(proc) => {
                if !proc.listening_ports.is_empty() {
                    (proc.pid, Some(proc.listening_ports[0]))
                } else {
                    // Walk subtree to find a descendant with a listening port.
                    let port = find_port_in_subtree(proc.pid, 3);
                    (proc.pid, port)
                }
            }
            None => continue,
        };

        matches.push(ServiceMatch {
            service_name: svc.name.clone(),
            pid,
            port,
        });
    }

    matches
}

/// Detect running docker containers related to `slug` and match them to docker services.
///
/// Runs `docker ps` and filters containers whose name contains the slug.
/// For each docker service, returns the first host port bound to the container's
/// `base_port` (or any port the container exposes if base_port doesn't match).
/// Best-effort: returns empty if docker is unavailable or nothing matches.
pub fn find_docker_services(services: &[&ServiceConfig], slug: &str) -> Vec<(String, u16)> {
    let output = match docker::docker_cmd()
        .args(["ps", "--format", "{{.Names}}\t{{.Ports}}"])
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return vec![],
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Parse lines into (container_name, ports_string)
    let containers: Vec<(&str, &str)> = stdout
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .collect();

    let mut result = Vec::new();

    for svc in services {
        // Find a container whose name contains the slug and (optionally) the service name.
        let container = containers
            .iter()
            .find(|(name, _)| {
                name.contains(slug) && (name.contains(&svc.name) || containers.len() == 1)
            })
            .or_else(|| containers.iter().find(|(name, _)| name.contains(slug)));

        if let Some((_, ports_str)) = container {
            if let Some(port) = parse_host_port(ports_str, svc.base_port) {
                result.push((svc.name.clone(), port));
            }
        }
    }

    result
}

/// Write a PID file for a discovered process at the standard ecluse path.
///
/// Path: `<ecluse_dir>/pids/<slug>/<service>.pid`
pub fn write_pid_file(
    ecluse_dir: &Path,
    slug: &str,
    service: &str,
    pid: u32,
) -> std::io::Result<PathBuf> {
    let pid_dir = ecluse_dir.join("pids").join(slug);
    std::fs::create_dir_all(&pid_dir)?;
    let pid_path = pid_dir.join(format!("{}.pid", service));
    std::fs::write(&pid_path, pid.to_string())?;
    Ok(pid_path)
}

/// Returns true iff the named tmux window's pane shell (or any descendant process)
/// is listening on `port`. False on any error (tmux absent, session/window gone, etc.)
pub fn tmux_window_owns_port(session: &str, window: &str, port: u16) -> bool {
    match tmux_pane_pid(session, window) {
        None => false,
        Some(pane_pid) => subtree_owns_port(pane_pid, port),
    }
}

/// Returns true iff the named window exists in the tmux session.
/// Used as a fallback when no port has been allocated yet for a service.
pub(crate) fn tmux_window_exists(session: &str, window: &str) -> bool {
    tmux_pane_pid(session, window).is_some()
}

// ── internal helpers ──────────────────────────────────────────────────────────

/// Return unique PIDs of processes with an open file descriptor inside `dir`.
fn pids_in_directory(dir: &Path) -> Vec<u32> {
    let output = match Command::new("lsof")
        .arg("+d")
        .arg(dir)
        .args(["-F", "p"])
        .output()
    {
        Ok(o) => o,
        Err(_) => return vec![],
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut pids: Vec<u32> = stdout
        .lines()
        .filter_map(|line| line.strip_prefix('p').and_then(|s| s.parse().ok()))
        .collect();
    pids.sort_unstable();
    pids.dedup();
    pids
}

/// Get the full command line of a process via `ps`.
fn process_cmdline(pid: u32) -> Option<String> {
    let output = Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "args="])
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// Return all TCP ports this PID is currently listening on.
pub(crate) fn pid_listening_ports(pid: u32) -> Vec<u16> {
    let output = match Command::new("lsof")
        .args([
            "-p",
            &pid.to_string(),
            "-iTCP",
            "-sTCP:LISTEN",
            "-n",
            "-P",
            "-F",
            "n",
        ])
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return vec![],
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut ports = Vec::new();
    for line in stdout.lines() {
        // lsof -F n lines look like: n*:3000 or n127.0.0.1:3000
        if let Some(rest) = line.strip_prefix('n') {
            if let Some(port_str) = rest.rsplit(':').next() {
                if let Ok(port) = port_str.parse::<u16>() {
                    ports.push(port);
                }
            }
        }
    }
    ports.sort_unstable();
    ports.dedup();
    ports
}

/// Recursively collect listening ports from child processes, up to `depth` levels.
fn find_port_in_subtree(pid: u32, depth: u8) -> Option<u16> {
    if depth == 0 {
        return None;
    }
    for child_pid in child_pids(pid) {
        let ports = pid_listening_ports(child_pid);
        if !ports.is_empty() {
            return Some(ports[0]);
        }
        if let Some(p) = find_port_in_subtree(child_pid, depth - 1) {
            return Some(p);
        }
    }
    None
}

/// Return direct child PIDs of `pid` using `pgrep`.
pub(crate) fn child_pids(pid: u32) -> Vec<u32> {
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

/// Get the pane shell PID for a named window in a tmux session.
///
/// Returns `None` if tmux is unavailable, the session doesn't exist, or the
/// window doesn't exist (tmux exits non-zero in all these cases).
fn tmux_pane_pid(session: &str, window: &str) -> Option<u32> {
    let target = format!("{}:{}", session, window);
    let output = Command::new("tmux")
        .args(["list-panes", "-t", &target, "-F", "#{pane_pid}"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find(|l| !l.trim().is_empty())
        .and_then(|l| l.trim().parse().ok())
}

/// Check whether `pid` or any process in its descendant subtree is listening on `port`.
///
/// Uses an iterative walk (no depth cap) via `child_pids` + `pid_listening_ports`.
fn subtree_owns_port(pid: u32, port: u16) -> bool {
    let mut stack: Vec<u32> = vec![pid];
    while let Some(current) = stack.pop() {
        if pid_listening_ports(current).contains(&port) {
            return true;
        }
        stack.extend(child_pids(current));
    }
    false
}

/// Generic launcher prefixes to strip before matching.
const STRIP_PREFIXES: &[&[&str]] = &[
    &["bundle", "exec"],
    &["npx"],
    &["yarn"],
    &["sh", "-c"],
    &["bash", "-c"],
    &["python", "-m"],
    &["python3", "-m"],
];

/// Extract meaningful tokens from a command string by stripping launcher prefixes.
fn meaningful_tokens(command: &str) -> Vec<String> {
    let parts: Vec<&str> = command.split_whitespace().collect();
    if parts.is_empty() {
        return vec![];
    }

    for prefix in STRIP_PREFIXES {
        if parts.len() >= prefix.len()
            && parts[..prefix.len()]
                .iter()
                .zip(prefix.iter())
                .all(|(a, b)| a.eq_ignore_ascii_case(b))
        {
            let remainder = &parts[prefix.len()..];
            if !remainder.is_empty() {
                return remainder.iter().map(|s| s.to_string()).collect();
            }
        }
    }

    parts.iter().map(|s| s.to_string()).collect()
}

/// Return true if `cmdline` contains all `tokens` as substrings (case-insensitive).
fn cmdline_matches(cmdline: &str, tokens: &[String]) -> bool {
    let lower = cmdline.to_lowercase();
    tokens.iter().all(|t| lower.contains(&t.to_lowercase()))
}

/// Parse the first host port from a docker port mapping string.
///
/// `ports_str` looks like: `0.0.0.0:5433->5432/tcp, :::5433->5432/tcp`
/// If `base_port` matches the container port, returns its host port.
/// Otherwise falls back to the first host port found.
fn parse_host_port(ports_str: &str, base_port: u16) -> Option<u16> {
    // Parse all mappings: host_port -> container_port
    let mut mappings: Vec<(u16, u16)> = Vec::new();
    for segment in ports_str.split(',') {
        let segment = segment.trim();
        // Format: [host_addr:]host_port->container_port/proto
        if let Some((left, right)) = segment.split_once("->") {
            let host_port: u16 = left.rsplit(':').next()?.parse().ok()?;
            let container_port: u16 = right.split('/').next()?.parse().ok()?;
            mappings.push((host_port, container_port));
        }
    }

    // Prefer a mapping where container_port == base_port
    if let Some((hp, _)) = mappings.iter().find(|(_, cp)| *cp == base_port) {
        return Some(*hp);
    }
    // Fall back to first host port
    mappings.first().map(|(hp, _)| *hp)
}

// ── shared test helpers ───────────────────────────────────────────────────────

#[cfg(test)]
pub(crate) fn make_native_svc(name: &str, base_port: u16, command: &str) -> ServiceConfig {
    ServiceConfig {
        name: name.into(),
        base_port,
        run: crate::config::ServiceRun::Native,
        compose: None,
        command: Some(command.into()),
        port_env: vec![],
        debug_port: None,
        extra_ports: vec![],
        host_port: None,
    }
}

#[cfg(test)]
pub(crate) fn make_docker_svc(name: &str, base_port: u16) -> ServiceConfig {
    ServiceConfig {
        name: name.into(),
        base_port,
        run: crate::config::ServiceRun::Docker,
        compose: None,
        command: None,
        port_env: vec![],
        debug_port: None,
        extra_ports: vec![],
        host_port: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // ── meaningful_tokens ─────────────────────────────────────────────────────

    #[test]
    fn strips_bundle_exec() {
        let tokens = meaningful_tokens("bundle exec rails server");
        assert_eq!(tokens, vec!["rails", "server"]);
    }

    #[test]
    fn strips_npx() {
        let tokens = meaningful_tokens("npx next dev");
        assert_eq!(tokens, vec!["next", "dev"]);
    }

    #[test]
    fn strips_yarn() {
        let tokens = meaningful_tokens("yarn run dev");
        assert_eq!(tokens, vec!["run", "dev"]);
    }

    #[test]
    fn strips_sh_c() {
        let tokens = meaningful_tokens("sh -c npm start");
        assert_eq!(tokens, vec!["npm", "start"]);
    }

    #[test]
    fn strips_python_m() {
        let tokens = meaningful_tokens("python -m uvicorn main:app");
        assert_eq!(tokens, vec!["uvicorn", "main:app"]);
    }

    #[test]
    fn no_prefix_returns_all_tokens() {
        let tokens = meaningful_tokens("npm run dev");
        assert_eq!(tokens, vec!["npm", "run", "dev"]);
    }

    #[test]
    fn empty_command_returns_empty() {
        let tokens = meaningful_tokens("");
        assert!(tokens.is_empty());
    }

    // ── cmdline_matches ───────────────────────────────────────────────────────

    #[test]
    fn matches_when_all_tokens_present() {
        assert!(cmdline_matches(
            "/usr/bin/node /app/node_modules/.bin/next dev",
            &["next".into(), "dev".into()]
        ));
    }

    #[test]
    fn no_match_when_token_missing() {
        assert!(!cmdline_matches(
            "/usr/bin/node server.js",
            &["next".into(), "dev".into()]
        ));
    }

    #[test]
    fn match_is_case_insensitive() {
        assert!(cmdline_matches(
            "Rails Server",
            &["rails".into(), "server".into()]
        ));
    }

    // ── parse_host_port ───────────────────────────────────────────────────────

    #[test]
    fn parses_simple_port_mapping() {
        let port = parse_host_port("0.0.0.0:5433->5432/tcp", 5432);
        assert_eq!(port, Some(5433));
    }

    #[test]
    fn prefers_base_port_match() {
        let port = parse_host_port("0.0.0.0:6380->6379/tcp, 0.0.0.0:5433->5432/tcp", 5432);
        assert_eq!(port, Some(5433));
    }

    #[test]
    fn falls_back_to_first_port_when_no_base_match() {
        let port = parse_host_port("0.0.0.0:9000->8080/tcp", 3000);
        assert_eq!(port, Some(9000));
    }

    #[test]
    fn returns_none_for_empty_ports_string() {
        assert!(parse_host_port("", 5432).is_none());
    }

    // ── match_services ────────────────────────────────────────────────────────

    #[test]
    fn matches_service_with_listening_port() {
        let svc = make_native_svc("api", 3000, "npm run dev");
        let procs = vec![DiscoveredProcess {
            pid: 42,
            cmdline: "npm run dev".into(),
            listening_ports: vec![3001],
        }];
        let matches = match_services(&[&svc], &procs);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].service_name, "api");
        assert_eq!(matches[0].pid, 42);
        assert_eq!(matches[0].port, Some(3001));
    }

    #[test]
    fn skips_service_with_no_matching_process() {
        let svc = make_native_svc("api", 3000, "npm run dev");
        let procs = vec![DiscoveredProcess {
            pid: 99,
            cmdline: "python manage.py runserver".into(),
            listening_ports: vec![8000],
        }];
        let matches = match_services(&[&svc], &procs);
        assert!(matches.is_empty());
    }

    #[test]
    fn skips_services_without_command() {
        let svc = make_docker_svc("postgres", 5432);
        let procs = vec![DiscoveredProcess {
            pid: 77,
            cmdline: "postgres".into(),
            listening_ports: vec![5432],
        }];
        let matches = match_services(&[&svc], &procs);
        assert!(matches.is_empty());
    }

    #[test]
    fn service_match_port_is_none_when_no_listening_port_in_subtree() {
        let svc = make_native_svc("api", 3000, "npm run dev");
        // Use a PID so high it almost certainly doesn't exist, ensuring no real subtree.
        let procs = vec![DiscoveredProcess {
            pid: 9_999_999,
            cmdline: "npm run dev".into(),
            listening_ports: vec![],
        }];
        let matches = match_services(&[&svc], &procs);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].port, None);
    }

    // ── pid_listening_ports ───────────────────────────────────────────────────

    #[test]
    fn pid_listening_ports_empty_for_dead_pid() {
        assert!(pid_listening_ports(9_999_999).is_empty());
    }

    #[test]
    fn pid_listening_ports_detects_own_bound_port() {
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let my_pid = std::process::id();
        assert!(pid_listening_ports(my_pid).contains(&port));
    }

    // ── child_pids ────────────────────────────────────────────────────────────

    #[test]
    fn child_pids_empty_for_dead_pid() {
        assert!(child_pids(9_999_999).is_empty());
    }

    // ── subtree_owns_port ─────────────────────────────────────────────────────

    #[test]
    fn subtree_owns_port_false_for_dead_pid() {
        assert!(!subtree_owns_port(9_999_999, 3000));
    }

    #[test]
    fn subtree_owns_port_false_when_port_not_owned() {
        // Port 1 is privileged — current process won't be listening on it.
        let my_pid = std::process::id();
        assert!(!subtree_owns_port(my_pid, 1));
    }

    #[test]
    fn subtree_owns_port_true_when_self_owns_port() {
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let my_pid = std::process::id();
        assert!(subtree_owns_port(my_pid, port));
    }

    // ── tmux_window_owns_port / tmux_window_exists ────────────────────────────

    #[test]
    fn tmux_window_owns_port_false_for_nonexistent_session() {
        assert!(!tmux_window_owns_port(
            "ecluse-nonexistent-xyz-9999",
            "api",
            3000
        ));
    }

    #[test]
    fn tmux_window_exists_false_for_nonexistent_session() {
        assert!(!tmux_window_exists("ecluse-nonexistent-xyz-9999", "api"));
    }

    // ── write_pid_file ────────────────────────────────────────────────────────

    #[test]
    fn write_pid_file_creates_correct_path() {
        let dir = TempDir::new().unwrap();
        let pid_path = write_pid_file(dir.path(), "my-slug", "api", 12345).unwrap();
        assert!(pid_path.ends_with("pids/my-slug/api.pid"));
        let content = std::fs::read_to_string(&pid_path).unwrap();
        assert_eq!(content, "12345");
    }

    #[test]
    fn write_pid_file_creates_parent_dirs() {
        let dir = TempDir::new().unwrap();
        let ecluse_dir = dir.path().join(".ecluse");
        let _ = write_pid_file(&ecluse_dir, "s1", "frontend", 9999).unwrap();
        assert!(ecluse_dir.join("pids/s1/frontend.pid").exists());
    }
}
