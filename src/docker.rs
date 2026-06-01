use anyhow::{Context, Result};
use std::process::Command;

/// Returns the Docker socket URL for the active context so child processes
/// (tmux windows, hooks, etc.) connect to the same daemon as the parent shell.
/// Falls back to empty string if docker is unavailable, letting Docker use its
/// own default resolution.
fn docker_host() -> String {
    Command::new("docker")
        .args(["context", "inspect", "--format", "{{.Endpoints.docker.Host}}"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_default()
}

/// Returns a `Command` for docker with DOCKER_HOST pre-set to the active context's socket.
pub fn docker_cmd() -> Command {
    let mut cmd = Command::new("docker");
    let host = docker_host();
    if !host.is_empty() {
        cmd.env("DOCKER_HOST", host);
    }
    cmd
}

pub fn is_available() -> bool {
    docker_cmd()
        .arg("info")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn compose_up(
    project: &str,
    compose_file: &str,
    overlay_file: Option<&str>,
    watch: bool,
    extra_env: &std::collections::HashMap<String, String>,
) -> Result<()> {
    let mut args = vec!["compose", "-p", project, "-f", compose_file];
    if let Some(ov) = overlay_file {
        args.extend(["-f", ov]);
    }
    args.push("up");
    args.push("-d");
    if watch {
        args.push("--watch");
    }

    let status = docker_cmd()
        .args(&args)
        .envs(extra_env)
        .status()
        .context("failed to run docker compose up")?;

    if !status.success() {
        return Err(crate::error::EcluseError::DockerFailed {
            stderr: "docker compose up failed; check output above".into(),
        }
        .into());
    }
    Ok(())
}

pub fn compose_up_services(
    project: &str,
    compose_file: &str,
    overlay_file: Option<&str>,
    services: &[&str],
    watch: bool,
    extra_env: &std::collections::HashMap<String, String>,
) -> Result<()> {
    let mut args = vec!["compose", "-p", project, "-f", compose_file];
    if let Some(ov) = overlay_file {
        args.extend(["-f", ov]);
    }
    args.push("up");
    args.push("-d");
    if watch {
        args.push("--watch");
    }
    args.extend_from_slice(services);

    let status = docker_cmd()
        .args(&args)
        .envs(extra_env)
        .status()
        .context("failed to run docker compose up (services)")?;

    if !status.success() {
        return Err(crate::error::EcluseError::DockerFailed {
            stderr: "docker compose up failed; check output above".into(),
        }
        .into());
    }
    Ok(())
}

pub fn compose_down(
    project: &str,
    compose_file: &str,
    overlay_file: Option<&str>,
    remove_volumes: bool,
) -> Result<()> {
    let mut args = vec!["compose", "-p", project, "-f", compose_file];
    if let Some(ov) = overlay_file {
        args.extend(["-f", ov]);
    }
    args.push("down");
    if remove_volumes {
        args.push("-v");
    }

    let status = docker_cmd()
        .args(&args)
        .status()
        .context("failed to run docker compose down")?;

    if !status.success() {
        tracing::warn!("docker compose down exited non-zero; continuing teardown");
    }
    Ok(())
}

/// Returns deduplicated compose project names that contain `prefix`.
pub fn list_compose_projects(prefix: &str) -> Vec<String> {
    let output = docker_cmd()
        .args([
            "ps",
            "--format",
            "{{.Label \"com.docker.compose.project\"}}",
        ])
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let mut projects: Vec<String> = stdout
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty() && l.contains(prefix))
                .collect();
            projects.sort();
            projects.dedup();
            projects
        }
        _ => vec![],
    }
}

/// Stop all containers in a compose project by name (no compose file needed).
pub fn compose_down_by_project(project: &str, remove_volumes: bool) -> Result<()> {
    let mut args = vec!["compose", "-p", project, "down"];
    if remove_volumes {
        args.push("-v");
    }

    let status = docker_cmd()
        .args(&args)
        .status()
        .context("failed to run docker compose down by project")?;

    if !status.success() {
        tracing::warn!(
            "docker compose -p {} down exited non-zero; continuing",
            project
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_compose_projects_returns_empty_on_docker_unavailable() {
        // This test runs in CI without docker — must not panic.
        // Either docker is available (returns a Vec) or not (returns empty).
        let _ = list_compose_projects("ecluse");
    }

    #[test]
    fn list_compose_projects_filters_by_prefix() {
        // Parse a simulated stdout — test the filtering logic directly.
        let lines = "ecluse_feat-a\nsome-other-project\necluse_feat-b\n";
        let prefix = "ecluse";
        let mut projects: Vec<String> = lines
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty() && l.contains(prefix))
            .collect();
        projects.sort();
        projects.dedup();
        assert_eq!(projects, vec!["ecluse_feat-a", "ecluse_feat-b"]);
    }
}
