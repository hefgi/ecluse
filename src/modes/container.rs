use anyhow::{Context, Result};
use chrono::Utc;
use std::path::Path;

use crate::compose;
use crate::config::Config;
use crate::docker;
use crate::env;
use crate::hooks;
use crate::log::StepLogger;
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
        no_inherit_env: bool,
        worktree_override: Option<std::path::PathBuf>,
        port_overrides: &std::collections::HashMap<String, u16>,
        service_filter: Option<&std::collections::HashSet<String>>,
        skip_services: &std::collections::HashSet<String>,
        existing_port_overrides: &std::collections::HashMap<String, u16>,
        log: &StepLogger,
    ) -> Result<Session> {
        let wt = WorktreeManager::new(root.to_owned());
        let worktree_path = worktree_override.unwrap_or_else(|| wt.worktree_path(config, slug));

        let suffix = format!("{}_{}", config.prefix, slug);
        let project = format!("{}_{}", config.prefix, slug);
        let overlay_dir = root.join(".ecluse").join("overlays");
        std::fs::create_dir_all(&overlay_dir).context("failed to create overlays directory")?;

        // pre_up: before anything exists — runs from repo root, no env vars yet
        if let Some(cmd) = &config.hooks.pre_up {
            log.step("Running pre_up hook...");
            log.detail(cmd);
            hooks::run(cmd, root, &std::collections::HashMap::new())?;
        }

        let docker_svcs_config: Vec<_> = config
            .docker_services()
            .into_iter()
            .filter(|s| service_filter.is_none_or(|f| f.contains(&s.name)))
            .collect();

        let mut allocated_ports: Vec<(String, u16)> = vec![];
        let mut written_overlays: Vec<String> = vec![];

        // Copy ports for skipped docker services from existing session.
        for svc in &docker_svcs_config {
            if skip_services.contains(&svc.name) {
                if let Some(&p) = existing_port_overrides.get(&svc.name) {
                    log.detail(&format!("{}: already running — skipped", svc.name));
                    allocated_ports.push((svc.name.clone(), p));
                }
            }
        }

        let docker_svcs_to_start: Vec<_> = docker_svcs_config
            .iter()
            .filter(|s| !skip_services.contains(&s.name))
            .copied()
            .collect();

        if !docker_svcs_config.is_empty() {
            if !docker_svcs_to_start.is_empty() {
                let groups = group_by_compose(root, &docker_svcs_to_start)?;

                for (compose_path, svcs) in &groups {
                    let svc_names: Vec<String> = svcs.iter().map(|s| s.name.clone()).collect();
                    log.step(&format!(
                        "Starting docker services: {}...",
                        svc_names.join(", ")
                    ));

                    let compose_data = compose::parse(compose_path)?;

                    // (host_port, container_port) — container_port defaults to base_port
                    let port_map: std::collections::HashMap<String, (u16, u16)> = svcs
                        .iter()
                        .map(|s| {
                            let host_port = if let Some(&p) = port_overrides.get(&s.name) {
                                p
                            } else {
                                validate::find_free_port(config, s, slot)?
                            };
                            allocated_ports.push((s.name.clone(), host_port));
                            log.detail(&format!("{}: {}", s.name, host_port));
                            Ok((s.name.clone(), (host_port, s.base_port)))
                        })
                        .collect::<Result<_>>()?;

                    let overlay_name = overlay_name_for_compose(slug, compose_path, root);
                    let overlay_path = overlay_dir.join(&overlay_name);

                    // Build extra_port_map: service_name → [(host_port, container_port)]
                    // for extra_ports entries on each docker service.
                    let extra_port_map: std::collections::HashMap<String, Vec<(u16, u16)>> = svcs
                        .iter()
                        .filter_map(|s| {
                            let extras: Vec<(u16, u16)> = s
                                .extra_ports
                                .iter()
                                .map(|ep| (ep.base_port.saturating_add(slot as u16), ep.base_port))
                                .collect();
                            if extras.is_empty() {
                                None
                            } else {
                                Some((s.name.clone(), extras))
                            }
                        })
                        .collect();

                    // Build env map for compose interpolation: ECLUSE_<NAME>_PORT + extra_ports vars
                    let mut compose_env: std::collections::HashMap<String, String> = port_map
                        .iter()
                        .map(|(n, (hp, _))| {
                            (format!("ECLUSE_{}_PORT", n.to_uppercase()), hp.to_string())
                        })
                        .collect();
                    for svc in svcs {
                        for ep in &svc.extra_ports {
                            let host_port = ep.base_port.saturating_add(slot as u16);
                            compose_env.insert(ep.port_env.clone(), host_port.to_string());
                        }
                    }

                    let yaml = compose::generate_overlay_with_ports(
                        &compose_data,
                        &port_map,
                        &extra_port_map,
                        &suffix,
                        None,
                        &config.prefix,
                        slot,
                    )?;
                    std::fs::write(&overlay_path, &yaml).context("failed to write overlay file")?;

                    let compose_str = compose_path.to_string_lossy().to_string();
                    let overlay_str = overlay_path.to_string_lossy().to_string();

                    if let Err(e) = docker::compose_up(
                        &project,
                        &compose_str,
                        Some(&overlay_str),
                        watch,
                        &compose_env,
                    ) {
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
            } // end if !docker_svcs_to_start.is_empty()
        } else {
            let compose_path = compose::find_compose_file(root).ok_or_else(|| {
                crate::error::EcluseError::ComposeFileNotFound(root.display().to_string())
            })?;
            let compose_data = compose::parse(&compose_path)?;

            let all_svc_names: Vec<String> = compose_data.services.keys().cloned().collect();
            log.step(&format!(
                "Starting docker services: {}...",
                all_svc_names.join(", ")
            ));

            let overlay_path = overlay_dir.join(format!("{}.yml", slug));
            let yaml = compose::generate_overlay(
                &compose_data,
                slot as u16,
                &suffix,
                None,
                &config.prefix,
                slot,
            )?;
            std::fs::write(&overlay_path, &yaml).context("failed to write overlay file")?;

            let compose_str = compose_path.to_string_lossy().to_string();
            let overlay_str = overlay_path.to_string_lossy().to_string();

            if let Err(e) = docker::compose_up(
                &project,
                &compose_str,
                Some(&overlay_str),
                watch,
                &std::collections::HashMap::new(),
            ) {
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
                    compose::service_host_port(svc, slot as u16).map(|p| {
                        log.detail(&format!("{name}: {p}"));
                        (name.clone(), p)
                    })
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
            log.step("Reusing existing worktree...");
            log.detail(&worktree_path.display().to_string());
        } else {
            log.step(&format!("Creating worktree (branch: {branch})..."));
            log.detail(&worktree_path.display().to_string());
            if let Err(e) = wt.create(&worktree_path, branch) {
                tear_down_all_overlays(&project, root, &written_overlays, true);
                for ov in &written_overlays {
                    let _ = std::fs::remove_file(ov);
                }
                return Err(e);
            }
        }

        if !reuse_worktree && !no_inherit_env && !config.inherit_env.is_empty() {
            log.step("Symlinking inherited env files...");
            crate::worktree::symlink_env_files(root, &worktree_path, &config.inherit_env, log)?;
        }

        log.step("Writing .env.ecluse...");
        let env_map = env::build_env(
            slot,
            slug,
            "container",
            &indexmap::IndexMap::new(),
            &allocated_ports,
            &[],
        );
        env::write_env_file(&worktree_path, &env_map)?;

        // post_up: all containers up, full env available
        if let Some(cmd) = &config.hooks.post_up {
            log.step("Running post_up hook...");
            log.detail(cmd);
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
            services_subset: service_filter.map(|f| {
                let mut v: Vec<String> = f.iter().cloned().collect();
                v.sort();
                v
            }),
        })
    }

    fn bring_down(
        &self,
        session: &Session,
        config: &Config,
        root: &Path,
        keep_volumes: bool,
        keep_worktree: bool,
        log: &StepLogger,
    ) -> Result<()> {
        // Reconstruct env map so hooks have access to the session's ports.
        let allocated_ports: Vec<(String, u16)> = session
            .port_overrides
            .iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect();
        let env_map = env::build_env(
            session.slot,
            &session.slug,
            "container",
            &indexmap::IndexMap::new(),
            &allocated_ports,
            &[],
        );

        // pre_down: before containers are stopped — app can flush/drain
        if let Some(cmd) = &config.hooks.pre_down {
            log.step("Running pre_down hook...");
            log.detail(cmd);
            hooks::run(cmd, std::path::Path::new(&session.worktree_path), &env_map)?;
        }

        if let Some(project) = &session.compose_project {
            let all_overlays: Vec<String> = session
                .overlay_file
                .iter()
                .cloned()
                .chain(session.overlay_files.iter().cloned())
                .collect();

            if !all_overlays.is_empty() {
                log.step("Stopping docker services...");
            }

            tear_down_all_overlays(project, root, &all_overlays, !keep_volumes);

            for ov in &all_overlays {
                let _ = std::fs::remove_file(ov);
            }
        }

        if !keep_worktree {
            log.step("Removing worktree...");
            log.detail(&session.worktree_path);
            let wt = WorktreeManager::new(root.to_owned());
            let wt_path = std::path::PathBuf::from(&session.worktree_path);
            wt.remove(&wt_path)?;
        }

        // post_down: everything torn down, worktree may no longer exist
        if let Some(cmd) = &config.hooks.post_down {
            log.step("Running post_down hook...");
            log.detail(cmd);
            hooks::run(cmd, root, &env_map)?;
        }

        Ok(())
    }
}
