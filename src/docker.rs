use anyhow::{Context, Result};
use std::process::Command;

pub fn is_available() -> bool {
    Command::new("docker")
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

    let status = Command::new("docker")
        .args(&args)
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

    let status = Command::new("docker")
        .args(&args)
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

    let status = Command::new("docker")
        .args(&args)
        .status()
        .context("failed to run docker compose down")?;

    if !status.success() {
        tracing::warn!("docker compose down exited non-zero; continuing teardown");
    }
    Ok(())
}
