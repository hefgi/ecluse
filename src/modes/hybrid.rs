use anyhow::{Context, Result};
use chrono::Utc;
use indexmap::IndexMap;
use std::path::Path;

use crate::compose;
use crate::config::Config;
use crate::docker;
use crate::env;
use crate::hooks;
use crate::log::StepLogger;
use crate::process;
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

        let mut allocated_docker_ports: Vec<(String, u16)> = vec![];
        let mut written_overlays: Vec<String> = vec![];

        // Copy ports for skipped docker services from existing session.
        for svc in &docker_svcs_config {
            if skip_services.contains(&svc.name) {
                if let Some(&p) = existing_port_overrides.get(&svc.name) {
                    log.detail(&format!("{}: already running — skipped", svc.name));
                    allocated_docker_ports.push((svc.name.clone(), p));
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
                            allocated_docker_ports.push((s.name.clone(), host_port));
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
                        Some(&svc_names),
                        &config.prefix,
                        slot,
                    )?;
                    std::fs::write(&overlay_path, &yaml).context("failed to write overlay file")?;

                    let compose_str = compose_path.to_string_lossy().to_string();
                    let overlay_str = overlay_path.to_string_lossy().to_string();
                    let svc_refs: Vec<&str> = svc_names.iter().map(|s| s.as_str()).collect();

                    if let Err(e) = docker::compose_up_services(
                        &project,
                        &compose_str,
                        Some(&overlay_str),
                        &svc_refs,
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

            log.step(&format!(
                "Starting docker services: {}...",
                data_svcs.join(", ")
            ));

            let overlay_path = overlay_dir.join(format!("{}.yml", slug));
            let yaml = compose::generate_overlay(
                &compose_data,
                slot as u16,
                &suffix,
                Some(&data_svcs),
                &config.prefix,
                slot,
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
                &std::collections::HashMap::new(),
            ) {
                let _ = std::fs::remove_file(&overlay_path);
                if !reuse_worktree {
                    let _ = wt.remove(&worktree_path);
                }
                return Err(e);
            }

            for (name, svc) in &compose_data.services {
                if data_svcs.contains(name) {
                    if let Some(p) = compose::service_host_port(svc, slot as u16) {
                        log.detail(&format!("{name}: {p}"));
                        allocated_docker_ports.push((name.clone(), p));
                    }
                }
            }

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

        log.step("Allocating native ports...");
        let native_svcs: Vec<_> = config
            .native_services()
            .into_iter()
            .filter(|s| service_filter.is_none_or(|f| f.contains(&s.name)))
            .collect();
        let native_ports: IndexMap<String, u16> = if native_svcs.is_empty() {
            let port = if let Some(&p) = port_overrides
                .get("app")
                .or_else(|| existing_port_overrides.get("app"))
            {
                p
            } else {
                let fallback = crate::config::ServiceConfig {
                    name: "app".into(),
                    base_port: 3000,
                    run: crate::config::ServiceRun::Native,
                    compose: None,
                    command: None,
                    port_env: vec![],
                    debug_port: None,
                    extra_ports: vec![],
                };
                validate::find_free_port(config, &fallback, slot)?
            };
            let mut m = IndexMap::new();
            m.insert("app".to_string(), port);
            log.detail(&format!("app: {port}"));
            m
        } else {
            native_svcs
                .iter()
                .map(|s| {
                    let port = if let Some(&p) = port_overrides.get(&s.name) {
                        p
                    } else if skip_services.contains(&s.name) {
                        existing_port_overrides.get(&s.name).copied().ok_or_else(|| {
                            anyhow::anyhow!(
                                "service '{}' is skipped but has no recorded port; run ecluse up without --skip or provide --port {}=<value>",
                                s.name, s.name
                            )
                        })?
                    } else {
                        validate::find_free_port(config, s, slot)?
                    };
                    log.detail(&format!("{}: {port}", s.name));
                    Ok((s.name.clone(), port))
                })
                .collect::<Result<IndexMap<_, _>>>()?
        };

        if !reuse_worktree && !no_inherit_env && !config.inherit_env.is_empty() {
            log.step("Symlinking inherited env files...");
            crate::worktree::symlink_env_files(root, &worktree_path, &config.inherit_env, log)?;
        }

        log.step("Writing .env.ecluse...");
        let env_map = env::build_env(
            slot,
            slug,
            "hybrid",
            &native_ports,
            &allocated_docker_ports,
            &native_svcs,
        );
        env::write_env_file(&worktree_path, &env_map)?;

        let global = process::load_global_config()?;

        let native_svcs_to_spawn: Vec<_> = native_svcs
            .iter()
            .filter(|s| !skip_services.contains(&s.name))
            .copied()
            .collect();

        let spawn = if native_svcs_to_spawn.iter().any(|s| s.command.is_some()) {
            log.step(&format!(
                "Spawning native services ({})...",
                global.process_manager
            ));
            for svc in &native_svcs_to_spawn {
                if let Some(cmd) = &svc.command {
                    let port = native_ports.get(&svc.name).copied().unwrap_or(0);
                    log.detail(&format!("{} on port {} — {}", svc.name, port, cmd));
                }
            }
            process::spawn_services(
                &global.process_manager,
                slug,
                &native_svcs_to_spawn,
                &worktree_path,
                &env_map,
            )?
        } else {
            process::spawn_services(
                &global.process_manager,
                slug,
                &native_svcs_to_spawn,
                &worktree_path,
                &env_map,
            )?
        };

        // post_up: all services up and spawned, full env available
        if let Some(cmd) = &config.hooks.post_up {
            log.step("Running post_up hook...");
            log.detail(cmd);
            if let Err(e) = hooks::run(cmd, &worktree_path, &env_map) {
                let pm = if spawn.tmux_session.is_some() || !spawn.pid_files.is_empty() {
                    Some(&global.process_manager)
                } else {
                    None
                };
                if let Some(pm) = pm {
                    process::kill_services(pm, &spawn);
                }
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

        let pm = if spawn.tmux_session.is_some() || !spawn.pid_files.is_empty() {
            Some(global.process_manager)
        } else {
            None
        };

        let app_port = native_ports.values().next().copied();

        let mut all_ports: std::collections::HashMap<String, u16> =
            native_ports.iter().map(|(k, v)| (k.clone(), *v)).collect();
        for (name, port) in &allocated_docker_ports {
            all_ports.insert(name.clone(), *port);
        }

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
            process_manager: pm,
            tmux_session: spawn.tmux_session,
            pid_files: spawn.pid_files,
            log_dir: spawn.log_dir,
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
        let native = config.native_services();
        // Reconstruct native ports from the persisted port_overrides so pre/post-down
        // hooks see the same ports that were actually allocated during bring_up (which
        // may differ from nominal values when find_free_port bumped them).
        let native_names: std::collections::HashSet<&str> =
            native.iter().map(|s| s.name.as_str()).collect();
        let native_ports: IndexMap<String, u16> = if native.is_empty() {
            session
                .port_overrides
                .iter()
                .filter(|(k, _)| k.as_str() == "app")
                .map(|(k, v)| (k.clone(), *v))
                .collect()
        } else {
            session
                .port_overrides
                .iter()
                .filter(|(k, _)| native_names.contains(k.as_str()))
                .map(|(k, v)| (k.clone(), *v))
                .collect()
        };
        let allocated_docker_ports: Vec<(String, u16)> = session
            .port_overrides
            .iter()
            .filter(|(k, _)| !native_names.contains(k.as_str()) && k.as_str() != "app")
            .map(|(k, v)| (k.clone(), *v))
            .collect();
        let env_map = env::build_env(
            session.slot,
            &session.slug,
            "hybrid",
            &native_ports,
            &allocated_docker_ports,
            &native,
        );

        // pre_down: before services are killed — app can drain/flush
        if let Some(cmd) = &config.hooks.pre_down {
            log.step("Running pre_down hook...");
            log.detail(cmd);
            hooks::run(cmd, std::path::Path::new(&session.worktree_path), &env_map)?;
        }

        if let Some(pm) = &session.process_manager {
            log.step(&format!("Killing native services ({pm})..."));
            process::kill_services(pm, &session.spawn_result());
        }

        if let Some(project) = &session.compose_project {
            let all_overlays: Vec<String> = session
                .overlay_file
                .iter()
                .cloned()
                .chain(session.overlay_files.iter().cloned())
                .collect();

            log.step("Stopping docker services...");

            if all_overlays.is_empty() {
                // No overlay paths recorded in state — fall back to the root compose file
                // so containers are always stopped even if state was written without overlays.
                if let Some(cp) = compose::find_compose_file(root) {
                    let _ = crate::docker::compose_down(
                        project,
                        &cp.to_string_lossy(),
                        None,
                        !keep_volumes,
                    );
                }
            } else {
                tear_down_all_overlays(project, root, &all_overlays, !keep_volumes);
                for ov in &all_overlays {
                    let _ = std::fs::remove_file(ov);
                }
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
