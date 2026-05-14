use anyhow::Result;
use chrono::Utc;
use indexmap::IndexMap;
use std::path::Path;

use crate::config::Config;
use crate::env;
use crate::hooks;
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
    ) -> Result<Session> {
        let wt = WorktreeManager::new(root.to_owned());
        let worktree_path = wt.worktree_path(config, slug);

        let native_ports = native_ports_for_slot(config, slot, port_overrides)?;

        if reuse_worktree {
            if !worktree_path.exists() {
                return Err(anyhow::anyhow!(
                    "worktree not found at {}; remove --reuse-worktree or run ecluse up without it",
                    worktree_path.display()
                ));
            }
        } else {
            wt.create(&worktree_path, branch)?;
        }

        let env_map = env::build_env(slot, slug, "host", &native_ports, &[]);
        env::write_env_file(&worktree_path, &env_map)?;

        if let Some(cmd) = &config.hooks.on_up {
            if let Err(e) = hooks::run(cmd, &worktree_path, &env_map) {
                if !reuse_worktree {
                    let _ = wt.remove(&worktree_path);
                }
                return Err(e);
            }
        }

        let global = process::load_global_config()?;
        let native_svcs = config.native_services();
        let spawn = process::spawn_services(
            &global.process_manager,
            slug,
            &native_svcs,
            &worktree_path,
            &env_map,
        )?;

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
        })
    }

    fn bring_down(
        &self,
        session: &Session,
        config: &Config,
        root: &Path,
        _keep_volumes: bool,
        keep_worktree: bool,
    ) -> Result<()> {
        if let Some(cmd) = &config.hooks.on_down {
            let native_ports =
                native_ports_for_slot(config, session.slot, &session.port_overrides)?;
            let env_map = env::build_env(session.slot, &session.slug, "host", &native_ports, &[]);
            hooks::run(cmd, std::path::Path::new(&session.worktree_path), &env_map)?;
        }

        if let Some(pm) = &session.process_manager {
            process::kill_services(pm, &session.spawn_result());
        }

        if !keep_worktree {
            let wt = WorktreeManager::new(root.to_owned());
            let wt_path = std::path::PathBuf::from(&session.worktree_path);
            wt.remove(&wt_path)?;
        }

        Ok(())
    }
}

/// Build the native port map for a slot, falling back to "app" on 3000+slot
/// when no [[services]] are defined. Finds a free port for each service,
/// unless a port is explicitly overridden via `overrides`.
fn native_ports_for_slot(
    config: &Config,
    slot: u8,
    overrides: &std::collections::HashMap<String, u16>,
) -> Result<IndexMap<String, u16>> {
    let native = config.native_services();
    if native.is_empty() {
        let port = if let Some(&p) = overrides.get("app") {
            p
        } else {
            let fallback = crate::config::ServiceConfig {
                name: "app".into(),
                base_port: 3000,
                run: crate::config::ServiceRun::Native,
                compose: None,
                command: None,
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
                } else {
                    validate::find_free_port(config, s, slot)?
                };
                Ok((s.name.clone(), port))
            })
            .collect()
    }
}
