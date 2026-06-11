use anyhow::{Context, Result};
use chrono::Utc;
use std::path::Path;

use crate::compose;
use crate::config::Config;
use crate::docker;
use crate::env;
use crate::hooks;
use crate::log::StepLogger;
use crate::rollback::Rollback;
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

        // Every step below registers its undo; any early return tears down
        // exactly what was created so far, in reverse order.
        let mut rollback = Rollback::new();
        // Only delete volumes the rollback created: on resume the session's
        // existing data volumes must survive a failed re-up.
        let rollback_volumes = !reuse_worktree;

        let docker_svcs_config: Vec<_> = config
            .docker_services()
            .into_iter()
            .filter(|s| service_filter.is_none_or(|f| f.contains(&s.name)))
            .collect();

        let mut allocated_ports: Vec<(String, u16)> = vec![];
        let mut written_overlays: Vec<String> = vec![];
        let mut compose_overlays: Vec<crate::state::ComposeOverlay> = vec![];

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

                    // Build port_map: services that publish their primary base_port to the host.
                    // Services with suppress_primary_publish are excluded — their only host-side
                    // publish is via extra_port_map.
                    let mut port_map: std::collections::HashMap<String, (u16, u16)> =
                        std::collections::HashMap::new();
                    for s in svcs {
                        if s.suppress_primary_publish() {
                            if let Some(ep) = s.extra_ports.first() {
                                let hp = ep.base_port.saturating_add(slot as u16);
                                allocated_ports.push((s.name.clone(), hp));
                                log.detail(&format!("{}: {hp} (via extra_ports)", s.name));
                            }
                        } else {
                            let host_port = if let Some(&p) = port_overrides.get(&s.name) {
                                p
                            } else {
                                validate::find_free_port(config, s, slot)?
                            };
                            allocated_ports.push((s.name.clone(), host_port));
                            log.detail(&format!("{}: {host_port}", s.name));
                            port_map.insert(s.name.clone(), (host_port, s.base_port));
                        }
                    }

                    let overlay_name = overlay_name_for_compose(slug, compose_path, root);
                    let overlay_path = overlay_dir.join(&overlay_name);

                    // Build extra_port_map using container_port from ExtraPort when set.
                    let extra_port_map: std::collections::HashMap<String, Vec<(u16, u16)>> = svcs
                        .iter()
                        .filter_map(|s| {
                            let extras: Vec<(u16, u16)> = s
                                .extra_port_mappings()
                                .into_iter()
                                .map(|(host_base, cport)| {
                                    (host_base.saturating_add(slot as u16), cport)
                                })
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
                    {
                        let overlay = overlay_path.clone();
                        rollback.push(move || {
                            let _ = std::fs::remove_file(&overlay);
                        });
                    }

                    let compose_str = compose_path.to_string_lossy().to_string();
                    let overlay_str = overlay_path.to_string_lossy().to_string();

                    docker::compose_up(
                        &project,
                        &compose_str,
                        Some(&overlay_str),
                        watch,
                        &compose_env,
                    )?;
                    {
                        let (p, c, o) = (project.clone(), compose_str.clone(), overlay_str.clone());
                        rollback.push(move || {
                            let _ = docker::compose_down(&p, &c, Some(&o), rollback_volumes);
                        });
                    }

                    compose_overlays.push(crate::state::ComposeOverlay {
                        compose: compose_str,
                        overlay: overlay_str.clone(),
                    });
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
            {
                let overlay = overlay_path.clone();
                rollback.push(move || {
                    let _ = std::fs::remove_file(&overlay);
                });
            }

            let compose_str = compose_path.to_string_lossy().to_string();
            let overlay_str = overlay_path.to_string_lossy().to_string();

            docker::compose_up(
                &project,
                &compose_str,
                Some(&overlay_str),
                watch,
                &std::collections::HashMap::new(),
            )?;
            {
                let (p, c, o) = (project.clone(), compose_str.clone(), overlay_str.clone());
                rollback.push(move || {
                    let _ = docker::compose_down(&p, &c, Some(&o), rollback_volumes);
                });
            }

            compose_overlays.push(crate::state::ComposeOverlay {
                compose: compose_str,
                overlay: overlay_str.clone(),
            });

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
            wt.create(&worktree_path, branch)?;
            {
                let root_owned = root.to_owned();
                let wt_path = worktree_path.clone();
                rollback.push(move || {
                    let _ = WorktreeManager::new(root_owned).remove(&wt_path);
                });
            }
        }

        if !no_inherit_env && !config.inherit_env.is_empty() {
            log.step("Inheriting env files...");
            crate::worktree::inherit_env_files(root, &worktree_path, &config.inherit_env, log)?;
        }

        log.step("Writing .env.ecluse...");
        let docker_svcs_ref: Vec<&crate::config::ServiceConfig> = docker_svcs_config.to_vec();
        let env_map = env::build_env(
            slot,
            slug,
            "container",
            &indexmap::IndexMap::new(),
            &allocated_ports,
            &[],
            &docker_svcs_ref,
        );
        env::write_env_file(&worktree_path, &env_map)?;

        // pre_spawn: env is written, containers already up — use for derived env (URLs etc.)
        if let Some(cmd) = &config.hooks.pre_spawn {
            log.step("Running pre_spawn hook...");
            log.detail(cmd);
            hooks::run(cmd, &worktree_path, &env_map)?;
        }

        // post_up: all containers up, full env available
        if let Some(cmd) = &config.hooks.post_up {
            log.step("Running post_up hook...");
            log.detail(cmd);
            hooks::run(cmd, &worktree_path, &env_map)?;
        }

        rollback.disarm();

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
            compose_overlays,
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
        let docker_svcs = config.docker_services();
        let docker_svcs_ref: Vec<&crate::config::ServiceConfig> = docker_svcs.to_vec();
        let env_map = env::build_env(
            session.slot,
            &session.slug,
            "container",
            &indexmap::IndexMap::new(),
            &allocated_ports,
            &[],
            &docker_svcs_ref,
        );

        // pre_down: before containers are stopped — app can flush/drain.
        // Failure is a warning, not fatal — teardown must always complete.
        if let Some(cmd) = &config.hooks.pre_down {
            log.step("Running pre_down hook...");
            log.detail(cmd);
            if let Err(e) = hooks::run(cmd, std::path::Path::new(&session.worktree_path), &env_map)
            {
                log.warn(&format!("pre_down hook failed (continuing teardown): {e}"));
            }
        }

        if let Some(project) = &session.compose_project {
            if !session.compose_overlays.is_empty() {
                log.step("Stopping docker services...");
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

                if !all_overlays.is_empty() {
                    log.step("Stopping docker services...");
                }

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{HookConfig, Mode};
    use crate::modes::ModeHandler;
    use crate::state::{ComposeOverlay, Session};
    use tempfile::TempDir;

    // Teardown must use the recorded (compose, overlay) pairs — including for
    // a hyphenated slug whose suffix matches a real subdirectory, where the
    // legacy filename parser would target the wrong compose file.
    #[test]
    fn bring_down_uses_recorded_pairs_and_removes_overlays() {
        let dir = TempDir::new().unwrap();
        let overlays = dir.path().join(".ecluse/overlays");
        std::fs::create_dir_all(&overlays).unwrap();
        std::fs::create_dir_all(dir.path().join("worker")).unwrap();
        std::fs::write(
            dir.path().join("worker/docker-compose.yml"),
            "services: {}\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("docker-compose.yml"), "services: {}\n").unwrap();
        let root_overlay = overlays.join("feat-worker.yml");
        std::fs::write(&root_overlay, "services: {}\n").unwrap();

        let session = Session {
            slug: "feat-worker".into(),
            mode: Mode::Container,
            slot: 1,
            branch: "feat-worker".into(),
            worktree_path: dir.path().join("wt").display().to_string(),
            compose_project: Some("ecluse_feat-worker".into()),
            overlay_file: Some(root_overlay.display().to_string()),
            overlay_files: vec![],
            compose_overlays: vec![ComposeOverlay {
                compose: dir.path().join("docker-compose.yml").display().to_string(),
                overlay: root_overlay.display().to_string(),
            }],
            app_port: None,
            started_at: "2026-01-01T00:00:00Z".into(),
            port_overrides: std::collections::HashMap::new(),
            process_manager: None,
            tmux_session: None,
            pid_files: vec![],
            log_dir: None,
            services_subset: None,
        };
        let config = Config {
            mode: Mode::Container,
            max_slots: 8,
            prefix: "ecluse".into(),
            worktree_dir: ".ecluse/worktrees".into(),
            app_label: "ecluse.role".into(),
            app_label_value: "app".into(),
            strict_port: false,
            port_search_range: 10,
            slot_stride: 1,
            services: vec![],
            hooks: HookConfig::default(),
            inherit_env: vec![],
        };
        let log = crate::log::StepLogger::new(true);

        // keep_worktree=true: no git interaction; compose_down is best-effort
        // and ignored when no docker daemon is available.
        ContainerMode
            .bring_down(&session, &config, dir.path(), true, true, &log)
            .unwrap();

        assert!(
            !root_overlay.exists(),
            "overlay from the recorded pair must be removed"
        );
    }
}
