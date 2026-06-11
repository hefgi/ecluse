use anyhow::Result;
use chrono::Utc;
use std::path::Path;

use crate::config::Config;
use crate::env;
use crate::hooks;
use crate::log::StepLogger;
use crate::process;
use crate::rollback::Rollback;
use crate::state::Session;
use crate::worktree::WorktreeManager;

use super::BringUpRequest;

pub struct HostMode;

impl super::ModeHandler for HostMode {
    fn bring_up(
        &self,
        req: &BringUpRequest,
        config: &Config,
        root: &Path,
        log: &StepLogger,
    ) -> Result<Session> {
        let native_svcs: Vec<_> = config
            .native_services()
            .into_iter()
            .filter(|s| req.service_filter.is_none_or(|f| f.contains(&s.name)))
            .collect();

        // pre_up: before anything exists — runs from repo root, no env vars yet
        if let Some(cmd) = &config.hooks.pre_up {
            log.step("Running pre_up hook...");
            log.detail(cmd);
            hooks::run(cmd, root, &std::collections::HashMap::new())?;
        }

        // Every step below registers its undo; any early return tears down
        // exactly what was created so far, in reverse order.
        let mut rollback = Rollback::new();

        log.step("Allocating ports...");
        // Historical quirk kept on purpose: host mode allocates ports for all
        // native services even when --services filters which ones spawn.
        let native_ports = super::native_ports_for_slot(
            config,
            req.slot,
            req.port_overrides,
            req.skip_services,
            req.existing_port_overrides,
            None,
        )?;
        for (name, port) in &native_ports {
            log.detail(&format!("{name}: {port}"));
        }

        let worktree_path = super::ensure_worktree(req, config, root, &mut rollback, log)?;

        if !req.no_inherit_env && !config.inherit_env.is_empty() {
            log.step("Inheriting env files...");
            crate::worktree::inherit_env_files(root, &worktree_path, &config.inherit_env, log)?;
        }

        log.step("Writing .env.ecluse...");
        let env_map = env::build_env(
            req.slot,
            req.slug,
            "host",
            &native_ports,
            &[],
            &native_svcs,
            &[],
        );
        env::write_env_file(&worktree_path, &env_map)?;

        // pre_spawn: env is written, services not yet started — use for derived env (URLs etc.)
        if let Some(cmd) = &config.hooks.pre_spawn {
            log.step("Running pre_spawn hook...");
            log.detail(cmd);
            hooks::run(cmd, &worktree_path, &env_map)?;
        }

        let (spawn, used_pm) = super::spawn_native_services(
            req,
            &native_svcs,
            &native_ports,
            &worktree_path,
            &env_map,
            &mut rollback,
            log,
        )?;

        // post_up: all services spawned, full env available
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
        let stored_port_overrides: std::collections::HashMap<String, u16> =
            native_ports.iter().map(|(k, v)| (k.clone(), *v)).collect();

        Ok(Session {
            slug: req.slug.to_string(),
            mode: crate::config::Mode::Host,
            slot: req.slot,
            branch: req.branch.to_string(),
            worktree_path: worktree_path.display().to_string(),
            status: crate::state::SessionStatus::Active,
            pending_op: None,
            compose_project: None,
            overlay_file: None,
            overlay_files: vec![],
            compose_overlays: vec![],
            app_port,
            started_at: Utc::now().to_rfc3339(),
            port_overrides: stored_port_overrides,
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
        _keep_volumes: bool,
        keep_worktree: bool,
        log: &StepLogger,
    ) -> Result<()> {
        let native_ports = super::native_ports_for_slot(
            config,
            session.slot,
            &session.port_overrides,
            &std::collections::HashSet::new(),
            &std::collections::HashMap::new(),
            None,
        )?;
        let native_svcs = config.native_services();
        let env_map = env::build_env(
            session.slot,
            &session.slug,
            "host",
            &native_ports,
            &[],
            &native_svcs,
            &[],
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
    use std::process::Command;
    use tempfile::TempDir;

    fn setup_git_repo(dir: &Path) {
        Command::new("git")
            .args(["init"])
            .current_dir(dir)
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "--allow-empty", "-m", "init"])
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "test")
            .env("GIT_AUTHOR_EMAIL", "test@test.com")
            .env("GIT_COMMITTER_NAME", "test")
            .env("GIT_COMMITTER_EMAIL", "test@test.com")
            .output()
            .unwrap();
    }

    fn make_config() -> Config {
        Config {
            mode: Mode::Host,
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
        }
    }

    fn bring_up(config: &Config, root: &Path, slug: &str, reuse: bool) -> Result<Session> {
        let log = crate::log::StepLogger::new(true);
        let port_overrides = std::collections::HashMap::new();
        let skip = std::collections::HashSet::new();
        let existing = std::collections::HashMap::new();
        let req = BringUpRequest {
            slug,
            slot: 1,
            branch: slug,
            watch: false,
            reuse_worktree: reuse,
            no_inherit_env: true,
            worktree_override: None,
            port_overrides: &port_overrides,
            service_filter: None,
            skip_services: &skip,
            existing_port_overrides: &existing,
        };
        HostMode.bring_up(&req, config, root, &log)
    }

    #[test]
    fn failed_post_up_hook_rolls_back_fresh_worktree() {
        let dir = TempDir::new().unwrap();
        setup_git_repo(dir.path());
        let mut config = make_config();
        config.hooks.post_up = Some("false".into());

        let result = bring_up(&config, dir.path(), "rb-post", false);
        assert!(result.is_err());
        assert!(
            !dir.path().join(".ecluse/worktrees/rb-post").exists(),
            "fresh worktree must be removed when post_up fails"
        );
    }

    // pre_spawn failure previously left the worktree behind (no manual cleanup
    // at that site) — the rollback guard must cover it like every other step.
    #[test]
    fn failed_pre_spawn_hook_rolls_back_fresh_worktree() {
        let dir = TempDir::new().unwrap();
        setup_git_repo(dir.path());
        let mut config = make_config();
        config.hooks.pre_spawn = Some("false".into());

        let result = bring_up(&config, dir.path(), "rb-spawn", false);
        assert!(result.is_err());
        assert!(
            !dir.path().join(".ecluse/worktrees/rb-spawn").exists(),
            "fresh worktree must be removed when pre_spawn fails"
        );
    }

    #[test]
    fn failed_post_up_hook_keeps_reused_worktree() {
        let dir = TempDir::new().unwrap();
        setup_git_repo(dir.path());
        let wt = WorktreeManager::new(dir.path().to_owned());
        let path = dir.path().join(".ecluse/worktrees/rb-reuse");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        wt.create(&path, "rb-reuse").unwrap();

        let mut config = make_config();
        config.hooks.post_up = Some("false".into());

        let result = bring_up(&config, dir.path(), "rb-reuse", true);
        assert!(result.is_err());
        assert!(path.exists(), "reused worktree must survive rollback");
    }

    #[test]
    fn successful_bring_up_keeps_worktree() {
        let dir = TempDir::new().unwrap();
        setup_git_repo(dir.path());
        let config = make_config();

        let session = bring_up(&config, dir.path(), "rb-ok", false).unwrap();
        assert!(
            dir.path().join(".ecluse/worktrees/rb-ok").exists(),
            "disarmed rollback must not remove anything"
        );
        assert_eq!(session.slug, "rb-ok");
    }
}
