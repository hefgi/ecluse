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
use crate::rollback::Rollback;
use crate::state::Session;
use crate::worktree::WorktreeManager;

use super::{tear_down_all_overlays, BringUpRequest, DockerStartup};

pub struct HybridMode;

/// Fallback when no docker `[[services]]` are declared: start every service in
/// the root compose file that is NOT labeled as the app (label-based split).
fn start_labeled_data_services(
    req: &BringUpRequest,
    config: &Config,
    root: &Path,
    rollback: &mut Rollback,
    log: &StepLogger,
) -> Result<DockerStartup> {
    let project = super::compose_project_name(config, req.slug);
    let overlay_dir = root.join(".ecluse").join("overlays");
    std::fs::create_dir_all(&overlay_dir).context("failed to create overlays directory")?;
    let rollback_volumes = !req.reuse_worktree;

    let compose_path = compose::find_compose_file(root).ok_or_else(|| {
        crate::error::EcluseError::ComposeFileNotFound(root.display().to_string())
    })?;
    let compose_data = compose::parse(&compose_path)?;

    let app_svcs = compose::app_services(&compose_data, &config.app_label, &config.app_label_value);
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

    let overlay_path = overlay_dir.join(format!("{}.yml", req.slug));
    let yaml = compose::generate_overlay(
        &compose_data,
        req.slot as u16,
        &project,
        Some(&data_svcs),
        &config.prefix,
        req.slot,
    )?;
    std::fs::write(&overlay_path, &yaml).context("failed to write overlay file")?;
    {
        let overlay = overlay_path.clone();
        rollback.push(move || {
            let _ = std::fs::remove_file(&overlay);
        });
    }

    let compose_str = compose_path.to_string_lossy().to_string();
    let overlay_str = overlay_path.to_string_lossy().to_string();
    let data_refs: Vec<&str> = data_svcs.iter().map(|s| s.as_str()).collect();

    docker::compose_up_services(
        &project,
        &compose_str,
        Some(&overlay_str),
        &data_refs,
        req.watch,
        &std::collections::HashMap::new(),
    )?;
    {
        let (p, c, o) = (project.clone(), compose_str.clone(), overlay_str.clone());
        rollback.push(move || {
            let _ = docker::compose_down(&p, &c, Some(&o), rollback_volumes);
        });
    }

    let mut allocated_ports = vec![];
    for (name, svc) in &compose_data.services {
        if data_svcs.contains(name) {
            if let Some(p) = compose::service_host_port(svc, req.slot as u16) {
                log.detail(&format!("{name}: {p}"));
                allocated_ports.push((name.clone(), p));
            }
        }
    }

    Ok(DockerStartup {
        allocated_ports,
        compose_overlays: vec![crate::state::ComposeOverlay {
            compose: compose_str,
            overlay: overlay_str.clone(),
        }],
        written_overlays: vec![overlay_str],
    })
}

