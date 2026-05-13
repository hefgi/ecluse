use anyhow::{Context, Result};
use chrono::Utc;
use std::path::Path;
use std::process::Command;

use crate::config::Config;
use crate::env;
use crate::postgres::PgClient;
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

        let app_port = offset; // first port in range = base + offset (stride * slot, relative)

        // Check port is free
        check_port_free(app_port)?;

        // Create worktree
        wt.create(&worktree_path, branch)?;

        // Provision database if configured
        let database_name = if config.is_db_enabled() {
            let pg = PgClient::from_config(&config.database);
            if !pg.is_reachable() {
                let _ = wt.remove(&worktree_path);
                return Err(crate::error::EcluseError::PostgresUnreachable.into());
            }
            let db_name = PgClient::db_name(&config.database.base, slug);
            if let Err(e) = pg.create_db(&db_name) {
                let _ = wt.remove(&worktree_path);
                return Err(e);
            }
            Some(db_name)
        } else {
            None
        };

        let env_map = env::build_env(
            slot,
            offset,
            "host",
            Some(app_port),
            database_name.as_deref(),
            &[],
        );
        env::write_env_file(&worktree_path, &env_map)?;

        Ok(Session {
            slug: slug.to_string(),
            mode: crate::config::Mode::Host,
            slot,
            offset,
            branch: branch.to_string(),
            worktree_path: worktree_path.display().to_string(),
            compose_project: None,
            overlay_file: None,
            app_port: Some(app_port),
            database_name,
            started_at: Utc::now().to_rfc3339(),
        })
    }

    fn bring_down(
        &self,
        session: &Session,
        config: &Config,
        root: &Path,
        _keep_volumes: bool,
        keep_database: bool,
    ) -> Result<()> {
        // Drop database unless --keep-database
        if !keep_database {
            if let Some(db) = &session.database_name {
                let pg = PgClient::from_config(&config.database);
                pg.drop_db(db)?;
            }
        }

        // Remove worktree
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
        // Parse PID from lsof output
        let stdout = String::from_utf8_lossy(&output.stdout);
        let pid: u32 = stdout
            .lines()
            .skip(1) // header
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|p| p.parse().ok())
            .unwrap_or(0);

        return Err(crate::error::EcluseError::PortInUse { port, pid }.into());
    }
    Ok(())
}
