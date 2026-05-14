use anyhow::{Context, Result};
use chrono::Utc;
use std::path::Path;

use crate::compose;
use crate::config::Config;
use crate::docker;
use crate::env;
use crate::hooks;
use crate::state::Session;
use crate::validate;
use crate::worktree::WorktreeManager;

use super::{group_by_compose, overlay_name_for_compose, tear_down_all_overlays};

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
        reuse_worktree: bool,
        port_overrides: &std::collections::HashMap<String, u16>,
    ) -> Result<Session> {
        let wt = WorktreeManager::new(root.to_owned());
        let worktree_path = wt.worktree_path(config, slug);

        let suffix = format!("{}_{}", config.prefix, slug);
        let project = format!("{}_{}", config.prefix, slug);
        let overlay_dir = root.join(".ecluse").join("overlays");
        std::fs::create_dir_all(&overlay_dir).context("failed to create overlays directory")?;

        let docker_svcs_config = config.docker_services();

        let mut allocated_ports: Vec<(String, u16)> = vec![];
        let mut written_overlays: Vec<String> = vec![];

        if !docker_svcs_config.is_empty() {
            // Group services by compose file — each group gets its own overlay + up call.
            let groups = group_by_compose(root, &docker_svcs_config)?;

            for (compose_path, svcs) in &groups {
                let compose_data = compose::parse(compose_path)?;

                let port_map: std::collections::HashMap<String, u16> = svcs
                    .iter()
                    .map(|s| {
                        let port = if let Some(&p) = port_overrides.get(&s.name) {
                            p
                        } else {
                            validate::find_free_port(config, s, slot)?
                        };
                        allocated_ports.push((s.name.clone(), port));
                        Ok((s.name.clone(), port))
                    })
                    .collect::<Result<_>>()?;

                let overlay_name = overlay_name_for_compose(slug, compose_path, root);
                let overlay_path = overlay_dir.join(&overlay_name);

                let yaml =
                    compose::generate_overlay_with_ports(&compose_data, &port_map, &suffix, None)?;
                std::fs::write(&overlay_path, &yaml).context("failed to write overlay file")?;

                let compose_str = compose_path.to_string_lossy().to_string();
                let overlay_str = overlay_path.to_string_lossy().to_string();

                if let Err(e) =
                    docker::compose_up(&project, &compose_str, Some(&overlay_str), watch)
                {
                    for ov in &written_overlays {
                        let _ = std::fs::remove_file(ov);
                    }
                    let _ = std::fs::remove_file(&overlay_path);
                    if !reuse_worktree {
                        let _ = wt.remove(&worktree_path);
                    }
                    return Err(e);
                }

                written_overlays.push(overlay_str);
            }
        } else {
            // Fallback: no [[services]] — single root compose file, offset by slot
            let compose_path = compose::find_compose_file(root).ok_or_else(|| {
                crate::error::EcluseError::ComposeFileNotFound(root.display().to_string())
            })?;
            let compose_data = compose::parse(&compose_path)?;

            let overlay_path = overlay_dir.join(format!("{}.yml", slug));
            let yaml = compose::generate_overlay(&compose_data, slot as u16, &suffix, None)?;
            std::fs::write(&overlay_path, &yaml).context("failed to write overlay file")?;

            let compose_str = compose_path.to_string_lossy().to_string();
            let overlay_str = overlay_path.to_string_lossy().to_string();

            if let Err(e) = docker::compose_up(&project, &compose_str, Some(&overlay_str), watch) {
                let _ = std::fs::remove_file(&overlay_path);
                if !reuse_worktree {
                    let _ = wt.remove(&worktree_path);
                }
                return Err(e);
            }

            allocated_ports = compose_data
                .services
                .iter()
                .filter_map(|(name, svc)| {
                    compose::service_host_port(svc, slot as u16).map(|p| (name.clone(), p))
                })
                .collect();

            written_overlays.push(overlay_str);
        }

        if reuse_worktree {
            if !worktree_path.exists() {
                return Err(anyhow::anyhow!(
                    "worktree not found at {}; remove --reuse-worktree or run ecluse up without it",
                    worktree_path.display()
                ));
            }
        } else {
            if let Err(e) = wt.create(&worktree_path, branch) {
                for ov in &written_overlays {
                    let _ = std::fs::remove_file(ov);
                }
                return Err(e);
            }
        }

        let env_map = env::build_env(
            slot,
            slug,
            "container",
            &indexmap::IndexMap::new(),
            &allocated_ports,
        );
        env::write_env_file(&worktree_path, &env_map)?;

        if let Some(cmd) = &config.hooks.on_up {
            if let Err(e) = hooks::run(cmd, &worktree_path, &env_map) {
                tear_down_all_overlays(&project, root, &written_overlays, true);
                if !reuse_worktree {
                    let _ = wt.remove(&worktree_path);
                }
                for ov in &written_overlays {
                    let _ = std::fs::remove_file(ov);
                }
                return Err(e);
            }
        }

        let app_port = allocated_ports.first().map(|(_, p)| *p);
        let stored_port_overrides: std::collections::HashMap<String, u16> =
            allocated_ports.iter().cloned().collect();

        let primary_overlay = written_overlays.first().cloned();
        let extra_overlays: Vec<String> = written_overlays.into_iter().skip(1).collect();

        Ok(Session {
            slug: slug.to_string(),
            mode: crate::config::Mode::Container,
            slot,
            branch: branch.to_string(),
            worktree_path: worktree_path.display().to_string(),
            compose_project: Some(project),
            overlay_file: primary_overlay,
            overlay_files: extra_overlays,
            app_port,
            started_at: Utc::now().to_rfc3339(),
            port_overrides: stored_port_overrides,
            process_manager: None,
            tmux_session: None,
            pid_files: vec![],
            log_dir: None,
        })
    }

    fn bring_down(
        &self,
        session: &Session,
        _config: &Config,
        root: &Path,
        keep_volumes: bool,
        keep_worktree: bool,
    ) -> Result<()> {
        if let Some(project) = &session.compose_project {
            let all_overlays: Vec<String> = session
                .overlay_file
                .iter()
                .cloned()
                .chain(session.overlay_files.iter().cloned())
                .collect();

            tear_down_all_overlays(project, root, &all_overlays, !keep_volumes);

            for ov in &all_overlays {
                let _ = std::fs::remove_file(ov);
            }
        }

        if !keep_worktree {
            let wt = WorktreeManager::new(root.to_owned());
            let wt_path = std::path::PathBuf::from(&session.worktree_path);
            wt.remove(&wt_path)?;
        }

        Ok(())
    }
}