impl super::ModeHandler for HybridMode {
    fn bring_up(
        &self,
        req: &BringUpRequest,
        config: &Config,
        root: &Path,
        log: &StepLogger,
    ) -> Result<Session> {
        // pre_up: before anything exists — runs from repo root, no env vars yet
        if let Some(cmd) = &config.hooks.pre_up {
            log.step("Running pre_up hook...");
            log.detail(cmd);
            hooks::run(cmd, root, &std::collections::HashMap::new())?;
        }

        // Every step below registers its undo; any early return tears down
        // exactly what was created so far, in reverse order.
        let mut rollback = Rollback::new();

        let docker_svcs_config = super::filtered_docker_services(config, req);
        let docker = if docker_svcs_config.is_empty() {
            start_labeled_data_services(req, config, root, &mut rollback, log)?
        } else {
            super::start_docker_services(req, config, root, true, &mut rollback, log)?
        };

        let worktree_path = super::ensure_worktree(req, config, root, &mut rollback, log)?;

        log.step("Allocating native ports...");
        let native_svcs: Vec<_> = config
            .native_services()
            .into_iter()
            .filter(|s| req.service_filter.is_none_or(|f| f.contains(&s.name)))
            .collect();
        let native_ports: IndexMap<String, u16> = super::native_ports_for_slot(
            config,
            req.slot,
            req.port_overrides,
            req.skip_services,
            req.existing_port_overrides,
            req.service_filter,
        )?;
        for (name, port) in &native_ports {
            log.detail(&format!("{name}: {port}"));
        }

        if !req.no_inherit_env && !config.inherit_env.is_empty() {
            log.step("Inheriting env files...");
            crate::worktree::inherit_env_files(root, &worktree_path, &config.inherit_env, log)?;
        }

        log.step("Writing .env.ecluse...");
        let all_svc_configs: Vec<&crate::config::ServiceConfig> = native_svcs
            .iter()
            .chain(docker_svcs_config.iter())
            .copied()
            .collect();
        let env_map = env::build_env(
            req.slot,
            config.slot_stride,
            req.slug,
            "hybrid",
            &native_ports,
            &docker.allocated_ports,
            &all_svc_configs,
        );
        env::write_env_file(&worktree_path, &env_map)?;

        // pre_spawn: env is written, native services not yet started — use for derived env (URLs etc.)
        if let Some(cmd) = &config.hooks.pre_spawn {
            log.step("Running pre_spawn hook...");
            log.detail(cmd);
            hooks::run(cmd, &worktree_path, &env_map)?;
        }

        super::check_extra_ports(config, &native_svcs, req.skip_services, req.slot, log)?;

        let (spawn, used_pm) = super::spawn_native_services(
            req,
            &native_svcs,
            &native_ports,
            &worktree_path,
            &env_map,
            &mut rollback,
            log,
        )?;

        // post_up: all services up and spawned, full env available
        if let Some(cmd) = &config.hooks.post_up {
            log.step("Running post_up hook...");
            log.detail(cmd);
            hooks::run(cmd, &worktree_path, &env_map)?;
        }

        rollback.disarm();

        let pm = if spawn.tmux_session.is_some() || !spawn.pid_files.is_empty() {
            Some(used_pm)
        } else {
            None
        };

        let app_port = native_ports.values().next().copied();

        let mut all_ports: std::collections::HashMap<String, u16> =
            native_ports.iter().map(|(k, v)| (k.clone(), *v)).collect();
        for (name, port) in &docker.allocated_ports {
            all_ports.insert(name.clone(), *port);
        }

        let primary_overlay = docker.written_overlays.first().cloned();
        let extra_overlays: Vec<String> = docker.written_overlays.into_iter().skip(1).collect();

        Ok(Session {
            slug: req.slug.to_string(),
            mode: crate::config::Mode::Hybrid,
            slot: req.slot,
            branch: req.branch.to_string(),
            worktree_path: worktree_path.display().to_string(),
            status: crate::state::SessionStatus::Active,
            pending_op: None,
            compose_project: Some(super::compose_project_name(config, req.slug)),
            overlay_file: primary_overlay,
            overlay_files: extra_overlays,
            compose_overlays: docker.compose_overlays,
            app_port,
            started_at: Utc::now().to_rfc3339(),
            port_overrides: all_ports,
            process_manager: pm,
            tmux_session: spawn.tmux_session,
            pid_files: spawn.pid_files,
            log_dir: spawn.log_dir,
            services_subset: req.service_filter.map(|f| {
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
        let all_svc_configs: Vec<&crate::config::ServiceConfig> = native
            .iter()
            .chain(config.docker_services().iter())
            .copied()
            .collect();
        let env_map = env::build_env(
            session.slot,
            config.slot_stride,
            &session.slug,
            "hybrid",
            &native_ports,
            &allocated_docker_ports,
            &all_svc_configs,
        );

        // pre_down: before services are killed — app can drain/flush.
        // Failure is a warning, not fatal — teardown must always complete.
        if let Some(cmd) = &config.hooks.pre_down {
            log.step("Running pre_down hook...");
            log.detail(cmd);
            if let Err(e) = hooks::run(cmd, std::path::Path::new(&session.worktree_path), &env_map)
            {
                log.warn(&format!("pre_down hook failed (continuing teardown): {e}"));
            }
        }

        if let Some(pm) = &session.process_manager {
            log.step(&format!("Killing native services ({pm})..."));
            process::kill_services(pm, &session.spawn_result());
        }
        process::remove_env_preamble(std::path::Path::new(&session.worktree_path), &session.slug);

        if let Some(project) = &session.compose_project {
            log.step("Stopping docker services...");

            if !session.compose_overlays.is_empty() {
                for pair in &session.compose_overlays {
                    let _ = docker::compose_down(
                        project,
                        &pair.compose,
                        Some(&pair.overlay),
                        !keep_volumes,
                    );
                    let _ = std::fs::remove_file(&pair.overlay);
                }
            } else {
                // Legacy state without compose_overlays: reconstruct compose
                // paths from overlay filenames.
                let all_overlays: Vec<String> = session
                    .overlay_file
                    .iter()
                    .cloned()
                    .chain(session.overlay_files.iter().cloned())
                    .collect();

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
        }

        if !keep_worktree {
            log.step("Removing worktree...");
            log.detail(&session.worktree_path);
            let wt = WorktreeManager::new(root.to_owned());
            let wt_path = std::path::PathBuf::from(&session.worktree_path);
            wt.remove(&wt_path)?;
        }

        // post_down: everything torn down, worktree may no longer exist.
        // Failure is a warning, not fatal.
        if let Some(cmd) = &config.hooks.post_down {
            log.step("Running post_down hook...");
            log.detail(cmd);
            if let Err(e) = hooks::run(cmd, root, &env_map) {
                log.warn(&format!("post_down hook failed: {e}"));
            }
        }

        Ok(())
    }
}
