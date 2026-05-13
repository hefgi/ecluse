use anyhow::{Context, Result};
use chrono::Utc;
use std::path::Path;

use crate::compose;
use crate::config::Config;
use crate::docker;
use crate::env;
use crate::state::Session;
use crate::worktree::WorktreeManager;

pub struct ContainerMode;

impl super::ModeHandler for ContainerMode {
    fn bring_up(
        &self,
        slug: &str,
        slot: u8,
        offset: u16,
        branch: &str,
        config: &Config,
        root: &Path,
        watch: bool,
    ) -> Result<Session> {
        let wt = WorktreeManager::new(root.to_owned());
        let worktree_path = wt.worktree_path(config, slug);

        let compose_path = compose::find_compose_file(root)
            .ok_or_else(|| crate::error::EcluseError::ComposeFileNotFound(root.display().to_string()))?;

        let compose_data = compose::parse(&compose_path)?;

        let suffix = format!("{}_{}", config.prefix, slug);
        let overlay_dir = root.join(".ecluse").join("overlays");
        std::fs::create_dir_all(&overlay_dir)
            .context("failed to create overlays directory")?;
        let overlay_path = overlay_dir.join(format!("{}.yml", slug));

        let overlay_yaml = compose::generate_overlay(&compose_data, offset, &suffix, None)?;
        std::fs::write(&overlay_path, &overlay_yaml)
            .context("failed to write overlay file")?;

        // Create worktree
        wt.create(&worktree_path, branch)?;

        let project = format!("{}_{}", config.prefix, slug);
        let compose_str = compose_path.to_string_lossy().to_string();
        let overlay_str = overlay_path.to_string_lossy().to_string();

        // Bring up containers
        if let Err(e) = docker::compose_up(&project, &compose_str, Some(&overlay_str), watch) {
            // Rollback worktree
            let _ = wt.remove(&worktree_path);
            let _ = std::fs::remove_file(&overlay_path);
            return Err(e);
        }

        // Build env and write .env.ecluse
        let data_service_ports: Vec<(String, u16)> = compose_data
            .services
            .iter()
            .filter_map(|(name, svc)| {
                compose::service_host_port(svc, offset).map(|p| (name.clone(), p))
            })
            .collect();

        let env_map = env::build_env(slot, offset, "container", None, None, &data_service_ports);
        env::write_env_file(&worktree_path, &env_map)?;

        // Find the main web port for display
        let app_port = compose_data
            .services
            .values()
            .filter_map(|svc| compose::service_host_port(svc, offset))
            .next();

        Ok(Session {
            slug: slug.to_string(),
            mode: crate::config::Mode::Container,
            slot,
            offset,
            branch: branch.to_string(),
            worktree_path: worktree_path.display().to_string(),
            compose_project: Some(project),
            overlay_file: Some(overlay_str),
            app_port,
            database_name: None,
            started_at: Utc::now().to_rfc3339(),
        })
    }

    fn bring_down(
        &self,
        session: &Session,
        _config: &Config,
        root: &Path,
        keep_volumes: bool,
        _keep_database: bool,
    ) -> Result<()> {
        // Bring down compose
        if let Some(project) = &session.compose_project {
            if let Some(overlay) = &session.overlay_file {
                // Find compose file
                if let Some(compose_path) = compose::find_compose_file(root) {
                    let compose_str = compose_path.to_string_lossy().to_string();
                    docker::compose_down(project, &compose_str, Some(overlay), !keep_volumes)?;
                }
            }
            // Remove overlay file
            if let Some(overlay) = &session.overlay_file {
                let _ = std::fs::remove_file(overlay);
            }
        }

        // Remove worktree
        let wt = WorktreeManager::new(root.to_owned());
        let wt_path = std::path::PathBuf::from(&session.worktree_path);
        wt.remove(&wt_path)?;

        Ok(())
    }
}
