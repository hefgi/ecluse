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
use crate::worktree::WorktreeManager;

use super::{tear_down_all_overlays, BringUpRequest, DockerStartup};

pub struct ContainerMode;

/// Bring up the whole root compose file (no `[[services]]` declared): every
/// service in it is started under the session's project with offset ports.
fn start_whole_compose_file(
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

    let all_svc_names: Vec<String> = compose_data.services.keys().cloned().collect();
    log.step(&format!(
        "Starting docker services: {}...",
        all_svc_names.join(", ")
    ));

    let overlay_path = overlay_dir.join(format!("{}.yml", req.slug));
    let yaml = compose::generate_overlay(
        &compose_data,
        req.slot as u16,
        &project,
        None,
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

    docker::compose_up(
        &project,
        &compose_str,
        Some(&overlay_str),
        req.watch,
        &std::collections::HashMap::new(),
    )?;
    {
        let (p, c, o) = (project.clone(), compose_str.clone(), overlay_str.clone());
        rollback.push(move || {
            let _ = docker::compose_down(&p, &c, Some(&o), rollback_volumes);
        });
    }

    let allocated_ports = compose_data
        .services
        .iter()
        .filter_map(|(name, svc)| {
            compose::service_host_port(svc, req.slot as u16).map(|p| {
                log.detail(&format!("{name}: {p}"));
                (name.clone(), p)
            })
        })
        .collect();

    Ok(DockerStartup {
        allocated_ports,
        compose_overlays: vec![crate::state::ComposeOverlay {
            compose: compose_str,
            overlay: overlay_str.clone(),
        }],
        written_overlays: vec![overlay_str],
    })
}

impl super::ModeHandler for ContainerMode {
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
            start_whole_compose_file(req, config, root, &mut rollback, log)?
        } else {
            super::start_docker_services(req, config, root, false, &mut rollback, log)?
        };

        let worktree_path = super::ensure_worktree(req, config, root, &mut rollback, log)?;

        if !req.no_inherit_env && !config.inherit_env.is_empty() {
            log.step("Inheriting env files...");
            crate::worktree::inherit_env_files(root, &worktree_path, &config.inherit_env, log)?;
        }

        log.step("Writing .env.ecluse...");
        let env_map = env::build_env(
            req.slot,
            config.slot_stride,
            req.slug,
            "container",
            &indexmap::IndexMap::new(),
            &docker.allocated_ports,
            &docker_svcs_config,
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

        let app_port = docker.allocated_ports.first().map(|(_, p)| *p);
        let stored_port_overrides: std::collections::HashMap<String, u16> =
            docker.allocated_ports.iter().cloned().collect();

        let primary_overlay = docker.written_overlays.first().cloned();
        let extra_overlays: Vec<String> = docker.written_overlays.into_iter().skip(1).collect();

        Ok(Session {
            slug: req.slug.to_string(),
            mode: crate::config::Mode::Container,
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
            port_overrides: stored_port_overrides,
            process_manager: None,
            tmux_session: None,
            pid_files: vec![],
            log_dir: None,
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
            config.slot_stride,
            &session.slug,
            "container",
            &indexmap::IndexMap::new(),
            &allocated_ports,
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
            status: crate::state::SessionStatus::Active,
            pending_op: None,
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
