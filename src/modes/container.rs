use anyhow::{Context, Result};
use chrono::Utc;
use std::path::Path;

use crate::compose;
use crate::config::Config;
use crate::docker;
use crate::env;
use crate::hooks;
use crate::state::Session;
use crate::worktree::WorktreeManager;

pub struct ContainerMode;

impl super::ModeHandler for ContainerMode {
    fn bring_up(
        &self,
        slug: &str,
        slot: u8,
        branch: &str,
        config: &Config,
        root: &Path,
        watch: bool,
    ) -> Result<Session> {
        let wt = WorktreeManager::new(root.to_owned());
        let worktree_path = wt.worktree_path(config, slug);

        let compose_path = compose::find_compose_file(root).ok_or_else(|| {
            crate::error::EcluseError::ComposeFileNotFound(root.display().to_string())
        })?;

        let compose_data = compose::parse(&compose_path)?;

        let suffix = format!("{}_{}", config.prefix, slug);
        let overlay_dir = root.join(".ecluse").join("overlays");
        std::fs::create_dir_all(&overlay_dir).context("failed to create overlays directory")?;
        let overlay_path = overlay_dir.join(format!("{}.yml", slug));

        // Build port overrides from docker services config if defined
        let docker_svcs_config = config.docker_services();
        let overlay_yaml = if !docker_svcs_config.is_empty() {
            let port_overrides: std::collections::HashMap<String, u16> = docker_svcs_config
                .iter()
                .map(|s| (s.name.clone(), s.port(slot)))
                .collect();
            compose::generate_overlay_with_ports(&compose_data, &port_overrides, &suffix, None)?
        } else {
            // Fallback: use slot as offset for backward compat
            compose::generate_overlay(&compose_data, slot as u16, &suffix, None)?
        };
        std::fs::write(&overlay_path, &overlay_yaml).context("failed to write overlay file")?;

        wt.create(&worktree_path, branch)?;

        let project = format!("{}_{}", config.prefix, slug);
        let compose_str = compose_path.to_string_lossy().to_string();
        let overlay_str = overlay_path.to_string_lossy().to_string();

        if let Err(e) = docker::compose_up(&project, &compose_str, Some(&overlay_str), watch) {
            let _ = wt.remove(&worktree_path);
            let _ = std::fs::remove_file(&overlay_path);
            return Err(e);
        }

        // Docker service ports for env vars
        let docker_ports: Vec<(String, u16)> = if !docker_svcs_config.is_empty() {
            docker_svcs_config
                .iter()
                .map(|s| (s.name.clone(), s.port(slot)))
                .collect()
        } else {
            // Fallback: derive from compose data using slot as offset
            compose_data
                .services
                .iter()
                .filter_map(|(name, svc)| {
                    compose::service_host_port(svc, slot as u16).map(|p| (name.clone(), p))
                })
                .collect()
        };

        let env_map = env::build_env(
            slot,
            slug,
            "container",
            &indexmap::IndexMap::new(),
            &docker_ports,
        );
        env::write_env_file(&worktree_path, &env_map)?;

        if let Some(cmd) = &config.hooks.on_up {
            if let Err(e) = hooks::run(cmd, &worktree_path, &env_map) {
                let _ = docker::compose_down(&project, &compose_str, Some(&overlay_str), true);
                let _ = wt.remove(&worktree_path);
                let _ = std::fs::remove_file(&overlay_path);
                return Err(e);
            }
        }

        let app_port = if !docker_svcs_config.is_empty() {
            docker_svcs_config.first().map(|s| s.port(slot))
        } else {
            compose_data
                .services
                .values()
                .filter_map(|svc| compose::service_host_port(svc, slot as u16))
                .next()
        };

        Ok(Session {
            slug: slug.to_string(),
            mode: crate::config::Mode::Container,
            slot,
            branch: branch.to_string(),
            worktree_path: worktree_path.display().to_string(),
            compose_project: Some(project),
            overlay_file: Some(overlay_str),
            app_port,
            started_at: Utc::now().to_rfc3339(),
        })
    }

    fn bring_down(
        &self,
        session: &Session,
        _config: &Config,
        root: &Path,
        keep_volumes: bool,
    ) -> Result<()> {
        if let Some(project) = &session.compose_project {
            if let Some(overlay) = &session.overlay_file {
                if let Some(compose_path) = compose::find_compose_file(root) {
                    let compose_str = compose_path.to_string_lossy().to_string();
                    docker::compose_down(project, &compose_str, Some(overlay), !keep_volumes)?;
                }
            }
            if let Some(overlay) = &session.overlay_file {
                let _ = std::fs::remove_file(overlay);
            }
        }

        let wt = WorktreeManager::new(root.to_owned());
        let wt_path = std::path::PathBuf::from(&session.worktree_path);
        wt.remove(&wt_path)?;

        Ok(())
    }
}
