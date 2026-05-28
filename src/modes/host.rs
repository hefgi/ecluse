use anyhow::Result;
use chrono::Utc;
use indexmap::IndexMap;
use std::path::Path;

use crate::config::Config;
use crate::env;
use crate::hooks;
use crate::log::StepLogger;
use crate::process;
use crate::state::Session;
use crate::validate;
use crate::worktree::WorktreeManager;

pub struct HostMode;

impl super::ModeHandler for HostMode {
    fn bring_up(
        &self,
        slug: &str,
        slot: u8,
        branch: &str,
        config: &Config,
        root: &Path,
        _watch: bool,
        reuse_worktree: bool,
        port_overrides: &std::collections::HashMap<String, u16>,
        service_filter: Option<&std::collections::HashSet<String>>,
        skip_services: &std::collections::HashSet<String>,
        existing_port_overrides: &std::collections::HashMap<String, u16>,
        log: &StepLogger,
    ) -> Result<Session> {
        let wt = WorktreeManager::new(root.to_owned());
        let worktree_path = wt.worktree_path(config, slug);
        let native_svcs: Vec<_> = config
            .native_services()
            .into_iter()
            .filter(|s| service_filter.is_none_or(|f| f.contains(&s.name)))
            .collect();

        // pre_up: before anything exists — runs from repo root, no env vars yet
        if let Some(cmd) = &config.hooks.pre_up {
            log.step("Running pre_up hook...");
            log.detail(cmd);
            hooks::run(cmd, root, &std::collections::HashMap::new())?;
        }

        log.step("Allocating ports...");
        let native_ports = native_ports_for_slot(
            config,
            slot,
            port_overrides,
            skip_services,
            existing_port_overrides,
        )?;
        for (name, port) in &native_ports {
            log.detail(&format!("{name}: {port}"));
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
        }

        log.step("Writing .env.ecluse...");
        let env_map = env::build_env(slot, slug, "host", &native_ports, &[], &native_svcs);
        env::write_env_file(&worktree_path, &env_map)?;

        let global = process::load_global_config()?;

        let svcs_to_spawn: Vec<_> = native_svcs
            .iter()
            .filter(|s| !skip_services.contains(&s.name))
            .copied()
            .collect();

        let spawn = if svcs_to_spawn.iter().any(|s| s.command.is_some()) {
            log.step(&format!(
                "Spawning native services ({})...",
                global.process_manager
            ));
            for svc in &svcs_to_spawn {
                if let Some(cmd) = &svc.command {
                    let port = native_ports.get(&svc.name).copied().unwrap_or(0);
                    log.detail(&format!("{} on port {} — {}", svc.name, port, cmd));
                }
            }
            process::spawn_services(
                &global.process_manager,
                slug,
                &svcs_to_spawn,
                &worktree_path,
                &env_map,
            )?
        } else {
            process::spawn_services(
                &global.process_manager,
                slug,
                &svcs_to_spawn,
                &worktree_path,
                &env_map,
            )?
        };

        // post_up: all services spawned, full env available
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
                if !reuse_worktree {
                    let _ = wt.remove(&worktree_path);
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
        let stored_port_overrides: std::collections::HashMap<String, u16> =
            native_ports.iter().map(|(k, v)| (k.clone(), *v)).collect();

        Ok(Session {
            slug: slug.to_string(),
            mode: crate::config::Mode::Host,
            slot,
            branch: branch.to_string(),
            worktree_path: worktree_path.display().to_string(),
            compose_project: None,
            overlay_file: None,
            overlay_files: vec![],
            app_port,
            started_at: Utc::now().to_rfc3339(),
            port_overrides: stored_port_overrides,
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
        _keep_volumes: bool,
        keep_worktree: bool,
        log: &StepLogger,
    ) -> Result<()> {
        let native_ports = native_ports_for_slot(
            config,
            session.slot,
            &session.port_overrides,
            &std::collections::HashSet::new(),
            &std::collections::HashMap::new(),
        )?;
        let native_svcs = config.native_services();
        let env_map = env::build_env(
            session.slot,
            &session.slug,
            "host",
            &native_ports,
            &[],
            &native_svcs,
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

/// Build the native port map for a slot, falling back to "app" on 3000+slot
/// when no [[services]] are defined. Skipped services copy their port from
/// `existing` instead of calling find_free_port.
fn native_ports_for_slot(
    config: &Config,
    slot: u8,
    overrides: &std::collections::HashMap<String, u16>,
    skip: &std::collections::HashSet<String>,
    existing: &std::collections::HashMap<String, u16>,
) -> Result<IndexMap<String, u16>> {
    let native = config.native_services();
    if native.is_empty() {
        let port = if let Some(&p) = overrides.get("app").or_else(|| existing.get("app")) {
            p
        } else {
            let fallback = crate::config::ServiceConfig {
                name: "app".into(),
                base_port: 3000,
                run: crate::config::ServiceRun::Native,
                compose: None,
                command: None,
                port_env: vec![],
            };
            validate::find_free_port(config, &fallback, slot)?
        };
        let mut m = IndexMap::new();
        m.insert("app".to_string(), port);
        Ok(m)
    } else {
        native
            .iter()
            .map(|s| {
                let port = if let Some(&p) = overrides.get(&s.name) {
                    p
                } else if skip.contains(&s.name) {
                    existing.get(&s.name).copied().ok_or_else(|| {
                        anyhow::anyhow!(
                            "service '{}' is skipped but has no recorded port; run ecluse up without --skip or provide --port {}=<value>",
                            s.name, s.name
                        )
                    })?
                } else {
                    validate::find_free_port(config, s, slot)?
                };
                Ok((s.name.clone(), port))
            })
            .collect()
    }
}
