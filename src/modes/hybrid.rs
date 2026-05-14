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
use crate::validate;
use crate::worktree::WorktreeManager;

use super::{group_by_compose, overlay_name_for_compose, tear_down_all_overlays};

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

        // ── Determine data services and bring them up ─────────────────────────
        // When [[services]] with run="docker" are defined, group them by their
        // compose file so each group gets its own overlay + docker compose call.
        // When no [[services]] are defined, fall back to label-based detection
        // against the root compose file.

        let mut allocated_docker_ports: Vec<(String, u16)> = vec![];
        let mut written_overlays: Vec<String> = vec![]; // (overlay_path)

        if !docker_svcs_config.is_empty() {
            // Group docker services by their resolved compose file path.
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
                        allocated_docker_ports.push((s.name.clone(), port));
                        Ok((s.name.clone(), port))
                    })
                    .collect::<Result<_>>()?;

                let svc_names: Vec<String> = svcs.iter().map(|s| s.name.clone()).collect();
                let overlay_name = overlay_name_for_compose(slug, compose_path, root);
                let overlay_path = overlay_dir.join(&overlay_name);

                let yaml = compose::generate_overlay_with_ports(
                    &compose_data,
                    &port_map,
                    &suffix,
                    Some(&svc_names),
                )?;
                std::fs::write(&overlay_path, &yaml)
                    .context("failed to write overlay file")?;

                let compose_str = compose_path.to_string_lossy().to_string();
                let overlay_str = overlay_path.to_string_lossy().to_string();
                let svc_refs: Vec<&str> = svc_names.iter().map(|s| s.as_str()).collect();

                if let Err(e) = docker::compose_up_services(
                    &project,
                    &compose_str,
                    Some(&overlay_str),
                    &svc_refs,
                    watch,
                ) {
                    // Roll back overlays written so far
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
            // Fallback: no [[services]] defined — use label detection on root compose file
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

            let overlay_path = overlay_dir.join(format!("{}.yml", slug));
            let yaml = compose::generate_overlay(
                &compose_data,
                slot as u16,
                &suffix,
                Some(&data_svcs),
            )?;
            std::fs::write(&overlay_path, &yaml).context("failed to write overlay file")?;

            let compose_str = compose_path.to_string_lossy().to_string();
            let overlay_str = overlay_path.to_string_lossy().to_string();
            let data_refs: Vec<&str> = data_svcs.iter().map(|s| s.as_str()).collect();

            if let Err(e) = docker::compose_up_services(
                &project,
                &compose_str,
                Some(&overlay_str),
                &data_refs,
                watch,
            ) {
                let _ = std::fs::remove_file(&overlay_path);
                if !reuse_worktree {
                    let _ = wt.remove(&worktree_path);
                }
                return Err(e);
            }

            // Derive ports for env vars from compose data
            for (name, svc) in &compose_data.services {
                if data_svcs.contains(name) {
                    if let Some(p) = compose::service_host_port(svc, slot as u16) {
                        allocated_docker_ports.push((name.clone(), p));
                    }
                }
            }

            written_overlays.push(overlay_str);
        }

        // ── Worktree ──────────────────────────────────────────────────────────
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

        // ── Native ports ──────────────────────────────────────────────────────
        let native_ports: IndexMap<String, u16> = {
            let native = config.native_services();
            if native.is_empty() {
                let port = if let Some(&p) = port_overrides.get("app") {
                    p
                } else {
                    let fallback = crate::config::ServiceConfig {
                        name: "app".into(),
                        base_port: 3000,
                        run: crate::config::ServiceRun::Native,
                        compose: None,
                    };
                    validate::find_free_port(config, &fallback, slot)?
                };
                let mut m = IndexMap::new();
                m.insert("app".to_string(), port);
                m
            } else {
                native
                    .iter()
                    .map(|s| {
                        let port = if let Some(&p) = port_overrides.get(&s.name) {
                            p
                        } else {
                            validate::find_free_port(config, s, slot)?
                        };
                        Ok((s.name.clone(), port))
                    })
                    .collect::<Result<IndexMap<_, _>>>()?
            }
        };

        let env_map = env::build_env(slot, slug, "hybrid", &native_ports, &allocated_docker_ports);
        env::write_env_file(&worktree_path, &env_map)?;

        if let Some(cmd) = &config.hooks.on_up {
            if let Err(e) = hooks::run(cmd, &worktree_path, &env_map) {
                // Best-effort teardown of all compose groups
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

        let app_port = native_ports.values().next().copied();

        let mut all_ports: std::collections::HashMap<String, u16> =
            native_ports.iter().map(|(k, v)| (k.clone(), *v)).collect();
        for (name, port) in &allocated_docker_ports {
            all_ports.insert(name.clone(), *port);
        }

        // Primary overlay is the first one written (for backward compat with
        // sessions that only have a single compose file).
        let primary_overlay = written_overlays.first().cloned();
        let extra_overlays: Vec<String> = written_overlays.into_iter().skip(1).collect();

        Ok(Session {
            slug: slug.to_string(),
            mode: crate::config::Mode::Hybrid,
            slot,
            branch: branch.to_string(),
            worktree_path: worktree_path.display().to_string(),
            compose_project: Some(project),
            overlay_file: primary_overlay,
            overlay_files: extra_overlays,
            app_port,
            started_at: Utc::now().to_rfc3339(),
            port_overrides: all_ports,
        })
    }

    fn bring_down(
        &self,
        session: &Session,
        config: &Config,
        root: &Path,
        keep_volumes: bool,
        keep_worktree: bool,
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
            let env_map =
                env::build_env(session.slot, &session.slug, "hybrid", &native_ports, &[]);
            hooks::run(cmd, std::path::Path::new(&session.worktree_path), &env_map)?;
        }

        if let Some(project) = &session.compose_project {
            // Collect all (compose_file, overlay) pairs to tear down.
            // primary overlay_file pairs with the root compose; extra overlay_files
            // are matched by extracting the compose path from the overlay name.
            let all_overlays: Vec<&str> = session
                .overlay_file
                .iter()
                .map(|s| s.as_str())
                .chain(session.overlay_files.iter().map(|s| s.as_str()))
                .collect();

            tear_down_all_overlays(project, root, &all_overlays.iter().map(|s| s.to_string()).collect::<Vec<_>>(), !keep_volumes);

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

