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

pub struct HybridMode;

impl super::ModeHandler for HybridMode {
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

        let compose_path = compose::find_compose_file(root).ok_or_else(|| {
            crate::error::EcluseError::ComposeFileNotFound(root.display().to_string())
        })?;

        let compose_data = compose::parse(&compose_path)?;

        let app_svcs =
            compose::app_services(&compose_data, &config.app_label, &config.app_label_value);
        let data_svcs =
            compose::data_services(&compose_data, &config.app_label, &config.app_label_value);

        if app_svcs.is_empty() {
            tracing::warn!(
                "No service labeled {}={} found; treating all services as data.",
                config.app_label,
                config.app_label_value
            );
        }

        let suffix = format!("{}_{}", config.prefix, slug);
        let overlay_dir = root.join(".ecluse").join("overlays");
        std::fs::create_dir_all(&overlay_dir).context("failed to create overlays directory")?;
        let overlay_path = overlay_dir.join(format!("{}.yml", slug));

        let overlay_yaml =
            compose::generate_overlay(&compose_data, offset, &suffix, Some(&data_svcs))?;
        std::fs::write(&overlay_path, &overlay_yaml).context("failed to write overlay file")?;

        wt.create(&worktree_path, branch)?;

        let project = format!("{}_{}", config.prefix, slug);
        let compose_str = compose_path.to_string_lossy().to_string();
        let overlay_str = overlay_path.to_string_lossy().to_string();

        let data_svc_refs: Vec<&str> = data_svcs.iter().map(|s| s.as_str()).collect();
        if let Err(e) = docker::compose_up_services(
            &project,
            &compose_str,
            Some(&overlay_str),
            &data_svc_refs,
            watch,
        ) {
            let _ = wt.remove(&worktree_path);
            let _ = std::fs::remove_file(&overlay_path);
            return Err(e);
        }

        // Named ports from [ports] config (app services running natively)
        let named_ports = env::effective_named_ports(&config.ports, config.base_port, offset);

        // Data service ports from compose
        let data_service_ports: Vec<(String, u16)> = compose_data
            .services
            .iter()
            .filter_map(|(name, svc)| {
                if data_svcs.contains(name) {
                    compose::service_host_port(svc, offset).map(|p| (name.clone(), p))
                } else {
                    None
                }
            })
            .collect();

        let env_map = env::build_env(slot, slug, offset, "hybrid", &named_ports, &data_service_ports);
        env::write_env_file(&worktree_path, &env_map)?;

        if let Some(cmd) = &config.hooks.on_up {
            if let Err(e) = hooks::run(cmd, &worktree_path, &env_map) {
                let _ = docker::compose_down(&project, &compose_str, Some(&overlay_str), true);
                let _ = wt.remove(&worktree_path);
                let _ = std::fs::remove_file(&overlay_path);
                return Err(e);
            }
        }

        let app_port = named_ports.values().next().copied();

        Ok(Session {
            slug: slug.to_string(),
            mode: crate::config::Mode::Hybrid,
            slot,
            offset,
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
        config: &Config,
        root: &Path,
        keep_volumes: bool,
    ) -> Result<()> {
        if let Some(cmd) = &config.hooks.on_down {
            let named_ports = env::effective_named_ports(&config.ports, config.base_port, session.offset);
            let env_map = env::build_env(
                session.slot,
                &session.slug,
                session.offset,
                "hybrid",
                &named_ports,
                &[],
            );
            hooks::run(cmd, std::path::Path::new(&session.worktree_path), &env_map)?;
        }

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
