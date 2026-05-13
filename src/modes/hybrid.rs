use anyhow::{Context, Result};
use chrono::Utc;
use indexmap::IndexMap;
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

        // Determine which compose services are "data" (docker) services
        // Prefer config.docker_services() if [[services]] are defined,
        // otherwise fall back to label-based detection.
        let docker_svcs_config = config.docker_services();
        let data_svcs: Vec<String> = if !docker_svcs_config.is_empty() {
            docker_svcs_config.iter().map(|s| s.name.clone()).collect()
        } else {
            let app_svcs =
                compose::app_services(&compose_data, &config.app_label, &config.app_label_value);
            let data =
                compose::data_services(&compose_data, &config.app_label, &config.app_label_value);
            if app_svcs.is_empty() {
                tracing::warn!(
                    "No service labeled {}={} found; treating all services as data.",
                    config.app_label,
                    config.app_label_value
                );
            }
            data
        };

        // Build port overrides for docker services: service_name → host port
        let docker_port_overrides: std::collections::HashMap<String, u16> =
            if !docker_svcs_config.is_empty() {
                docker_svcs_config
                    .iter()
                    .map(|s| (s.name.clone(), s.port(slot)))
                    .collect()
            } else {
                std::collections::HashMap::new()
            };

        let suffix = format!("{}_{}", config.prefix, slug);
        let overlay_dir = root.join(".ecluse").join("overlays");
        std::fs::create_dir_all(&overlay_dir).context("failed to create overlays directory")?;
        let overlay_path = overlay_dir.join(format!("{}.yml", slug));

        let overlay_yaml = if docker_port_overrides.is_empty() {
            // Fallback: use offset-based rewriting (no explicit service config)
            // For the fallback, we use slot as offset so slot 1 = offset 1
            compose::generate_overlay(&compose_data, slot as u16, &suffix, Some(&data_svcs))?
        } else {
            compose::generate_overlay_with_ports(
                &compose_data,
                &docker_port_overrides,
                &suffix,
                Some(&data_svcs),
            )?
        };
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

        // Native ports from [[services]] config (or fallback)
        let native_ports: IndexMap<String, u16> = {
            let native = config.native_services();
            if native.is_empty() {
                let mut m = IndexMap::new();
                m.insert("app".to_string(), 3000u16 + slot as u16);
                m
            } else {
                native
                    .iter()
                    .map(|s| (s.name.clone(), s.port(slot)))
                    .collect()
            }
        };

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
                    if data_svcs.contains(name) {
                        compose::service_host_port(svc, slot as u16).map(|p| (name.clone(), p))
                    } else {
                        None
                    }
                })
                .collect()
        };

        let env_map = env::build_env(slot, slug, "hybrid", &native_ports, &docker_ports);
        env::write_env_file(&worktree_path, &env_map)?;

        if let Some(cmd) = &config.hooks.on_up {
            if let Err(e) = hooks::run(cmd, &worktree_path, &env_map) {
                let _ = docker::compose_down(&project, &compose_str, Some(&overlay_str), true);
                let _ = wt.remove(&worktree_path);
                let _ = std::fs::remove_file(&overlay_path);
                return Err(e);
            }
        }

        let app_port = native_ports.values().next().copied();

        Ok(Session {
            slug: slug.to_string(),
            mode: crate::config::Mode::Hybrid,
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
        config: &Config,
        root: &Path,
        keep_volumes: bool,
    ) -> Result<()> {
        if let Some(cmd) = &config.hooks.on_down {
            let native = config.native_services();
            let native_ports: IndexMap<String, u16> = if native.is_empty() {
                let mut m = IndexMap::new();
                m.insert("app".to_string(), 3000u16 + session.slot as u16);
                m
            } else {
                native
                    .iter()
                    .map(|s| (s.name.clone(), s.port(session.slot)))
                    .collect()
            };
            let env_map = env::build_env(session.slot, &session.slug, "hybrid", &native_ports, &[]);
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
