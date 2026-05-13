use anyhow::{Context, Result};
use chrono::Utc;
use std::path::Path;
use std::process::Command;

use crate::config::Config;
use crate::env;
use crate::hooks;
use crate::state::Session;
use crate::worktree::WorktreeManager;

pub struct HostMode;

impl super::ModeHandler for HostMode {
    fn bring_up(
        &self,
        slug: &str,
        slot: u8,
        offset: u16,
        branch: &str,
        config: &Config,
        root: &Path,
        _watch: bool,
    ) -> Result<Session> {
        let wt = WorktreeManager::new(root.to_owned());
        let worktree_path = wt.worktree_path(config, slug);

        let named_ports = env::effective_named_ports(&config.ports, config.base_port, offset);

        for port in named_ports.values() {
            check_port_free(*port)?;
        }

        wt.create(&worktree_path, branch)?;

        let env_map = env::build_env(slot, slug, offset, "host", &named_ports, &[]);
        env::write_env_file(&worktree_path, &env_map)?;

        if let Some(cmd) = &config.hooks.on_up {
            if let Err(e) = hooks::run(cmd, &worktree_path, &env_map) {
                let _ = wt.remove(&worktree_path);
                return Err(e);
            }
        }

        let app_port = named_ports.values().next().copied();

        Ok(Session {
            slug: slug.to_string(),
            mode: crate::config::Mode::Host,
            slot,
            offset,
            branch: branch.to_string(),
            worktree_path: worktree_path.display().to_string(),
            compose_project: None,
            overlay_file: None,
            app_port,
            started_at: Utc::now().to_rfc3339(),
        })
    }

    fn bring_down(
        &self,
        session: &Session,
        config: &Config,
        root: &Path,
        _keep_volumes: bool,
    ) -> Result<()> {
        if let Some(cmd) = &config.hooks.on_down {
            let named_ports = env::effective_named_ports(&config.ports, config.base_port, session.offset);
            let env_map = env::build_env(
                session.slot,
                &session.slug,
                session.offset,
                "host",
                &named_ports,
                &[],
            );
            hooks::run(cmd, std::path::Path::new(&session.worktree_path), &env_map)?;
        }

        let wt = WorktreeManager::new(root.to_owned());
        let wt_path = std::path::PathBuf::from(&session.worktree_path);
        wt.remove(&wt_path)?;

        Ok(())
    }
}

fn check_port_free(port: u16) -> Result<()> {
    let output = Command::new("lsof")
        .args(["-iTCP", &format!(":{}", port), "-sTCP:LISTEN", "-n", "-P"])
        .output()
        .context("failed to run lsof")?;

    if output.status.success() && !output.stdout.is_empty() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let pid: u32 = stdout
            .lines()
            .nth(1)
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|p| p.parse().ok())
            .unwrap_or(0);
        return Err(crate::error::EcluseError::PortInUse { port, pid }.into());
    }
    Ok(())
}
