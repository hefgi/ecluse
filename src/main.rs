mod cli;
mod compose;
mod config;
mod detect;
mod discover;
mod docker;
mod env;
mod error;
mod hooks;
mod log;
mod modes;
mod process;
mod rollback;
mod slot;
mod state;
mod sync;
mod validate;
mod whose_pid;
mod worktree;

use anyhow::{Context, Result};
use clap::Parser;
use std::io::{self, Write};
use std::path::Path;
use tabled::{Table, Tabled};

fn main() {
    let cli = cli::Cli::parse();

    let level = if cli.verbose { "debug" } else { "info" };
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| level.into()),
        )
        .with_writer(std::io::stderr)
        .init();

    if let Err(e) = run(cli) {
        eprintln!("error: {}", e);
        std::process::exit(1);
    }
}

fn run(cli: cli::Cli) -> Result<()> {
    match cli.command {
        cli::Command::Init(args) => cmd_init(args),
        cli::Command::Up(args) => cmd_up(args),
        cli::Command::Down(args) => cmd_down(args),
        cli::Command::Ls(args) => cmd_ls(args),
        cli::Command::Shell(args) => cmd_shell(args),
        cli::Command::Env(args) => cmd_env(args),
        cli::Command::Validate(args) => cmd_validate(args),
        cli::Command::Shutdown(args) => cmd_shutdown(args),
        cli::Command::Sync(args) => cmd_sync(args),
        cli::Command::Flush(args) => cmd_flush(args),
        cli::Command::Status(args) => cmd_status(args),
        cli::Command::WhosePid(args) => cmd_whose_pid(args),
    }
}

// ── whose-pid ────────────────────────────────────────────────────────────────

fn cmd_whose_pid(args: cli::WhosePidArgs) -> Result<()> {
    let (_, root) = config::Config::find_and_load()?;
    let guard = state::StateGuard::acquire_shared(&root)?;
    let owner = whose_pid::resolve(&root, &guard.state.sessions, args.pid);

    if args.json {
        let out = match &owner {
            Some(o) => serde_json::json!({
                "pid": args.pid,
                "owned": true,
                "slug": o.slug,
                "slot": o.slot,
                "service": o.service,
                "port": o.port,
            }),
            None => serde_json::json!({
                "pid": args.pid,
                "owned": false,
            }),
        };
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        match &owner {
            Some(o) => {
                let svc = o.service.as_deref().unwrap_or("?");
                let port = o.port.map(|p| format!(", port {p}")).unwrap_or_default();
                println!(
                    "PID {} is owned by session '{}' (slot {}, service '{}'{})",
                    args.pid, o.slug, o.slot, svc, port
                );
            }
            None => {
                println!("PID {} is not owned by any ecluse session", args.pid);
            }
        }
    }

    if owner.is_none() {
        std::process::exit(1);
    }
    Ok(())
}

// ── init ─────────────────────────────────────────────────────────────────────

fn cmd_init(args: cli::InitArgs) -> Result<()> {
    let log = log::StepLogger::new(args.quiet);

    let cwd = std::env::current_dir().context("could not determine current directory")?;

    log.step("Verifying git repository...");
    worktree::WorktreeManager::verify_git_repo(&cwd)?;

    let root = worktree::WorktreeManager::main_worktree_root(&cwd)?;
    if root != cwd {
        log.detail(&format!(
            "running from a worktree — writing config to main worktree at {}",
            root.display()
        ));
    }

    let mode: config::Mode = if let Some(m) = &args.mode {
        m.parse()?
    } else {
        log.step("Detecting mode...");
        let result = detect::detect(&cwd);

        if let Some(reason) = &result.unsupported_reason {
            eprintln!("ecluse does not support this repo:\n  {}", reason);
            eprintln!("\nTo override: ecluse init --mode <container|host|hybrid>");
            std::process::exit(1);
        }

        if args.explain
            || matches!(
                result.confidence,
                detect::Confidence::Low | detect::Confidence::None
            )
        {
            detect::print_detection_result(&result);
        } else {
            match &result.recommended {
                Some(m) => log.detail(&format!("{m} ({})", result.confidence)),
                None => {
                    eprintln!("No mode could be recommended. Use --mode to specify one.");
                    detect::print_detection_result(&result);
                    std::process::exit(1);
                }
            }
        }

        match result.recommended {
            None => {
                eprintln!("Use: ecluse init --mode <container|host|hybrid>");
                std::process::exit(1);
            }
            Some(recommended) => {
                if args.yes {
                    recommended
                } else {
                    prompt_mode_confirm(recommended)?
                }
            }
        }
    };

    log.step("Determining process manager...");
    let pm = process::detect_process_manager();
    log.detail(&pm.to_string());

    let config_path = root.join(".ecluse.toml");
    if config_path.exists() && !args.yes {
        print!(".ecluse.toml already exists. Overwrite? [y/N] ");
        io::stdout().flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Aborted.");
            return Ok(());
        }
    }

    log.step("Writing .ecluse.toml...");
    let cfg = config::Config {
        mode: mode.clone(),
        max_slots: args.max_slots,
        prefix: args.prefix.clone(),
        worktree_dir: ".ecluse/worktrees".into(),
        app_label: "ecluse.role".into(),
        app_label_value: "app".into(),
        strict_port: false,
        port_search_range: 10,
        slot_stride: 1,
        services: vec![],
        hooks: config::HookConfig::default(),
        inherit_env: vec![
            config::InheritEnvEntry::symlink(".env"),
            config::InheritEnvEntry::symlink(".env.local"),
        ],
    };
    cfg.save(&root)?;
    log.detail(&format!("mode: {mode}, max_slots: {}", args.max_slots));

    let global_cfg = process::GlobalConfig {
        process_manager: pm.clone(),
    };
    match process::save_global_config(&global_cfg) {
        Ok(()) => log.detail(&format!(
            "process_manager = {pm} (written to ~/.config/ecluse/config.toml)"
        )),
        Err(e) => log.warn(&format!("could not write global config: {e}")),
    }

    let ecluse_dir = root.join(".ecluse");
    log.step(&format!(
        "Creating .ecluse/ in main worktree at {}...",
        ecluse_dir.display()
    ));
    std::fs::create_dir_all(&ecluse_dir)?;

    let gitignore_path = root.join(".gitignore");
    let should_add = if gitignore_path.exists() {
        let content = std::fs::read_to_string(&gitignore_path)?;
        !content.lines().any(|l| l.trim() == ".ecluse/")
    } else {
        true
    };

    if should_add {
        if args.yes {
            log.step("Adding .ecluse/ to .gitignore...");
            append_gitignore(&gitignore_path)?;
        } else {
            print!("Add .ecluse/ to .gitignore? [Y/n] ");
            io::stdout().flush()?;
            let mut input = String::new();
            io::stdin().read_line(&mut input)?;
            if input.trim().is_empty() || input.trim().eq_ignore_ascii_case("y") {
                log.step("Adding .ecluse/ to .gitignore...");
                append_gitignore(&gitignore_path)?;
            }
        }
    }

    println!();
    log.success(&format!(
        "Initialized ecluse in {} (mode: {mode})",
        root.display()
    ));
    println!();
    println!("Next step:  ecluse up <slug>");

    Ok(())
}

fn prompt_mode_confirm(recommended: config::Mode) -> Result<config::Mode> {
    print!("Accept {} mode? [Y/n/container/host/hybrid] ", recommended);
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let trimmed = input.trim().to_lowercase();
    match trimmed.as_str() {
        "" | "y" | "yes" => Ok(recommended),
        "n" | "no" => {
            print!("Enter mode [container/host/hybrid]: ");
            io::stdout().flush()?;
            let mut mode_input = String::new();
            io::stdin().read_line(&mut mode_input)?;
            Ok(mode_input.trim().parse()?)
        }
        other => Ok(other.parse()?),
    }
}

fn append_gitignore(path: &std::path::Path) -> Result<()> {
    use std::fs::OpenOptions;
    use std::io::Write;
    let mut f = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(f, ".ecluse/")?;
    Ok(())
}

// ── up ────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // ── validate_slug ─────────────────────────────────────────────────────────

    #[test]
    fn valid_slug_two_chars() {
        assert!(validate_slug("ab").is_ok());
    }

    #[test]
    fn valid_slug_alphanumeric_with_hyphens() {
        assert!(validate_slug("feat-my-feature").is_ok());
    }

    #[test]
    fn valid_slug_numbers_only() {
        assert!(validate_slug("12").is_ok());
    }

    #[test]
    fn valid_slug_max_length() {
        // max is 32 chars total: [a-z0-9] + up to 30 of [a-z0-9-] + [a-z0-9]
        let slug = "a".repeat(32);
        assert!(validate_slug(&slug).is_ok());
    }

    #[test]
    fn valid_slug_mixed_numbers_letters_hyphens() {
        assert!(validate_slug("fix-123-foo").is_ok());
    }

    #[test]
    fn invalid_slug_uppercase() {
        assert!(validate_slug("Feat-foo").is_err());
    }

    #[test]
    fn invalid_slug_leading_hyphen() {
        assert!(validate_slug("-feat-foo").is_err());
    }

    #[test]
    fn invalid_slug_trailing_hyphen() {
        assert!(validate_slug("feat-foo-").is_err());
    }

    #[test]
    fn invalid_slug_single_char() {
        assert!(validate_slug("a").is_err());
    }

    #[test]
    fn invalid_slug_special_chars() {
        assert!(validate_slug("feat_foo").is_err());
        assert!(validate_slug("feat.foo").is_err());
        assert!(validate_slug("feat foo").is_err());
    }

    #[test]
    fn invalid_slug_too_long() {
        // 63 chars — one over max
        let slug = "a".repeat(63);
        assert!(validate_slug(&slug).is_err());
    }

    #[test]
    fn sanitize_to_slug_truncates_long_input() {
        // 90-char branch name must be truncated to ≤62 chars
        let input = "feat/".to_string() + &"a".repeat(85);
        let (slug, branch) = sanitize_to_slug(&input).unwrap();
        assert_eq!(branch, input);
        assert!(slug.len() <= 62);
        assert!(!slug.ends_with('-'));
    }

    #[test]
    fn invalid_slug_empty() {
        assert!(validate_slug("").is_err());
    }

    // ── append_gitignore ──────────────────────────────────────────────────────

    #[test]
    fn append_gitignore_creates_file_if_absent() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".gitignore");
        append_gitignore(&path).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains(".ecluse/"));
    }

    #[test]
    fn append_gitignore_appends_to_existing_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".gitignore");
        std::fs::write(&path, "node_modules/\ndist/\n").unwrap();
        append_gitignore(&path).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("node_modules/"));
        assert!(content.contains(".ecluse/"));
    }

    #[test]
    fn append_gitignore_ends_with_newline() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".gitignore");
        append_gitignore(&path).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.ends_with('\n'));
    }

    // ── sanitize_to_slug ──────────────────────────────────────────────────────

    #[test]
    fn sanitize_to_slug_replaces_slash() {
        let (slug, branch) = sanitize_to_slug("feat/sc-123-foo").unwrap();
        assert_eq!(slug, "feat-sc-123-foo");
        assert_eq!(branch, "feat/sc-123-foo");
    }

    #[test]
    fn sanitize_to_slug_lowercases() {
        let (slug, branch) = sanitize_to_slug("FEAT/SC-123").unwrap();
        assert_eq!(slug, "feat-sc-123");
        assert_eq!(branch, "FEAT/SC-123");
    }

    #[test]
    fn sanitize_to_slug_already_valid() {
        let (slug, branch) = sanitize_to_slug("feat-sc-123-foo").unwrap();
        assert_eq!(slug, "feat-sc-123-foo");
        assert_eq!(branch, "feat-sc-123-foo");
    }

    #[test]
    fn sanitize_to_slug_trims_leading_trailing_hyphens() {
        let (slug, branch) = sanitize_to_slug("/feat/").unwrap();
        assert_eq!(slug, "feat");
        assert_eq!(branch, "/feat/");
    }

    #[test]
    fn sanitize_to_slug_multiple_slashes() {
        let (slug, _) = sanitize_to_slug("feat/add-auth/sub").unwrap();
        assert_eq!(slug, "feat-add-auth-sub");
    }

    #[test]
    fn sanitize_to_slug_invalid_after_sanitization() {
        // Single char after sanitization → invalid slug
        assert!(sanitize_to_slug("a").is_err());
    }

    // ── status_str ────────────────────────────────────────────────────────────

    fn svc_status(
        managed: bool,
        healthy: bool,
        wrong_owner: bool,
        listener_pid: Option<u32>,
    ) -> ServiceStatus {
        ServiceStatus {
            name: "api".into(),
            kind: "native",
            port: Some(3001),
            healthy,
            managed,
            pid: Some(42),
            tmux_window: None,
            listener_pid,
            wrong_owner,
            actual_port: None,
            conflicting_slot: None,
            listening_ports: vec![],
        }
    }

    #[test]
    fn status_str_unmanaged_shows_dash() {
        let s = svc_status(false, false, false, None);
        assert_eq!(status_str(&s), "\u{2014}");
    }

    #[test]
    fn status_str_healthy_managed_shows_up() {
        let s = svc_status(true, true, false, None);
        assert_eq!(status_str(&s), "\u{2713} up");
    }

    #[test]
    fn status_str_unhealthy_managed_shows_down() {
        let s = svc_status(true, false, false, None);
        assert_eq!(status_str(&s), "\u{2717} down");
    }

    #[test]
    fn status_str_wrong_owner_with_listener_pid_shows_pid() {
        let s = svc_status(true, false, true, Some(99999));
        assert_eq!(status_str(&s), "\u{2717} wrong owner (PID 99999)");
    }

    #[test]
    fn status_str_wrong_owner_without_listener_pid() {
        let s = svc_status(true, false, true, None);
        assert_eq!(status_str(&s), "\u{2717} wrong owner");
    }

    #[test]
    fn status_str_wrong_owner_takes_precedence_over_healthy() {
        // A service can simultaneously have its stored PID alive AND a
        // different process bound to its port. `wrong_owner` wins.
        let s = svc_status(true, true, true, Some(99999));
        assert_eq!(status_str(&s), "\u{2717} wrong owner (PID 99999)");
    }

    // ── Stopped-session guards ─────────────────────────────────────────────────

    fn test_config() -> config::Config {
        config::Config {
            mode: config::Mode::Host,
            max_slots: 4,
            prefix: "test".into(),
            worktree_dir: ".ecluse/worktrees".into(),
            app_label: "ecluse.role".into(),
            app_label_value: "app".into(),
            strict_port: false,
            port_search_range: 10,
            slot_stride: 1,
            services: vec![],
            hooks: config::HookConfig::default(),
            inherit_env: vec![],
        }
    }

    fn session_with_status(status: state::SessionStatus) -> state::Session {
        state::Session {
            slug: "feat".into(),
            mode: config::Mode::Host,
            slot: 1,
            branch: "feat".into(),
            worktree_path: "/tmp/does-not-exist".into(),
            status,
            pending_op: None,
            compose_project: None,
            overlay_file: None,
            overlay_files: vec![],
            compose_overlays: vec![],
            app_port: None,
            started_at: "2026-01-01T00:00:00Z".into(),
            port_overrides: std::collections::HashMap::new(),
            process_manager: None,
            tmux_session: None,
            pid_files: vec![],
            log_dir: None,
            services_subset: None,
        }
    }

    #[test]
    fn ensure_session_settled_allows_active() {
        let s = session_with_status(state::SessionStatus::Active);
        assert!(ensure_session_settled(&s).is_ok());
    }

    #[test]
    fn ensure_session_settled_rejects_pending() {
        let s = session_with_status(state::SessionStatus::Pending);
        let err = ensure_session_settled(&s).unwrap_err().to_string();
        assert!(err.contains("in progress"), "got: {err}");
    }

    #[test]
    fn ensure_session_settled_rejects_stopped_with_up_hint() {
        let s = session_with_status(state::SessionStatus::Stopped);
        let err = ensure_session_settled(&s).unwrap_err().to_string();
        assert!(err.contains("stopped"), "got: {err}");
        // Actionable: points the user at the command that revives it.
        assert!(err.contains("ecluse up"), "got: {err}");
    }

    #[test]
    fn teardown_skips_bring_down_for_stopped_and_keeps_worktree() {
        // keep_worktree=true on a Stopped session: no bring_down, no removal,
        // just Ok — the worktree stays on disk untouched.
        let dir = tempfile::TempDir::new().unwrap();
        let wt = dir.path().join("kept");
        std::fs::create_dir(&wt).unwrap();
        let mut s = session_with_status(state::SessionStatus::Stopped);
        s.worktree_path = wt.to_string_lossy().into_owned();
        let config = test_config();
        let log = log::StepLogger::new(true);

        teardown_or_skip_stopped(&s, &config, dir.path(), false, true, &log).unwrap();
        assert!(wt.exists(), "kept worktree must remain on disk");
    }

    #[test]
    fn teardown_removes_worktree_for_stopped_when_not_kept() {
        // keep_worktree=false on a Stopped session: bring_down is skipped but
        // the worktree must still be removed. Needs a real registered worktree.
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        let git = |args: &[&str]| {
            let ok = std::process::Command::new("git")
                .args(args)
                .current_dir(root)
                .status()
                .unwrap()
                .success();
            assert!(ok, "git {args:?} failed");
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "t@t"]);
        git(&["config", "user.name", "t"]);
        git(&["commit", "-q", "--allow-empty", "-m", "init"]);
        let wt = root.join("wt-gone");
        git(&["worktree", "add", "-q", wt.to_str().unwrap(), "-b", "gone"]);
        assert!(wt.exists());

        let mut s = session_with_status(state::SessionStatus::Stopped);
        s.worktree_path = wt.to_string_lossy().into_owned();
        let config = test_config();
        let log = log::StepLogger::new(true);

        teardown_or_skip_stopped(&s, &config, root, false, false, &log).unwrap();
        assert!(!wt.exists(), "worktree must be removed when not kept");
    }

    // ── status_str: wrong-port (discovery) ────────────────────────────────────

    #[test]
    fn status_str_wrong_port_shows_discovered_port() {
        let mut s = svc_status(true, false, false, None);
        s.actual_port = Some(3005);
        assert_eq!(status_str(&s), "\u{2717} wrong port 3005");
    }

    #[test]
    fn status_str_wrong_port_names_conflicting_slot() {
        let mut s = svc_status(true, false, false, None);
        s.actual_port = Some(3003);
        s.conflicting_slot = Some((3, Some("feat-x".into())));
        assert_eq!(status_str(&s), "\u{2717} wrong port 3003 (slot 3)");
    }

    // wrong_owner is a distinct condition (someone ELSE holds our port) and
    // must keep precedence over "we are on the wrong port".
    #[test]
    fn status_str_wrong_owner_takes_precedence_over_wrong_port() {
        let mut s = svc_status(true, false, true, Some(777));
        s.actual_port = Some(3005);
        assert_eq!(status_str(&s), "\u{2717} wrong owner (PID 777)");
    }

    #[test]
    fn status_str_unmanaged_ignores_wrong_port() {
        let mut s = svc_status(false, false, false, None);
        s.actual_port = Some(3005);
        assert_eq!(status_str(&s), "\u{2014}");
    }

    // ── actual_str ────────────────────────────────────────────────────────────

    #[test]
    fn actual_str_is_dash_when_nothing_listening() {
        let s = svc_status(true, false, false, None);
        assert_eq!(actual_str(&s), "\u{2014}");
    }

    #[test]
    fn actual_str_is_dash_when_only_assigned_port_listening() {
        let mut s = svc_status(true, true, false, None);
        s.listening_ports = vec![3001];
        assert_eq!(actual_str(&s), "\u{2014}");
    }

    #[test]
    fn actual_str_shows_port_on_mismatch() {
        let mut s = svc_status(true, false, false, None);
        s.actual_port = Some(3005);
        s.listening_ports = vec![3005];
        assert_eq!(actual_str(&s), "3005");
    }

    // A healthy service with an HMR socket lists both ports, but status_str
    // stays "up" because actual_port (the wrong-port signal) is unset.
    #[test]
    fn actual_str_lists_extra_sockets_without_flagging() {
        let mut s = svc_status(true, true, false, None);
        s.listening_ports = vec![3001, 24678];
        assert_eq!(actual_str(&s), "3001 24678");
        assert_eq!(status_str(&s), "\u{2713} up");
    }

    // ── mismatch_hint ─────────────────────────────────────────────────────────

    #[test]
    fn mismatch_hint_none_without_discovered_port() {
        let s = svc_status(true, false, false, None);
        assert!(mismatch_hint("feat-a", &s).is_none());
    }

    #[test]
    fn mismatch_hint_points_at_down_up_not_kill() {
        let mut s = svc_status(true, false, false, None);
        s.actual_port = Some(3005);
        let hint = mismatch_hint("feat-a", &s).unwrap();
        assert!(hint.contains("ecluse down feat-a --keep-worktree"));
        assert!(hint.contains("ecluse up feat-a"));
        assert!(!hint.contains("kill"));
    }

    // The single most important message in the feature: an agent that reads
    // this must not kill the sibling session's process.
    #[test]
    fn mismatch_hint_names_owning_session_and_forbids_kill() {
        let mut s = svc_status(true, false, false, None);
        s.actual_port = Some(3003);
        s.conflicting_slot = Some((3, Some("feat-x".into())));
        let hint = mismatch_hint("feat-a", &s).unwrap();
        assert!(hint.contains("slot 3"));
        assert!(hint.contains("feat-x"));
        assert!(hint.contains("do not kill"));
    }

    #[test]
    fn mismatch_hint_reports_unowned_slot_territory() {
        let mut s = svc_status(true, false, false, None);
        s.actual_port = Some(3004);
        s.conflicting_slot = Some((4, None));
        let hint = mismatch_hint("feat-a", &s).unwrap();
        assert!(hint.contains("slot 4's territory"));
    }

    // ── listening_summary ─────────────────────────────────────────────────────

    #[test]
    fn listening_summary_dash_when_nothing_listening() {
        assert_eq!(listening_summary(&[3001], &[]), "-");
    }

    #[test]
    fn listening_summary_no_marker_when_all_expected_present() {
        assert_eq!(listening_summary(&[3001, 5433], &[3001, 5433]), "3001 5433");
    }

    #[test]
    fn listening_summary_marks_missing_expected_port() {
        assert_eq!(listening_summary(&[3001], &[3005]), "3005 !");
    }

    // Extra sockets (HMR, debug, inspector) are normal and must not be
    // reported as a mismatch — only a MISSING assigned port is.
    #[test]
    fn listening_summary_tolerates_extra_ports() {
        assert_eq!(
            listening_summary(&[3001], &[3001, 24678]),
            "3001 24678",
            "extra HMR socket must not flag a mismatch"
        );
    }

    #[test]
    fn listening_summary_marks_partial_match() {
        assert_eq!(listening_summary(&[3001, 5433], &[3001]), "3001 !");
    }

    // ── expected_native_ports / port_mismatch ─────────────────────────────────

    fn hybrid_config() -> config::Config {
        toml::from_str(
            r#"
mode = "hybrid"

[[services]]
name = "api"
run = "native"
base_port = 3000
command = "pnpm dev"

[[services]]
name = "worker"
run = "native"
base_port = 9000

[[services]]
name = "postgres"
run = "docker"
base_port = 5432
"#,
        )
        .unwrap()
    }

    fn session_with_ports(ports: &[(&str, u16)]) -> state::Session {
        let overrides: std::collections::HashMap<String, u16> =
            ports.iter().map(|(k, v)| (k.to_string(), *v)).collect();
        serde_json::from_value(serde_json::json!({
            "slug": "feat-a",
            "mode": "hybrid",
            "slot": 1,
            "branch": "feat-a",
            "worktree_path": "/tmp/feat-a",
            "compose_project": null,
            "overlay_file": null,
            "app_port": null,
            "started_at": "2026-01-01T00:00:00Z",
            "port_overrides": overrides,
        }))
        .unwrap()
    }

    // Hybrid sessions keep Docker service ports in port_overrides. Those are
    // held by the Docker daemon, never by the session's process tree, so they
    // must not count as "expected" — otherwise every hybrid session shows `!`.
    #[test]
    fn expected_native_ports_excludes_docker_services_in_hybrid() {
        let config = hybrid_config();
        let session = session_with_ports(&[("api", 3001), ("worker", 9001), ("postgres", 5433)]);
        let expected = expected_native_ports(&config, &session);
        assert_eq!(expected, vec![3001], "only spawned native services count");
        assert!(!port_mismatch(&expected, &[3001]));
        assert_eq!(listening_summary(&expected, &[3001]), "3001");
    }

    #[test]
    fn expected_native_ports_falls_back_to_nominal_port() {
        let config = hybrid_config();
        let session = session_with_ports(&[("postgres", 5433)]);
        assert_eq!(expected_native_ports(&config, &session), vec![3001]);
    }

    #[test]
    fn port_mismatch_false_when_nothing_listening() {
        assert!(!port_mismatch(&[3001], &[]));
    }

    #[test]
    fn port_mismatch_true_when_assigned_port_missing() {
        assert!(port_mismatch(&[3001], &[3005]));
    }

    #[test]
    fn port_mismatch_ignores_extra_ports() {
        assert!(!port_mismatch(&[3001], &[3001, 24678]));
    }

    // ── discover_service ──────────────────────────────────────────────────────

    // Two tmux panes: 100 (service A, nothing listening) and 200 (service B,
    // whose child 201 listens on 3002). 999 is unrelated and holds 3001.
    const PS_TWO_PANES: &str = "100 1\n200 1\n201 200\n999 1\n";

    #[test]
    fn discover_service_does_not_blame_sibling_pane_port() {
        let snap = discover::parse_snapshot("p201\nn*:3002\n", PS_TWO_PANES);
        let a = discover_service(&snap, Some(100), true, Some(3001));
        assert!(!a.healthy);
        assert!(a.tree_ports.is_empty());
        assert_eq!(a.wrong_port, None, "B's port must not be reported for A");
        let b = discover_service(&snap, Some(200), true, Some(3002));
        assert!(b.healthy);
        assert_eq!(b.wrong_port, None);
        assert!(!b.wrong_owner);
    }

    #[test]
    fn discover_service_reports_wrong_port_when_assigned_missing() {
        let snap = discover::parse_snapshot("p201\nn*:3005\n", PS_TWO_PANES);
        let d = discover_service(&snap, Some(200), true, Some(3002));
        assert!(!d.healthy);
        assert_eq!(d.wrong_port, Some(3005));
        assert_eq!(d.tree_ports, vec![3005]);
    }

    // Extra sockets while the assigned port is held are not a wrong port —
    // same rule as `ecluse ls`.
    #[test]
    fn discover_service_tolerates_extra_sockets() {
        let snap = discover::parse_snapshot("p201\nn*:3002\nn*:24678\n", PS_TWO_PANES);
        let d = discover_service(&snap, Some(200), true, Some(3002));
        assert!(d.healthy);
        assert_eq!(d.wrong_port, None);
        assert_eq!(d.tree_ports, vec![3002, 24678]);
    }

    #[test]
    fn discover_service_flags_foreign_listener_as_wrong_owner() {
        let snap = discover::parse_snapshot("p999\nn*:3001\n", PS_TWO_PANES);
        let d = discover_service(&snap, Some(100), true, Some(3001));
        assert!(d.wrong_owner);
        assert_eq!(d.listener_pid, Some(999));
        assert!(!d.healthy);
    }

    #[test]
    fn discover_service_descendant_listener_is_not_wrong_owner() {
        let snap = discover::parse_snapshot("p201\nn*:3002\n", PS_TWO_PANES);
        let d = discover_service(&snap, Some(200), true, Some(3002));
        assert_eq!(d.listener_pid, Some(201));
        assert!(!d.wrong_owner);
    }

    // A recycled pid yields no root: nothing is attributed to the service,
    // but a listener on its port is still somebody else's.
    #[test]
    fn discover_service_without_root_attributes_nothing() {
        let snap = discover::parse_snapshot("p201\nn*:3002\n", PS_TWO_PANES);
        let d = discover_service(&snap, None, true, Some(3002));
        assert!(!d.healthy);
        assert!(d.tree_ports.is_empty());
        assert_eq!(d.wrong_port, None);
        assert!(d.wrong_owner);
    }

    #[test]
    fn discover_service_alive_without_port_is_healthy() {
        let snap = discover::parse_snapshot("", PS_TWO_PANES);
        let d = discover_service(&snap, Some(100), true, None);
        assert!(d.healthy);
        assert_eq!(d.listener_pid, None);
    }

    #[test]
    fn discover_service_skips_owner_check_without_recorded_process() {
        let snap = discover::parse_snapshot("p999\nn*:3001\n", PS_TWO_PANES);
        let d = discover_service(&snap, None, false, Some(3001));
        assert_eq!(d.listener_pid, None);
        assert!(!d.wrong_owner);
    }

    // ── service_tree_root ─────────────────────────────────────────────────────

    #[test]
    fn service_tree_root_accepts_live_token_verified_pid() {
        let dir = TempDir::new().unwrap();
        let pid_file = dir.path().join("api.pid");
        let me = std::process::id();
        process::write_pid_file_with_token(&pid_file, me).unwrap();
        assert_eq!(service_tree_root(&pid_file, None, "api"), Some(me));
    }

    // The pid is alive but its start token doesn't match: the pid was recycled
    // by an unrelated process, so its ports must not be attributed.
    #[test]
    fn service_tree_root_rejects_recycled_pid() {
        let dir = TempDir::new().unwrap();
        let pid_file = dir.path().join("api.pid");
        let me = std::process::id();
        std::fs::write(&pid_file, format!("{me}\nnot-the-real-start-time\n")).unwrap();
        assert_eq!(service_tree_root(&pid_file, None, "api"), None);
    }

    #[test]
    fn service_tree_root_none_without_pid_file_or_tmux() {
        let dir = TempDir::new().unwrap();
        let pid_file = dir.path().join("api.pid");
        assert_eq!(service_tree_root(&pid_file, None, "api"), None);
    }
}

/// Sanitize a branch name or slug into a valid ecluse slug + original branch pair.
/// Replaces '/' with '-', lowercases, trims leading/trailing hyphens, and
/// truncates to 62 chars (trimming any trailing hyphen after truncation).
/// The branch is the original input (used for `git worktree add`).
/// The slug is the sanitized form (used for paths, Docker, tmux).
fn sanitize_to_slug(input: &str) -> Result<(String, String)> {
    let branch = input.to_string();
    let slug = input.to_lowercase().replace('/', "-");
    let slug = slug.trim_matches('-').to_string();
    let slug = if slug.len() > 62 {
        slug[..62].trim_end_matches('-').to_string()
    } else {
        slug
    };
    validate_slug(&slug)?;
    Ok((slug, branch))
}

fn current_git_branch(cwd: &std::path::Path) -> Result<String> {
    let output = std::process::Command::new("git")
        .args(["branch", "--show-current"])
        .current_dir(cwd)
        .output()
        .context("failed to run git branch --show-current")?;
    let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if branch.is_empty() {
        return Err(anyhow::anyhow!(
            "detached HEAD — pass a branch name explicitly: ecluse up <branch>"
        ));
    }
    Ok(branch)
}

fn prompt_branch_name() -> Result<String> {
    // Non-interactive: fail fast instead of hanging on stdin.
    if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        return Err(anyhow::anyhow!(
            "not inside a git worktree and stdin is not a terminal; pass a branch name explicitly: ecluse up <branch>"
        ));
    }
    print!("You are in the main worktree. Enter a branch name to create a worktree for: ");
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let branch = input.trim().to_string();
    if branch.is_empty() {
        return Err(anyhow::anyhow!("no branch name provided"));
    }
    Ok(branch)
}

/// Resolve (slug, branch, reuse_worktree) for `ecluse up`.
///
/// 1. Explicit arg → sanitize to slug, use original as branch.
/// 2. cwd inside an ecluse-registered worktree → return stored slug + branch.
/// 3. cwd inside a non-ecluse git worktree (git rev-parse --git-dir contains
///    /.git/worktrees/) → read branch from cwd, auto-register (reuse_worktree=true).
/// 4. cwd in main worktree / repo root → prompt for branch name.
fn resolve_slug_and_branch(
    arg: &Option<String>,
    state: &state::State,
    _root: &std::path::Path,
) -> Result<(String, String, bool, Option<std::path::PathBuf>)> {
    if let Some(input) = arg {
        let (slug, branch) = sanitize_to_slug(input)?;
        return Ok((slug, branch, false, None));
    }

    let cwd = std::env::current_dir().context("could not determine current directory")?;

    // 1. Inside an ecluse-registered worktree → reuse stored slug/branch.
    // Includes Stopped sessions, so `up` from a kept worktree resumes its slot.
    if let Some(session) = state
        .sessions
        .iter()
        .find(|s| cwd.starts_with(std::path::Path::new(&s.worktree_path)))
    {
        return Ok((session.slug.clone(), session.branch.clone(), true, None));
    }

    // 2. Inside a non-ecluse git worktree → auto-register it, preserving the actual path.
    if worktree::is_inside_git_worktree(&cwd) {
        let actual_root = worktree::git_worktree_root(&cwd)?;
        let branch = current_git_branch(&cwd)?;
        let (slug, branch) = sanitize_to_slug(&branch)?;
        return Ok((slug, branch, true, Some(actual_root)));
    }

    // 3. In main worktree / repo root → prompt for branch name.
    let branch = prompt_branch_name()?;
    let (slug, branch) = sanitize_to_slug(&branch)?;
    Ok((slug, branch, false, None))
}

/// Error unless the session is `Active` — its env and services are only
/// meaningful then. `Pending` means an op is in flight; `Stopped` means the
/// worktree was kept but services are down, so reading its env/status/shell
/// would surface stale slot values for services that are no longer running.
fn ensure_session_settled(session: &state::Session) -> Result<()> {
    match session.status {
        state::SessionStatus::Active => Ok(()),
        state::SessionStatus::Pending => Err(anyhow::anyhow!(
            "session '{}' has an up/down operation in progress; retry when it finishes, or run `ecluse down {}` if it crashed",
            session.slug,
            session.slug
        )),
        state::SessionStatus::Stopped => Err(anyhow::anyhow!(
            "session '{}' is stopped (worktree kept at {}); run `ecluse up {}` to restart it",
            session.slug,
            session.worktree_path,
            session.slug
        )),
    }
}

fn resolve_slug_from_args(arg: Option<&str>, state: &state::State, hint: &str) -> Result<String> {
    match arg {
        Some(s) => {
            validate_slug(s)?;
            Ok(s.to_string())
        }
        None => {
            let cwd = std::env::current_dir().context("could not determine current directory")?;

            // Inside any known ecluse session (Stopped included) — use it.
            if let Some(session) = state
                .sessions
                .iter()
                .find(|s| cwd.starts_with(std::path::Path::new(&s.worktree_path)))
            {
                return Ok(session.slug.clone());
            }

            // No active session — give a context-aware hint.
            let msg = if worktree::is_inside_git_worktree(&cwd) {
                // Inside a linked git worktree with no active session (e.g. torn down).
                let branch = current_git_branch(&cwd).unwrap_or_else(|_| "this branch".into());
                format!(
                    "no active session for '{branch}'; run `ecluse up` to start one\n  hint: {hint}"
                )
            } else if let Ok(root) = worktree::WorktreeManager::main_worktree_root(&cwd) {
                if cwd == root {
                    // At repo root — no sessions at all.
                    format!(
                        "no active sessions; run `ecluse up <branch>` to start one\n  hint: {hint}"
                    )
                } else {
                    format!(
                        "not inside an ecluse worktree; pass a slug or cd into a worktree\n  hint: {hint}"
                    )
                }
            } else {
                format!(
                    "not inside an ecluse worktree; pass a slug or cd into a worktree\n  hint: {hint}"
                )
            };

            Err(anyhow::anyhow!("{msg}"))
        }
    }
}

/// Resolve whether to keep or delete a worktree before tearing down a session.
///
/// - `keep_worktree` set: skip the prompt, keep the worktree.
/// - `yes` set: skip the prompt, delete the worktree (non-interactive confirmation).
/// - Neither: prompt the user interactively.
///
/// Returns `Ok(true)` to keep the worktree, `Ok(false)` to delete it,
/// or `Err` to abort the whole `down` operation.
fn resolve_worktree_keep(
    worktree_path: &std::path::Path,
    keep_worktree: bool,
    yes: bool,
) -> Result<bool> {
    if keep_worktree {
        return Ok(true);
    }
    if yes {
        return Ok(false);
    }
    use worktree::WorktreeRemovalChoice;
    match worktree::prompt_worktree_removal(worktree_path) {
        WorktreeRemovalChoice::Stop => Err(anyhow::anyhow!(
            "aborted; pass --keep-worktree to preserve it or -y to delete it"
        )),
        WorktreeRemovalChoice::KeepWorktree => Ok(true),
        WorktreeRemovalChoice::DeleteWorktree => Ok(false),
    }
}

fn validate_slug(slug: &str) -> Result<()> {
    let re = regex_lite::Regex::new(r"^[a-z0-9][a-z0-9\-]{0,60}[a-z0-9]$").unwrap();
    if !re.is_match(slug) {
        return Err(error::EcluseError::SlugInvalid(slug.to_string()).into());
    }
    Ok(())
}

fn validate_branch(branch: &str) -> Result<()> {
    if branch.is_empty() {
        return Err(anyhow::anyhow!("branch name must not be empty"));
    }
    if branch.starts_with('-') {
        return Err(anyhow::anyhow!(
            "invalid branch name '{}': must not start with '-'",
            branch
        ));
    }
    // Reject git refspec metacharacters that could resolve to unintended commits
    for ch in ["..", "~", "^", ":"] {
        if branch.contains(ch) {
            return Err(anyhow::anyhow!(
                "invalid branch name '{}': must not contain '{}'",
                branch,
                ch
            ));
        }
    }
    Ok(())
}

fn cmd_up(args: cli::UpArgs) -> Result<()> {
    // --json implies --quiet for step output
    let log = log::StepLogger::new(args.quiet || args.json);

    log.step("Loading config...");
    let (config, root) = config::Config::find_and_load()?;
    log.detail(&format!("mode: {}", config.mode));

    let warnings = validate::validate_config(&config)?;
    for w in &warnings {
        log.warn(w);
    }

    let global = process::load_global_config()?;
    validate::validate_process_manager(&global.process_manager)?;

    let port_overrides: std::collections::HashMap<String, u16> =
        args.port_overrides.iter().cloned().collect();
    let service_filter: Option<std::collections::HashSet<String>> =
        parse_service_filter(&args.services, &config)?;

    // Resolve slug + branch from a read-only snapshot. Resolution can prompt
    // for a branch name; the prompt must not hold ANY lock (a shared lock
    // still blocks exclusive acquirers), so clone the state and release the
    // guard before resolving.
    let snapshot = {
        let guard = state::StateGuard::acquire_shared(&root)?;
        guard.state.clone()
    };
    let (slug, branch, implicit_reuse, worktree_override) =
        resolve_slug_and_branch(&args.slug, &snapshot, &root)?;
    validate_branch(&branch)?;

    // Short exclusive section: route to resume, or reserve the slot with a
    // pending session, then release the lock for the slow provisioning work.
    let (slot, op_id) = {
        let mut guard = state::StateGuard::acquire(&root)?;

        if let Some(existing) = guard.state.find_session(&slug).cloned() {
            if existing.status == state::SessionStatus::Pending {
                return Err(anyhow::anyhow!(
                    "session '{slug}' has an operation in progress (started {}); wait for it to finish, or run `ecluse down {slug}` if it crashed",
                    existing.started_at
                ));
            }
            // Slugs are sanitized branch names (feat/foo → feat-foo), so two
            // different branches can collide on one slug. Addressing the
            // session by its slug or by its exact branch resumes it; anything
            // else would silently resume the wrong branch.
            if let Some(requested) = args.slug.as_deref() {
                if requested != existing.slug && existing.branch != branch {
                    return Err(anyhow::anyhow!(
                        "slug '{}' is already used by branch '{}' (requested branch '{}'); run `ecluse down {}` first or pick a different branch name",
                        slug,
                        existing.branch,
                        branch,
                        slug
                    ));
                }
            }
            log.step("Looking for existing session...");
            if existing.status == state::SessionStatus::Stopped {
                log.detail(&format!(
                    "found stopped session '{}' (slot {}) — restarting at same slot",
                    slug, existing.slot
                ));
            } else {
                log.detail(&format!(
                    "found session '{}' (slot {}) — reusing worktree",
                    slug, existing.slot
                ));
            }
            return cmd_up_resume(existing, args, config, root, guard, log);
        }

        log.step("Allocating slot...");
        let allocator = slot::SlotAllocator::new(&config, &guard.state);
        let slot = allocator.allocate_next()?;
        log.detail(&format!("slot {slot}"));

        let planned_worktree = worktree_override.clone().unwrap_or_else(|| {
            worktree::WorktreeManager::new(root.clone()).worktree_path(&config, &slug)
        });
        let op_id = state::new_op_id();
        guard.state.add_session(state::Session {
            slug: slug.clone(),
            mode: config.mode.clone(),
            slot,
            branch: branch.clone(),
            worktree_path: planned_worktree.display().to_string(),
            status: state::SessionStatus::Pending,
            pending_op: Some(state::PendingOp {
                id: op_id.clone(),
                since: chrono::Utc::now().to_rfc3339(),
            }),
            compose_project: None,
            overlay_file: None,
            overlay_files: vec![],
            compose_overlays: vec![],
            app_port: None,
            started_at: chrono::Utc::now().to_rfc3339(),
            port_overrides: std::collections::HashMap::new(),
            process_manager: None,
            tmux_session: None,
            pid_files: vec![],
            log_dir: None,
            services_subset: None,
        });
        guard.commit()?;
        (slot, op_id)
    };

    let handler = modes::get_handler(&config);
    let no_skip = std::collections::HashSet::new();
    let no_existing = std::collections::HashMap::new();
    let result = handler.bring_up(
        &modes::BringUpRequest {
            slug: &slug,
            slot,
            branch: &branch,
            watch: args.watch,
            reuse_worktree: args.reuse_worktree || implicit_reuse,
            no_inherit_env: args.no_inherit_env,
            worktree_override,
            port_overrides: &port_overrides,
            service_filter: service_filter.as_ref(),
            skip_services: &no_skip,
            existing_port_overrides: &no_existing,
        },
        &config,
        &root,
        &log,
    );

    // Re-acquire to finalize: replace the pending reservation with the real
    // session, or drop it when provisioning failed (bring_up rolled back).
    // Only the operation that wrote the reservation may finalize it — if
    // another command (down/flush) removed or took over the entry meanwhile,
    // writing state here would resurrect a session whose resources are gone.
    let mut guard = state::StateGuard::acquire(&root)?;
    if guard.state.still_owned(&slug, &op_id) {
        guard.state.remove_session(&slug);
        match result {
            Ok(session) => {
                if args.json {
                    print_up_json(&session, &root)?;
                } else {
                    print_up_summary(&session, &config, &log);
                }
                guard.state.add_session(session);
                guard.commit()?;
                Ok(())
            }
            Err(e) => {
                guard.commit()?;
                Err(e)
            }
        }
    } else {
        drop(guard);
        match result {
            Ok(session) => {
                log.warn(&format!(
                    "session '{slug}' was removed or taken over by another command while provisioning; tearing the new resources back down"
                ));
                let _ = handler.bring_down(&session, &config, &root, false, false, &log);
                Err(anyhow::anyhow!(
                    "session '{slug}' was removed by another command while it was being provisioned; the resources it created were torn down — re-run `ecluse up {slug}`"
                ))
            }
            Err(e) => Err(e),
        }
    }
}

/// Validate --services names and build the filter set.
fn parse_service_filter(
    services: &Option<Vec<String>>,
    config: &config::Config,
) -> Result<Option<std::collections::HashSet<String>>> {
    match services.as_deref() {
        None | Some([]) => Ok(None),
        Some(names) => {
            let set: std::collections::HashSet<String> = names.iter().cloned().collect();
            for name in &set {
                if !config.services.iter().any(|s| &s.name == name) {
                    let list = config
                        .services
                        .iter()
                        .map(|s| s.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    let hint = if list.is_empty() {
                        "no services are defined in .ecluse.toml".to_string()
                    } else {
                        format!("defined services are: {list}")
                    };
                    return Err(anyhow::anyhow!("unknown service '{}'; {hint}", name));
                }
            }
            Ok(Some(set))
        }
    }
}

/// Resume an existing session: restart downed services, skip healthy ones.
/// With --force: kill everything first, then start all (minus --skip).
///
/// The session is marked Pending and the lock released while services are
/// health-checked and started; the entry is restored or replaced when done.
fn cmd_up_resume(
    existing: state::Session,
    args: cli::UpArgs,
    config: config::Config,
    root: std::path::PathBuf,
    mut guard: state::StateGuard,
    log: log::StepLogger,
) -> Result<()> {
    // Build and validate the explicit --skip set before touching state.
    let explicit_skip: std::collections::HashSet<String> = args
        .skip
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .cloned()
        .collect();
    for name in &explicit_skip {
        if !config.services.iter().any(|s| &s.name == name) {
            let list = config
                .services
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            let hint = if list.is_empty() {
                "no services are defined in .ecluse.toml".to_string()
            } else {
                format!("defined services are: {list}")
            };
            return Err(anyhow::anyhow!("unknown service '{}'; {hint}", name));
        }
    }

    // Mark pending and release the lock for the health checks + startup.
    let op_id = match guard.state.mark_pending(&existing.slug) {
        Some((_, op_id)) => op_id,
        None => return Err(error::EcluseError::SessionNotFound(existing.slug.clone()).into()),
    };
    guard.commit()?;
    drop(guard);

    let outcome = resume_provision(&existing, &args, &config, &root, &explicit_skip, &log);

    // Re-acquire to finalize: replace with the refreshed session, or restore
    // the original (still-active) entry when nothing changed or on failure.
    // Skip entirely when another command took the session over meanwhile —
    // writing here would resurrect an entry that command deleted.
    let mut guard = state::StateGuard::acquire(&root)?;
    if !guard.state.still_owned(&existing.slug, &op_id) {
        drop(guard);
        if let Ok(Some((updated, _, _))) = &outcome {
            log.warn(&format!(
                "session '{}' was removed or taken over by another command during resume; stopping the services this resume started",
                existing.slug
            ));
            let handler = modes::get_handler(&config);
            let _ = handler.bring_down(updated, &config, &root, true, true, &log);
        }
        return match outcome {
            Err(e) => Err(e),
            _ => Err(anyhow::anyhow!(
                "session '{}' was removed by another command while it was being resumed; re-run `ecluse up {}`",
                existing.slug,
                existing.slug
            )),
        };
    }
    guard.state.remove_session(&existing.slug);
    match outcome {
        Ok(Some((updated, started, skipped))) => {
            if !args.quiet && !args.json {
                println!();
                log.success(&format!(
                    "{} service{} started, {} skipped",
                    started,
                    if started == 1 { "" } else { "s" },
                    skipped
                ));
            }
            if args.json {
                print_up_json(&updated, &root)?;
            }
            guard.state.add_session(updated);
            guard.commit()?;
            Ok(())
        }
        Ok(None) => {
            // All services already running. Stopped sessions never get here
            // (resume_provision sends them through bring_up).
            let mut restored = existing.clone();
            restored.status = state::SessionStatus::Active;
            restored.pending_op = None;
            guard.state.add_session(restored.clone());
            guard.commit()?;
            log.step("All services already running — nothing to do.");
            if args.json {
                print_up_json(&restored, &root)?;
            } else {
                print_up_summary(&restored, &config, &log);
            }
            Ok(())
        }
        Err(e) => {
            guard.state.add_session(existing);
            guard.commit()?;
            Err(e)
        }
    }
}

/// Health-check and start services for a resumed session. Runs without the
/// state lock. Returns None when everything is already running, otherwise
/// the refreshed session plus (started, skipped) counts.
fn resume_provision(
    existing: &state::Session,
    args: &cli::UpArgs,
    config: &config::Config,
    root: &std::path::Path,
    explicit_skip: &std::collections::HashSet<String>,
    log: &log::StepLogger,
) -> Result<Option<(state::Session, usize, usize)>> {
    let handler = modes::get_handler(config);

    let mut skip_services: std::collections::HashSet<String> = explicit_skip.clone();

    if args.force {
        // Kill all non-skipped services.
        log.step("--force: killing services on allocated ports...");
        force_kill_session_services(existing, config, root, explicit_skip, log);
    } else {
        // Auto-detect already-running services and add them to skip set.
        log.step("Checking service health...");
        let native_svcs: Vec<_> = config
            .services
            .iter()
            .filter(|s| s.run == config::ServiceRun::Native)
            .collect();
        let docker_svcs: Vec<_> = config
            .services
            .iter()
            .filter(|s| s.run == config::ServiceRun::Docker)
            .collect();

        let docker_matches = if !docker_svcs.is_empty() {
            sync::find_docker_services(
                &docker_svcs,
                &modes::compose_project_name(config, &existing.slug),
            )
        } else {
            vec![]
        };

        for svc in &native_svcs {
            if explicit_skip.contains(&svc.name) {
                log.detail(&format!("{}: skipped (--skip)", svc.name));
                continue;
            }
            // Identity-based check: the session's own pid file (token-verified)
            // or tmux window — never lsof discovery, whose depth-1 scan misses
            // servers with a cwd in a subdirectory and then spawns duplicates.
            let expected_port = existing.port_overrides.get(&svc.name).copied();
            let alive = sync::native_service_running(root, existing, &svc.name, expected_port);
            if alive {
                log.detail(&format!("{}: \u{2713} already running — skipped", svc.name));
                skip_services.insert(svc.name.clone());
            } else {
                log.detail(&format!("{}: \u{2717} down — will start", svc.name));
            }
        }
        for svc in &docker_svcs {
            if explicit_skip.contains(&svc.name) {
                log.detail(&format!("{}: skipped (--skip)", svc.name));
                continue;
            }
            let running = docker_matches.iter().any(|(name, _)| name == &svc.name);
            if running {
                log.detail(&format!("{}: \u{2713} already running — skipped", svc.name));
                skip_services.insert(svc.name.clone());
            } else {
                log.detail(&format!("{}: \u{2717} down — will start", svc.name));
            }
        }
    }

    let skipped_count = skip_services.len();
    let total = config.services.len();
    let to_start = total.saturating_sub(skipped_count);

    // A Stopped session had its port_overrides/app_port cleared by
    // `mark_stopped`. Even when nothing needs (re)starting, fall through to
    // `bring_up` so it re-probes and re-fills the port mapping — otherwise the
    // resumed Active session would be persisted with no ports and `ecluse env`
    // would emit nothing. Active sessions keep the cheap early-return.
    let resuming_stopped = existing.status == state::SessionStatus::Stopped;
    if to_start == 0 && !args.force && !resuming_stopped {
        return Ok(None);
    }

    // Resume always reuses the recorded worktree. If a Stopped session's kept
    // worktree was deleted by hand, bring_up would surface the generic
    // "remove --reuse-worktree" advice — nonsense here, since the user ran a bare
    // `ecluse up`. Give a tailored, actionable message instead.
    if resuming_stopped && !std::path::Path::new(&existing.worktree_path).exists() {
        return Err(anyhow::anyhow!(
            "worktree for stopped session '{}' is missing at {}; run `ecluse down {}` to clear the stale entry, then `ecluse up {}` to start fresh",
            existing.slug,
            existing.worktree_path,
            existing.slug,
            existing.slug
        ));
    }

    let port_overrides: std::collections::HashMap<String, u16> =
        args.port_overrides.iter().cloned().collect();
    let service_filter = parse_service_filter(&args.services, config)?;

    let updated_session = handler.bring_up(
        &modes::BringUpRequest {
            slug: &existing.slug,
            slot: existing.slot,
            branch: &existing.branch,
            watch: args.watch,
            reuse_worktree: true, // always reuse-worktree on resume
            no_inherit_env: args.no_inherit_env,
            // Honor the worktree path recorded in state.json. Without this, bring_up
            // recomputes the default `<root>/<worktree_dir>/<slug>` location and breaks
            // sessions whose worktree lives outside `.ecluse/worktrees/` — e.g. those
            // auto-registered from a sibling git worktree directory.
            worktree_override: Some(std::path::PathBuf::from(&existing.worktree_path)),
            port_overrides: &port_overrides,
            service_filter: service_filter.as_ref(),
            skip_services: &skip_services,
            existing_port_overrides: &existing.port_overrides,
        },
        config,
        root,
        log,
    )?;

    Ok(Some((updated_session, to_start, skipped_count)))
}

/// Kill all non-skipped services for a session.
/// Native: kill by port (lsof) for PIDs this session owns, then by PID files.
/// Docker: docker stop by container name.
fn force_kill_session_services(
    session: &state::Session,
    config: &config::Config,
    root: &std::path::Path,
    skip: &std::collections::HashSet<String>,
    log: &log::StepLogger,
) {
    // Kill by port first, while the session's pid files still exist — they
    // are how ownership is established. Docker services are stopped via
    // docker stop — never kill their host port by PID, as the listening
    // process may be the container runtime itself (e.g. OrbStack) rather
    // than the container.
    let docker_svc_names: std::collections::HashSet<&str> = config
        .services
        .iter()
        .filter(|s| s.run == config::ServiceRun::Docker)
        .map(|s| s.name.as_str())
        .collect();
    for (svc_name, port) in &session.port_overrides {
        if skip.contains(svc_name) {
            log.detail(&format!("{}: skipped (--skip)", svc_name));
            continue;
        }
        if docker_svc_names.contains(svc_name.as_str()) {
            continue;
        }
        let output = std::process::Command::new("lsof")
            .args(["-ti", &format!("TCP:{}", port), "-sTCP:LISTEN"])
            .output();
        if let Ok(out) = output {
            let stdout = String::from_utf8_lossy(&out.stdout);
            for pid_str in stdout.split_whitespace() {
                let Ok(pid) = pid_str.trim().parse::<u32>() else {
                    continue;
                };
                // The recorded port may be stale — another session's
                // auto-bumped service or an unrelated app could hold it by
                // now. Only kill PIDs that resolve to THIS session.
                let owned = whose_pid::resolve(root, std::slice::from_ref(session), pid)
                    .is_some_and(|o| o.slug == session.slug);
                if !owned {
                    log.warn(&format!(
                        "port {} is held by PID {} which is not owned by session '{}'; skipping — kill it manually if intended",
                        port, pid, session.slug
                    ));
                    continue;
                }
                log.detail(&format!(
                    "killing process {} on port {} ({})",
                    pid, port, svc_name
                ));
                process::kill_pid_with_grace(pid);
            }
        }
    }

    // Kill native via existing PID files.
    if let Some(pm) = &session.process_manager {
        let result = session.spawn_result();
        let pid_files_to_kill: Vec<_> = result
            .pid_files
            .iter()
            .filter(|pf| {
                let svc_name = pf.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                !skip.contains(svc_name)
            })
            .cloned()
            .collect();
        let filtered_result = process::SpawnResult {
            tmux_session: result.tmux_session.clone(),
            pid_files: pid_files_to_kill,
            log_dir: result.log_dir.clone(),
        };
        process::kill_services(pm, &filtered_result);
    }

    // Stop docker containers for non-skipped docker services.
    let docker_svcs: Vec<_> = config
        .services
        .iter()
        .filter(|s| s.run == config::ServiceRun::Docker && !skip.contains(&s.name))
        .collect();
    for svc in &docker_svcs {
        let container_name = format!("{}-{}-{}", config.prefix, svc.name, session.slot);
        log.detail(&format!("stopping container {}", container_name));
        let _ = docker::docker_cmd()
            .args(["stop", &container_name])
            .status();
    }
}

fn print_up_summary(session: &state::Session, _config: &config::Config, log: &log::StepLogger) {
    println!();
    log.success(&format!(
        "Session '{}' ready (slot {})",
        session.slug, session.slot
    ));
    println!();
    println!("  Worktree:  {}", session.worktree_path);
    println!("  Mode:      {}", session.mode);
    println!("  Branch:    {}", session.branch);

    match &session.mode {
        config::Mode::Container => {
            if let Some(port) = session.app_port {
                println!("  App URL:   http://localhost:{}", port);
            }
            println!(
                "  Project:   {}",
                session.compose_project.as_deref().unwrap_or("-")
            );
        }
        config::Mode::Host | config::Mode::Hybrid => {
            if let Some(port) = session.app_port {
                println!("  App port:  {}", port);
            }
            println!();
            println!("  Next:  ecluse shell {}", session.slug);
        }
    }
    println!();
}

fn print_up_json(session: &state::Session, _root: &std::path::Path) -> Result<()> {
    let env_file = std::path::Path::new(&session.worktree_path).join(".env.ecluse");
    let mut env_vars: serde_json::Map<String, serde_json::Value> = serde_json::Map::new();
    for (k, v) in env::parse_env_file(&env_file) {
        env_vars.insert(k, serde_json::Value::String(v));
    }
    let out = serde_json::json!({
        "slug": session.slug,
        "slot": session.slot,
        "mode": session.mode.to_string(),
        "branch": session.branch,
        "worktree_path": session.worktree_path,
        "env_file": env_file.display().to_string(),
        "env": env_vars,
    });
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

// ── down ──────────────────────────────────────────────────────────────────────

fn cmd_down(args: cli::DownArgs) -> Result<()> {
    let log = log::StepLogger::new(args.quiet);

    log.step("Loading config...");
    let (config, root) = config::Config::find_and_load()?;

    // Resolve the target from a read-only snapshot; the interactive worktree
    // prompt below must never run while holding the exclusive lock.
    let slug = {
        let guard = state::StateGuard::acquire_shared(&root)?;
        resolve_slug_from_args(args.slug.as_deref(), &guard.state, "ecluse down <slug>")?
    };

    // Short exclusive section: re-verify the session and mark it pending so
    // the slug + slot stay reserved while teardown runs without the lock.
    // Marking takes over the entry — including from a crashed or still-running
    // up/down, whose finalize will then stand down via the ownership check.
    log.step(&format!("Loading session '{slug}'..."));
    let (session, op_id) = {
        let mut guard = state::StateGuard::acquire(&root)?;
        let (current, op_id) = guard
            .state
            .mark_pending(&slug)
            .ok_or_else(|| error::EcluseError::SessionNotFound(slug.clone()))?;
        if current.status == state::SessionStatus::Pending {
            log.warn(&format!(
                "session '{slug}' has an operation in progress (started {}); taking it over and tearing it down",
                current
                    .pending_op
                    .as_ref()
                    .map(|op| op.since.as_str())
                    .unwrap_or(current.started_at.as_str())
            ));
        }
        guard.commit()?;
        (current, op_id)
    };
    log.detail(&format!("slot {}, mode: {}", session.slot, session.mode));

    let keep_worktree = match resolve_worktree_keep(
        std::path::Path::new(&session.worktree_path),
        args.keep_worktree,
        args.delete_worktree,
    ) {
        Ok(k) => k,
        Err(e) => {
            // Aborted at the prompt — restore the session before bailing out.
            restore_session(&root, &session, &op_id)?;
            return Err(e);
        }
    };

    let result = teardown_or_skip_stopped(
        &session,
        &config,
        &root,
        args.keep_volumes,
        keep_worktree,
        &log,
    );

    if let Err(e) = result {
        // Teardown failed — restore the session so it can be retried. Use the
        // shared helper (remove-then-add under its own lock): `mark_pending`
        // left the Pending entry in place, so a bare add_session would
        // duplicate it. The helper also preserves a pre-existing Stopped status
        // and is a no-op if another command took the session over.
        restore_session(&root, &session, &op_id)?;
        return Err(e);
    }

    let mut guard = state::StateGuard::acquire(&root)?;
    if guard.state.still_owned(&slug, &op_id) {
        if keep_worktree {
            // Services are down but the worktree stays on disk.
            // Mark Stopped so the next `ecluse up` from inside the worktree
            // resumes at this slot rather than allocating a new one.
            guard.state.mark_stopped(&slug)?;
        } else {
            guard.state.remove_session(&slug);
        }
        guard.commit()?;
    } else {
        // Another command took the session over during teardown — leave the
        // entry to its new owner. Teardown itself succeeded, but this invocation
        // no longer owns the outcome, so report only the takeover and stop:
        // printing "torn down" here would contradict the warning.
        drop(guard);
        log.warn(&format!(
            "session '{slug}' was taken over by another command during teardown; leaving its state entry alone"
        ));
        return Ok(());
    }

    if args.keep_branch {
        eprintln!(
            "warning: --keep-branch has no effect; branches are never deleted by ecluse down (branch '{}' is kept)",
            session.branch
        );
    }

    println!();
    if keep_worktree {
        log.success(&format!(
            "Session '{}' torn down (worktree kept at {}).",
            slug, session.worktree_path
        ));
    } else {
        log.success(&format!("Session '{}' torn down.", slug));
    }
    println!();

    Ok(())
}

/// Restore a session that was marked Pending for an operation that then aborted
/// or failed without changing anything durable. A pre-existing Stopped status is
/// preserved (a `down`/`shutdown` on an already stopped session that bailed);
/// otherwise the entry settles back to Active. No-op when `op_id` no longer owns
/// the entry — another command took the session over and restoring would clobber
/// its work.
fn restore_session(root: &std::path::Path, session: &state::Session, op_id: &str) -> Result<()> {
    let mut guard = state::StateGuard::acquire(root)?;
    if !guard.state.still_owned(&session.slug, op_id) {
        return Ok(());
    }
    guard.state.remove_session(&session.slug);
    let mut restored = session.clone();
    // Restore the session's settled status: keep a pre-existing Stopped (a
    // down/shutdown on an already-stopped session that then failed/aborted), but
    // settle everything else to Active. `session` is the snapshot returned by
    // `mark_pending`, so it can be Active, Stopped, OR Pending — the Pending case
    // arises when we took over a crashed in-flight op, and it must become Active
    // (never stay wedged Pending).
    restored.status = match session.status {
        state::SessionStatus::Stopped => state::SessionStatus::Stopped,
        _ => state::SessionStatus::Active,
    };
    restored.pending_op = None;
    guard.state.add_session(restored);
    guard.commit()
}

/// Tear down a session's services, or skip that when it is already Stopped.
///
/// A Stopped session (from a prior `down --keep-worktree`) has no running
/// services and cleared runtime state, so calling `bring_down` would fire
/// pre_down/post_down hooks against nominal ports with nothing behind them.
/// We skip it — but `bring_down` is also the only place the worktree is
/// removed, so when the caller isn't keeping the worktree we remove it here
/// directly, or a `down --delete-worktree` on a Stopped session would orphan
/// the directory. Shared by `cmd_down` and `cmd_shutdown` so the two paths
/// can't drift.
fn teardown_or_skip_stopped(
    session: &state::Session,
    config: &config::Config,
    root: &std::path::Path,
    keep_volumes: bool,
    keep_worktree: bool,
    log: &log::StepLogger,
) -> Result<()> {
    if session.status != state::SessionStatus::Stopped {
        let handler = modes::get_handler_for_mode(&session.mode);
        return handler.bring_down(session, config, root, keep_volumes, keep_worktree, log);
    }

    log.detail("session already stopped — skipping service teardown");
    if !keep_worktree {
        log.step("Removing worktree...");
        log.detail(&session.worktree_path);
        worktree::WorktreeManager::new(root.to_owned())
            .remove(std::path::Path::new(&session.worktree_path))?;
    }
    Ok(())
}

// ── shutdown ──────────────────────────────────────────────────────────────────

fn cmd_shutdown(args: cli::ShutdownArgs) -> Result<()> {
    let log = log::StepLogger::new(args.quiet);

    log.step("Loading config...");
    let (config, root) = config::Config::find_and_load()?;

    // Work from a snapshot; each session is marked pending under a short
    // exclusive section so prompts and teardown never hold the lock.
    let sessions: Vec<state::Session> = {
        let guard = state::StateGuard::acquire_shared(&root)?;
        guard.state.sessions.clone()
    };

    if sessions.is_empty() {
        println!("no active sessions");
        return Ok(());
    }

    let total = sessions.len();
    let mut failed: Vec<String> = Vec::new();
    // Outcome tallies that partition the sessions this run actually acted on:
    // `removed` = torn down and dropped from state; `stopped` = torn down but
    // kept as Stopped (worktree preserved). Counted explicitly rather than
    // derived, so sessions skipped as already-gone or taken over by a concurrent
    // command aren't miscounted as torn down.
    let mut removed = 0usize;
    let mut stopped = 0usize;

    for session in sessions {
        log.step(&format!("Tearing down '{}'...", session.slug));
        log.detail(&format!("slot {}, mode: {}", session.slot, session.mode));

        let keep_wt = match resolve_worktree_keep(
            std::path::Path::new(&session.worktree_path),
            args.keep_worktrees,
            args.delete_worktrees,
        ) {
            Ok(k) => k,
            Err(_) => {
                log.warn(&format!("'{}' skipped (aborted by user)", session.slug));
                failed.push(session.slug.clone());
                continue;
            }
        };

        // Re-verify under the lock (another command may have removed it) and
        // mark pending for the unlocked teardown.
        let (current, op_id) = {
            let mut guard = state::StateGuard::acquire(&root)?;
            match guard.state.mark_pending(&session.slug) {
                None => {
                    log.detail("already removed — skipped");
                    continue;
                }
                Some((current, op_id)) => {
                    guard.commit()?;
                    (current, op_id)
                }
            }
        };

        let teardown =
            teardown_or_skip_stopped(&current, &config, &root, args.keep_volumes, keep_wt, &log);
        match teardown {
            Ok(()) => {
                let mut guard = state::StateGuard::acquire(&root)?;
                if guard.state.still_owned(&current.slug, &op_id) {
                    if keep_wt {
                        // Mirror `cmd_down`: the worktree stays on disk, so
                        // preserve the entry as Stopped to reserve the slot for
                        // the next `ecluse up` rather than dropping it.
                        guard.state.mark_stopped(&current.slug)?;
                        stopped += 1;
                    } else {
                        guard.state.remove_session(&current.slug);
                        removed += 1;
                    }
                    guard.commit()?;
                } else {
                    log.warn(&format!(
                        "'{}' was taken over by another command during teardown; leaving its state entry alone",
                        current.slug
                    ));
                }
            }
            Err(e) => {
                log.warn(&format!("'{}' failed: {}", current.slug, e));
                failed.push(current.slug.clone());
                restore_session(&root, &current, &op_id)?;
            }
        }
    }

    println!();
    // `removed` and `stopped` partition the sessions torn down by this run.
    let torn_down = removed + stopped;
    let kept_note = if stopped > 0 {
        format!(" ({stopped} kept as stopped — worktrees preserved)")
    } else {
        String::new()
    };
    if failed.is_empty() {
        log.success(&format!(
            "Shutdown complete — {} session{} torn down{}.",
            torn_down,
            if torn_down == 1 { "" } else { "s" },
            kept_note
        ));
    } else {
        log.warn(&format!(
            "{}/{} session{} torn down{}; {} failed: {}",
            torn_down,
            total,
            if total == 1 { "" } else { "s" },
            kept_note,
            failed.len(),
            failed.join(", ")
        ));
    }
    println!();

    if !failed.is_empty() {
        eprintln!(
            "hint: if services or containers are still running, try `ecluse flush` to hard-reset all ecluse state"
        );
        return Err(anyhow::anyhow!(
            "shutdown completed with errors; {} session(s) could not be torn down: {}",
            failed.len(),
            failed.join(", ")
        ));
    }

    Ok(())
}

// ── ls ────────────────────────────────────────────────────────────────────────

#[derive(Tabled)]
struct SessionRow {
    #[tabled(rename = "SLUG")]
    slug: String,
    #[tabled(rename = "MODE")]
    mode: String,
    #[tabled(rename = "SLOT")]
    slot: u8,
    #[tabled(rename = "PORTS")]
    ports: String,
    /// Ports actually being listened on by this session's process trees.
    /// `!` marks a set that differs from PORTS — see `listening_summary`.
    #[tabled(rename = "LISTENING")]
    listening: String,
    #[tabled(rename = "TMUX")]
    tmux: String,
    #[tabled(rename = "BRANCH")]
    branch: String,
    #[tabled(rename = "STARTED")]
    started: String,
}

/// Root PIDs whose process trees belong to `session`: the token-verified pid
/// files written at spawn, plus tmux pane PIDs for tmux-managed sessions.
///
/// A pid whose start token no longer matches was recycled by an unrelated
/// process; attributing its ports to this session would be a misattribution of
/// exactly the kind `whose_pid` guards against.
fn session_root_pids(root: &Path, session: &state::Session) -> Vec<u32> {
    let mut pids = Vec::new();

    let pid_dir = root.join(".ecluse").join("pids").join(&session.slug);
    if let Ok(entries) = std::fs::read_dir(&pid_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("pid") {
                continue;
            }
            if let Some((pid, token)) = process::read_pid_file(&path) {
                if process::pid_file_alive(pid, &token) {
                    pids.push(pid);
                }
            }
        }
    }

    if let Some(ref tmux_session) = session.tmux_session {
        pids.extend(process::tmux_session_pane_pids(tmux_session));
    }

    pids.sort_unstable();
    pids.dedup();
    pids
}

/// Ports this session's process trees are actually listening on.
fn discovered_ports(
    root: &Path,
    session: &state::Session,
    snap: &discover::PortSnapshot,
) -> Vec<u16> {
    let mut ports: Vec<u16> = Vec::new();
    for pid in session_root_pids(root, session) {
        for port in snap.ports_for_tree(pid) {
            if !ports.contains(&port) {
                ports.push(port);
            }
        }
    }
    ports.sort_unstable();
    ports
}

/// Ports a session's own process trees are expected to hold: the assigned
/// ports of the native services ecluse spawns (those with a `command`).
///
/// `port_overrides` can't be used wholesale: in hybrid mode it also carries the
/// Docker service ports, which are held by the Docker daemon/proxy and never by
/// the session's process tree, so they would always look missing.
fn expected_native_ports(config: &config::Config, session: &state::Session) -> Vec<u16> {
    let mut ports: Vec<u16> = config
        .services
        .iter()
        .filter(|svc| svc.run == config::ServiceRun::Native && svc.command.is_some())
        .filter_map(|svc| native_expected_port(config, session, svc))
        .collect();
    ports.sort_unstable();
    ports.dedup();
    ports
}

/// The port ecluse assigned to a native service in `session`.
///
/// `port_overrides` is the source of truth; old state files without an entry
/// for a native service fall back to the nominal port computed from config.
fn native_expected_port(
    config: &config::Config,
    session: &state::Session,
    svc: &config::ServiceConfig,
) -> Option<u16> {
    session.port_overrides.get(&svc.name).copied().or_else(|| {
        if svc.base_port == 0 {
            None
        } else {
            Some(svc.port(session.slot, config.slot_stride))
        }
    })
}

/// True when the session is listening on something, but not on every port
/// ecluse assigned. Extra ports beyond the assigned set are normal (HMR, debug,
/// inspector) and never count as a mismatch.
fn port_mismatch(expected: &[u16], discovered: &[u16]) -> bool {
    !discovered.is_empty() && expected.iter().any(|e| !discovered.contains(e))
}

/// Render the LISTENING column: the discovered ports, with a trailing `!` when
/// they don't match what ecluse assigned.
///
/// Only ports ecluse allocated participate in the comparison. A dev server that
/// also opens an HMR or debug socket would otherwise show a permanent mismatch.
fn listening_summary(expected: &[u16], discovered: &[u16]) -> String {
    if discovered.is_empty() {
        return "-".into();
    }
    let rendered = discovered
        .iter()
        .map(|p| p.to_string())
        .collect::<Vec<_>>()
        .join(" ");
    if port_mismatch(expected, discovered) {
        format!("{} !", rendered)
    } else {
        rendered
    }
}

fn cmd_ls(args: cli::LsArgs) -> Result<()> {
    let (config, root) = config::Config::find_and_load()?;
    let guard = state::StateGuard::acquire_shared(&root)?;

    if guard.state.sessions.is_empty() {
        println!("no active sessions");
        return Ok(());
    }

    // One listener/process-table snapshot shared by every session.
    let snap = discover::snapshot();

    if args.json {
        // Sessions serialize as-is, plus the discovered view alongside the
        // assigned one. Never merged into port_overrides: state stays truth.
        let sessions_json: Vec<serde_json::Value> = guard
            .state
            .sessions
            .iter()
            .map(|s| {
                let expected = expected_native_ports(&config, s);
                let listening = discovered_ports(&root, s, &snap);
                let mut value = serde_json::to_value(s)?;
                if let Some(obj) = value.as_object_mut() {
                    obj.insert("listening_ports".into(), serde_json::json!(listening));
                    obj.insert(
                        "port_mismatch".into(),
                        serde_json::json!(port_mismatch(&expected, &listening)),
                    );
                }
                Ok(value)
            })
            .collect::<Result<Vec<_>>>()?;
        println!("{}", serde_json::to_string_pretty(&sessions_json)?);
        return Ok(());
    }

    let rows: Vec<SessionRow> = guard
        .state
        .sessions
        .iter()
        .map(|s| {
            let mut pairs: Vec<String> = s
                .port_overrides
                .iter()
                .map(|(k, v)| format!("{}={}", k, v))
                .collect();
            pairs.sort();
            let ports = if pairs.is_empty() {
                "-".into()
            } else {
                pairs.join(" ")
            };
            let expected = expected_native_ports(&config, s);
            let listening = listening_summary(&expected, &discovered_ports(&root, s, &snap));
            SessionRow {
                listening,
                slug: match s.status {
                    state::SessionStatus::Pending => format!("{} (pending)", s.slug),
                    state::SessionStatus::Stopped => format!("{} (stopped)", s.slug),
                    state::SessionStatus::Active => s.slug.clone(),
                },
                mode: s.mode.to_string(),
                slot: s.slot,
                ports,
                tmux: s.tmux_session.clone().unwrap_or_default(),
                branch: s.branch.clone(),
                started: s
                    .started_at
                    .get(..16)
                    .unwrap_or(&s.started_at)
                    .replace('T', " "),
            }
        })
        .collect();

    let any_tmux = guard
        .state
        .sessions
        .iter()
        .any(|s| s.tmux_session.is_some());
    let mut table = Table::new(rows);
    {
        use tabled::settings::object::Columns;
        use tabled::settings::{Modify, Width};
        // Truncate PORTS (col 3) to 40 chars so long port lists don't wrap the header.
        table.with(Modify::new(Columns::single(3)).with(Width::truncate(40).suffix("…")));
        // Same for LISTENING (col 4).
        table.with(Modify::new(Columns::single(4)).with(Width::truncate(40).suffix("…")));
    }
    if !any_tmux {
        use tabled::settings::object::Columns;
        use tabled::settings::Disable;
        // TMUX is column index 5
        // (SLUG=0, MODE=1, SLOT=2, PORTS=3, LISTENING=4, TMUX=5)
        table.with(Disable::column(Columns::single(5)));
    }
    println!("{}", table);

    let log = log::StepLogger::new(false);
    for s in &guard.state.sessions {
        for w in process::check_processes_alive(&s.process_manager, &s.spawn_result(), &s.slug) {
            log.warn(&format!("[{}] {}", s.slug, w));
        }
        // A pending entry only lives for the duration of one up/down; one that
        // sticks around means the owning command crashed and the slot leaks.
        if let Some(op) = &s.pending_op {
            if let Ok(since) = chrono::DateTime::parse_from_rfc3339(&op.since) {
                let age = chrono::Utc::now().signed_duration_since(since);
                if age > chrono::Duration::minutes(15) {
                    log.warn(&format!(
                        "session '{}' has been pending for {} minutes — if the owning ecluse command crashed, run `ecluse down {}` to clean it up and free slot {}",
                        s.slug,
                        age.num_minutes(),
                        s.slug,
                        s.slot
                    ));
                }
            }
        }
    }

    Ok(())
}

// ── shell ─────────────────────────────────────────────────────────────────────

fn cmd_shell(args: cli::ShellArgs) -> Result<()> {
    let (_, root) = config::Config::find_and_load()?;
    let guard = state::StateGuard::acquire_shared(&root)?;

    let slug = resolve_slug_from_args(args.slug.as_deref(), &guard.state, "ecluse shell <slug>")?;

    let session = guard
        .state
        .find_session(&slug)
        .ok_or_else(|| error::EcluseError::SessionNotFound(slug.clone()))?
        .clone();
    ensure_session_settled(&session)?;

    let worktree = std::path::Path::new(&session.worktree_path);
    let env_file = worktree.join(".env.ecluse");
    let env_vars: Vec<(String, String)> = env::parse_env_file(&env_file);

    if let Some(tmux_session) = &session.tmux_session {
        println!(
            "Attaching to tmux session '{}' for ecluse session '{}'.",
            tmux_session, session.slug
        );
        let status = std::process::Command::new("tmux")
            .args(["attach", "-t", tmux_session])
            .status()
            .context("failed to attach to tmux session")?;
        std::process::exit(status.code().unwrap_or(0));
    }

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());

    println!(
        "Entering ecluse session '{}' (slot {}).",
        session.slug, session.slot
    );
    println!("Type 'exit' to leave.\n");

    let status = std::process::Command::new(&shell)
        .current_dir(worktree)
        .envs(env_vars)
        .status()
        .with_context(|| format!("failed to launch shell: {}", shell))?;

    std::process::exit(status.code().unwrap_or(0));
}

// ── validate ──────────────────────────────────────────────────────────────────

fn cmd_validate(args: cli::ValidateArgs) -> Result<()> {
    let log = log::StepLogger::new(args.quiet);

    log.step("Loading config...");
    let (config, root) = config::Config::find_and_load()?;
    log.detail(&format!("mode: {}", config.mode));

    log.step("Checking port ranges...");
    let warnings = validate::validate_config(&config)?;
    for w in &warnings {
        log.warn(w);
    }

    log.step("Checking process manager...");
    let global = process::load_global_config()?;
    validate::validate_process_manager(&global.process_manager)?;
    log.detail(&global.process_manager.to_string());

    println!();
    log.success(&format!(
        "Config at {} is valid.",
        root.join(".ecluse.toml").display()
    ));
    println!();
    println!("  max_slots:         {}", config.max_slots);
    println!("  strict_port:       {}", config.strict_port);
    println!("  port_search_range: {}", config.port_search_range);
    println!("  process_manager:   {}", global.process_manager);

    if !config.services.is_empty() {
        println!("  services:");
        for svc in &config.services {
            println!(
                "    {} ({}) base_port={} slot_1_port={}",
                svc.name,
                svc.run,
                svc.base_port,
                svc.port(1, config.slot_stride),
            );
        }
    }

    if args.ports {
        println!();
        println!("Port allocation across all slots:");
        let header_parts: Vec<String> = config
            .services
            .iter()
            .map(|s| format!("{:>20}", s.name))
            .collect();
        println!("  {:>6}  {}", "slot", header_parts.join("  "));
        for slot in 1..=config.max_slots {
            let port_parts: Vec<String> = config
                .services
                .iter()
                .map(|s| format!("{:>20}", s.port(slot, config.slot_stride)))
                .collect();
            println!("  {:>6}  {}", slot, port_parts.join("  "));
        }
    }

    Ok(())
}

// ── env ───────────────────────────────────────────────────────────────────────

fn cmd_env(args: cli::EnvArgs) -> Result<()> {
    let (_, root) = config::Config::find_and_load()?;
    let guard = state::StateGuard::acquire_shared(&root)?;

    let slug = resolve_slug_from_args(args.slug.as_deref(), &guard.state, "ecluse env <slug>")?;
    let session = guard
        .state
        .find_session(&slug)
        .ok_or_else(|| error::EcluseError::SessionNotFound(slug.clone()))?
        .clone();
    ensure_session_settled(&session)?;

    let env_file = std::path::Path::new(&session.worktree_path).join(".env.ecluse");

    let mut env_vars: serde_json::Map<String, serde_json::Value> = serde_json::Map::new();
    for (k, v) in env::parse_env_file(&env_file) {
        env_vars.insert(k, serde_json::Value::String(v));
    }

    let out = serde_json::json!({
        "slug": session.slug,
        "slot": session.slot,
        "mode": session.mode.to_string(),
        "branch": session.branch,
        "worktree_path": session.worktree_path,
        "env_file": env_file.display().to_string(),
        "env": env_vars,
    });
    println!("{}", serde_json::to_string_pretty(&out)?);

    let log = log::StepLogger::new(false);
    for w in process::check_processes_alive(
        &session.process_manager,
        &session.spawn_result(),
        &session.slug,
    ) {
        log.warn(&w);
    }

    Ok(())
}

// ── sync ──────────────────────────────────────────────────────────────────────

fn cmd_sync(args: cli::SyncArgs) -> Result<()> {
    let log = log::StepLogger::new(args.quiet || args.json);

    log.step("Loading config...");
    let (config, root) = config::Config::find_and_load()?;
    log.detail(&format!("mode: {}", config.mode));

    let wt_manager = worktree::WorktreeManager::new(root.clone());

    // Resolve slug and worktree path together.
    let (slug, worktree_path) = match args.slug {
        Some(ref s) => {
            validate_slug(s)?;
            let canonical = wt_manager.worktree_path(&config, s);
            let path = if canonical.exists() {
                canonical
            } else {
                // Fall back to the cwd only when it really is a linked git
                // worktree of this repo — never on path-string coincidences.
                let cwd =
                    std::env::current_dir().context("could not determine current directory")?;
                let belongs = worktree::is_inside_git_worktree(&cwd)
                    && worktree::WorktreeManager::main_worktree_root(&cwd)
                        .ok()
                        .and_then(|r| std::fs::canonicalize(r).ok())
                        == std::fs::canonicalize(&root).ok();
                if belongs {
                    worktree::git_worktree_root(&cwd)?
                } else {
                    return Err(error::EcluseError::WorktreeNotFound { slug: s.clone() }.into());
                }
            };
            (s.clone(), path)
        }
        None => {
            // Derive slug from cwd: must be inside <worktree_dir>/<slug>
            let cwd = std::env::current_dir().context("could not determine current directory")?;
            let wt_root = root.join(&config.worktree_dir);
            let rel = cwd.strip_prefix(&wt_root).map_err(|_| {
                anyhow::anyhow!(
                    "not inside an ecluse worktree; pass a slug or cd into a worktree\n  hint: ecluse sync <slug>"
                )
            })?;
            let slug = rel
                .components()
                .next()
                .and_then(|c| c.as_os_str().to_str())
                .ok_or_else(|| anyhow::anyhow!("could not determine slug from cwd"))?
                .to_string();
            validate_slug(&slug)?;
            let worktree_path = wt_root.join(&slug);
            (slug, worktree_path)
        }
    };

    log.detail(&format!("worktree: {}", worktree_path.display()));

    // Acquire state lock.
    let mut guard = state::StateGuard::acquire(&root)?;
    let existing = guard.state.find_session(&slug).cloned();
    if let Some(ref s) = existing {
        // Never overwrite an entry another command is mid-operating on.
        ensure_session_settled(s)?;
    }
    let update_mode = existing.is_some();

    // Allocate or reuse slot.
    let slot = match &existing {
        Some(s) => s.slot,
        None => {
            log.step("Allocating slot...");
            let allocator = slot::SlotAllocator::new(&config, &guard.state);
            let s = allocator.allocate_next()?;
            log.detail(&format!("slot {s}"));
            s
        }
    };

    // Determine current branch.
    let branch = {
        let output = std::process::Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .current_dir(&worktree_path)
            .output()
            .context("failed to run git rev-parse")?;
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    };

    // Discover native processes.
    log.step("Discovering processes in worktree...");
    let discovered = sync::find_processes_in_worktree(&worktree_path);
    log.detail(&format!("found {} process(es)", discovered.len()));

    let native_svcs: Vec<&config::ServiceConfig> = config
        .services
        .iter()
        .filter(|s| s.run == config::ServiceRun::Native)
        .collect();

    let docker_svcs: Vec<&config::ServiceConfig> = config
        .services
        .iter()
        .filter(|s| s.run == config::ServiceRun::Docker)
        .collect();

    // Match native services to discovered processes.
    log.step("Matching services...");
    let native_matches = sync::match_services(&native_svcs, &discovered);

    // Detect docker services.
    let docker_matches = if !docker_svcs.is_empty() {
        log.step("Detecting docker services...");
        sync::find_docker_services(&docker_svcs, &modes::compose_project_name(&config, &slug))
    } else {
        vec![]
    };

    if native_matches.is_empty() && docker_matches.is_empty() {
        return Err(error::EcluseError::NoProcessesFound {
            path: worktree_path.display().to_string(),
        }
        .into());
    }

    // Warn about unmatched native services.
    for svc in &native_svcs {
        if svc.command.is_some() && !native_matches.iter().any(|m| m.service_name == svc.name) {
            log.warn(&format!(
                "could not find a running process for service '{}' (command: {})",
                svc.name,
                svc.command.as_deref().unwrap_or(""),
            ));
        }
    }

    // Write PID files and collect port_overrides.
    let ecluse_dir = root.join(".ecluse");
    let mut port_overrides: std::collections::HashMap<String, u16> =
        std::collections::HashMap::new();
    let mut pid_files: Vec<std::path::PathBuf> = vec![];

    for m in &native_matches {
        log.detail(&format!(
            "service '{}': PID {} port {:?}",
            m.service_name, m.pid, m.port
        ));
        let pid_path = sync::write_pid_file(&ecluse_dir, &slug, &m.service_name, m.pid)?;
        pid_files.push(pid_path);
        if let Some(port) = m.port {
            port_overrides.insert(m.service_name.clone(), port);
        }
    }
    for (name, port) in &docker_matches {
        log.detail(&format!("docker service '{}': port {}", name, port));
        port_overrides.insert(name.clone(), *port);
    }

    // Build and write .env.ecluse.
    log.step("Writing .env.ecluse...");
    let mut native_ports = indexmap::IndexMap::new();
    for m in &native_matches {
        if let Some(port) = m.port {
            native_ports.insert(m.service_name.clone(), port);
        }
    }
    let all_svc_configs: Vec<&config::ServiceConfig> = native_svcs
        .iter()
        .chain(config.docker_services().iter())
        .copied()
        .collect();
    let env_map = env::build_env(
        slot,
        config.slot_stride,
        &slug,
        &config.mode.to_string(),
        &native_ports,
        &docker_matches,
        &all_svc_configs,
    );
    env::write_env_file(&worktree_path, &env_map)?;

    let app_port = native_matches
        .first()
        .and_then(|m| m.port)
        .or_else(|| docker_matches.first().map(|(_, p)| *p));

    let session = state::Session {
        slug: slug.clone(),
        mode: config.mode.clone(),
        slot,
        branch,
        worktree_path: worktree_path.display().to_string(),
        status: state::SessionStatus::Active,
        pending_op: None,
        app_port,
        port_overrides,
        process_manager: Some(process::ProcessManager::Nohup),
        pid_files,
        log_dir: None,
        compose_project: None,
        overlay_file: None,
        overlay_files: vec![],
        compose_overlays: vec![],
        started_at: chrono::Utc::now().to_rfc3339(),
        tmux_session: None,
        services_subset: None,
    };

    if update_mode {
        guard.state.remove_session(&slug);
        log.step("Updating existing session...");
    } else {
        log.step("Registering new session...");
    }
    guard.state.add_session(session.clone());
    guard.commit()?;

    if args.json {
        print_up_json(&session, &root)?;
    } else {
        println!();
        log.success(&format!(
            "Session '{}' synced (slot {})",
            session.slug, session.slot
        ));
        println!();
        println!("  Worktree:  {}", session.worktree_path);
        println!("  Mode:      {}", session.mode);
        println!("  Branch:    {}", session.branch);
        if let Some(port) = session.app_port {
            println!("  App port:  {}", port);
        }
        println!();
        println!(
            "  Services synced: {}",
            if native_matches.is_empty() && docker_matches.is_empty() {
                "none".to_string()
            } else {
                native_matches
                    .iter()
                    .map(|m| {
                        m.port
                            .map(|p| format!("{}:{}", m.service_name, p))
                            .unwrap_or_else(|| format!("{}:(no port)", m.service_name))
                    })
                    .chain(
                        docker_matches
                            .iter()
                            .map(|(n, p)| format!("{}:{} (docker)", n, p)),
                    )
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        );
        println!();
    }

    Ok(())
}

// ── flush ─────────────────────────────────────────────────────────────────────

fn cmd_flush(args: cli::FlushArgs) -> Result<()> {
    let log = log::StepLogger::new(args.quiet);

    log.step("Loading config...");
    let (config, root) = config::Config::find_and_load()?;

    if !args.yes {
        print!(
            "This will destroy all ecluse sessions, worktrees, and running services.\n\
             It will also KILL every process with a file open inside the worktrees \
             (including editors, shells, and `tail -f` against worktree logs) and \
             every process listening on a configured port.\n\
             There is no undo. Continue? [y/N] "
        );
        std::io::stdout().flush()?;
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Aborted.");
            return Ok(());
        }
    }

    // Step 1: graceful shutdown of known sessions (best-effort).
    {
        let mut guard = state::StateGuard::acquire(&root)?;
        let sessions: Vec<state::Session> = guard.state.sessions.clone();
        if !sessions.is_empty() {
            log.step(&format!(
                "Tearing down {} known session(s)...",
                sessions.len()
            ));
            for session in sessions {
                log.detail(&format!("  down '{}'", session.slug));
                // Shared helper skips bring_down (and its hooks) for Stopped
                // sessions with nothing running, and removes the worktree
                // (flush always deletes worktrees) — same decision cmd_down uses.
                if let Err(e) =
                    teardown_or_skip_stopped(&session, &config, &root, false, false, &log)
                {
                    log.warn(&format!(
                        "  '{}' teardown failed: {e} (continuing)",
                        session.slug
                    ));
                }
                guard.state.remove_session(&session.slug);
            }
            guard.commit()?;
        }
    }

    // Step 2: kill orphaned tmux sessions named ecluse-*.
    if process::binary_available("tmux") {
        log.step("Killing orphaned tmux sessions...");
        let output = std::process::Command::new("tmux")
            .args(["list-sessions", "-F", "#{session_name}"])
            .output();
        if let Ok(out) = output {
            let stdout = String::from_utf8_lossy(&out.stdout);
            for name in stdout.lines() {
                let name = name.trim();
                if name.starts_with("ecluse-") {
                    log.detail(&format!("  kill tmux session '{name}'"));
                    if let Err(e) = std::process::Command::new("tmux")
                        .args(["kill-session", "-t", name])
                        .status()
                    {
                        log.warn(&format!("  could not kill tmux session '{name}': {e}"));
                    }
                }
            }
        }
    }

    // Step 3: stop orphaned docker compose projects matching <prefix>_*.
    if docker::is_available() {
        log.step("Stopping orphaned docker compose projects...");
        let projects = docker::list_compose_projects(&config.prefix);
        for project in projects {
            log.detail(&format!("  compose down -p '{project}'"));
            if let Err(e) = docker::compose_down_by_project(&project, false) {
                log.warn(&format!("  could not stop project '{project}': {e}"));
            }
        }
    }

    // Step 3a: sweep every process whose cwd is inside a worktree. Step 1
    // killed services tracked in state.json; this catches detached descendants
    // (workerd, vite plugins that setsid()) and processes that crashed out of
    // a recorded session. Runs BEFORE worktree removal so git worktree remove
    // doesn't race a live process holding file handles.
    let worktree_dir_path = root.join(&config.worktree_dir);
    if worktree_dir_path.exists() {
        log.step("Sweeping stray processes with cwd in worktrees...");
        if let Ok(entries) = std::fs::read_dir(&worktree_dir_path) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                for pid in sync::pids_in_directory(&path) {
                    // Skip our own pid — flush runs from inside the repo and
                    // would otherwise commit suicide on the first sweep.
                    if pid == std::process::id() {
                        continue;
                    }
                    log.detail(&format!(
                        "  kill -TERM -- -{} (cwd {})",
                        pid,
                        path.display()
                    ));
                    process::kill_process_group_with_grace(pid);
                }
            }
        }
    }

    // Step 3b: sweep every listener on any port the config can allocate. This
    // catches orphans that no longer have an open file inside the worktree
    // (e.g. a daemonized process that chdir'd to /) but are still holding a
    // port from the configured range.
    log.step("Sweeping listeners on configured ports...");
    let mut swept_listener_pids: std::collections::HashSet<u32> = Default::default();
    for svc in &config.services {
        for slot in 1..=config.max_slots {
            // Primary port (covers host_port override).
            let primary = svc.port(slot, config.slot_stride);
            // Extra ports (debugger sockets, secondary listeners).
            let extras: Vec<u16> = svc
                .extra_ports
                .iter()
                .map(|ep| {
                    ep.base_port.saturating_add(
                        (slot as u16).saturating_mul(config.slot_stride.max(1) as u16),
                    )
                })
                .collect();
            for port in std::iter::once(primary).chain(extras) {
                if let Some(pid) = validate::port_listener(port) {
                    if pid == 0 || pid == std::process::id() {
                        continue;
                    }
                    if !swept_listener_pids.insert(pid) {
                        continue;
                    }
                    log.detail(&format!("  kill -TERM -- -{} (port {})", pid, port));
                    process::kill_process_group_with_grace(pid);
                }
            }
        }
    }

    // Step 4: remove all worktrees under worktree_dir.
    let worktree_dir = root.join(&config.worktree_dir);
    if worktree_dir.exists() {
        log.step("Removing worktrees...");
        if let Ok(entries) = std::fs::read_dir(&worktree_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    log.detail(&format!("  git worktree remove --force {}", path.display()));
                    let _ = std::process::Command::new("git")
                        .args(["worktree", "remove", "--force", &path.display().to_string()])
                        .current_dir(&root)
                        .status();
                }
            }
        }
        let _ = std::process::Command::new("git")
            .args(["worktree", "prune"])
            .current_dir(&root)
            .status();
    }

    // Step 5: wipe .ecluse subdirs.
    let ecluse_dir = root.join(".ecluse");
    for subdir in &["pids", "logs", "overlays", "preambles"] {
        let path = ecluse_dir.join(subdir);
        if path.exists() {
            log.detail(&format!("  remove {}", path.display()));
            if let Err(e) = std::fs::remove_dir_all(&path) {
                log.warn(&format!("  could not remove {}: {e}", path.display()));
            }
        }
    }

    // Step 6: reset state.json.
    log.step("Resetting state.json...");
    let mut guard = state::StateGuard::acquire(&root)?;
    guard.state = state::State::default();
    guard.commit().context("failed to reset state.json")?;

    println!();
    log.success("Flush complete — ecluse is in a clean state.");
    println!();

    Ok(())
}

// ── status ────────────────────────────────────────────────────────────────────

struct ServiceStatus {
    name: String,
    kind: &'static str,
    port: Option<u16>,
    healthy: bool,
    /// False for port-allocation-only native services (no command) — ecluse
    /// never spawns them, so their health is not ecluse's responsibility.
    managed: bool,
    pid: Option<u32>,
    tmux_window: Option<String>,
    /// PID of whatever process is actually listening on `port`, if any.
    /// Only populated for native services; docker port mappings are owned by
    /// the daemon, not the container process, so the check doesn't apply.
    listener_pid: Option<u32>,
    /// True iff a listener is bound to `port` AND that listener is neither
    /// `pid` nor a descendant of it. A stale orphan from a previous session
    /// hijacking the port — `ecluse status` reports the service as down
    /// even though something IS responding to requests.
    wrong_owner: bool,
    /// The port this service's process tree is *actually* listening on, when
    /// it differs from the assigned `port`. Populated by discovery; never
    /// written back to state — a mismatch is a bug to report, not a value to
    /// adopt (see `incidents/2026-06-09-rubbr-cross-agent-kill-spiral`).
    actual_port: Option<u16>,
    /// Set when `actual_port` falls inside another slot's territory. Names the
    /// slot and, when a session holds it, that session's slug — the one fact
    /// that stops an agent from killing a sibling's service.
    conflicting_slot: Option<(u8, Option<String>)>,
    /// Every port this service's own process tree is listening on, from the
    /// shared snapshot. Includes extra sockets (HMR, debug, inspector).
    listening_ports: Vec<u16>,
}

/// What the shared port snapshot says about one native service.
#[derive(Debug, Default, PartialEq)]
struct ServiceDiscovery {
    /// The service's tree is alive and (when a port is assigned) holds it.
    healthy: bool,
    /// Every port the service's tree is listening on.
    tree_ports: Vec<u16>,
    /// Whoever holds the assigned port, if anyone.
    listener_pid: Option<u32>,
    /// The assigned port is held by a process outside the service's tree.
    wrong_owner: bool,
    /// The tree does not hold the assigned port but listens on this one.
    wrong_port: Option<u16>,
}

/// Root pid of one native service's process tree.
///
/// The service's pid file wins when it exists, and its pid counts only if the
/// recorded start token still matches — a recycled pid belongs to an unrelated
/// process whose ports must not be attributed to this service (same check as
/// `session_root_pids`). Legacy tmux sessions without pid files fall back to
/// the pane pid of the service's own window — never every pane in the session,
/// which would blame one service for a sibling's port.
fn service_tree_root(pid_file: &Path, tmux_session: Option<&str>, window: &str) -> Option<u32> {
    if let Some((pid, token)) = process::read_pid_file(pid_file) {
        return process::pid_file_alive(pid, &token).then_some(pid);
    }
    tmux_session.and_then(|t| sync::tmux_pane_pid(t, window))
}

/// Classify one native service against the shared snapshot.
///
/// `expect_owner` is true when ecluse recorded a process for this service, so
/// a foreign listener on the assigned port means someone else holds it.
///
/// "Wrong port" is reported only when the tree does NOT hold the assigned port
/// and does listen on something else. When the assigned port is held, extra
/// sockets are normal and are reported in `tree_ports` without flagging
/// anything — the same rule `ecluse ls` applies.
fn discover_service(
    snap: &discover::PortSnapshot,
    root: Option<u32>,
    expect_owner: bool,
    port: Option<u16>,
) -> ServiceDiscovery {
    let tree_ports = root.map(|r| snap.ports_for_tree(r)).unwrap_or_default();
    let healthy = match (root, port) {
        (None, _) => false,
        (Some(_), None) => true,
        (Some(_), Some(p)) => tree_ports.contains(&p),
    };
    let listener_pid = if expect_owner {
        port.and_then(|p| snap.listener_pid(p))
    } else {
        None
    };
    let wrong_owner = match listener_pid {
        Some(actual) => root.is_none_or(|r| actual != r && !snap.is_descendant(r, actual)),
        None => false,
    };
    let wrong_port = match port {
        Some(p) if !tree_ports.contains(&p) => tree_ports.first().copied(),
        _ => None,
    };
    ServiceDiscovery {
        healthy,
        tree_ports,
        listener_pid,
        wrong_owner,
        wrong_port,
    }
}

/// Human-readable ACTUAL column: every port the service's tree is listening
/// on, or `—` when that is nothing or exactly the assigned port (nothing
/// interesting to report).
fn actual_str(s: &ServiceStatus) -> String {
    let only_assigned = s.port.is_some_and(|p| s.listening_ports == [p]);
    if s.listening_ports.is_empty() || only_assigned {
        return "\u{2014}".into();
    }
    s.listening_ports
        .iter()
        .map(|p| p.to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The remediation hint for a wrong-port service.
///
/// Always points at `down --keep-worktree` + `up`, which is idempotent, only
/// touches this session's own services, and re-probes ports. Never suggests
/// `kill`: under parallel sessions the process on a neighbouring port is
/// almost always another agent's working service.
fn mismatch_hint(slug: &str, s: &ServiceStatus) -> Option<String> {
    let actual = s.actual_port?;
    let reset = format!(
        "run: ecluse down {} --keep-worktree && ecluse up {}",
        slug, slug
    );
    Some(match &s.conflicting_slot {
        Some((slot, Some(owner))) => format!(
            "service '{}' is listening on {} but ecluse assigned {}; {} belongs to slot {} \
             (session '{}') — do not kill it, {}",
            s.name,
            actual,
            s.port.map(|p| p.to_string()).unwrap_or_else(|| "-".into()),
            actual,
            slot,
            owner,
            reset
        ),
        Some((slot, None)) => format!(
            "service '{}' is listening on {} but ecluse assigned {}; {} is slot {}'s territory — {}",
            s.name,
            actual,
            s.port.map(|p| p.to_string()).unwrap_or_else(|| "-".into()),
            actual,
            slot,
            reset
        ),
        None => format!(
            "service '{}' is listening on {} but ecluse assigned {}; {}",
            s.name,
            actual,
            s.port.map(|p| p.to_string()).unwrap_or_else(|| "-".into()),
            reset
        ),
    })
}

/// Human-readable status string for a service row. Extracted from cmd_status
/// so the wrong-owner branch can be unit-tested.
fn status_str(s: &ServiceStatus) -> String {
    if !s.managed {
        "\u{2014}".into() // — port-only, not ecluse-managed
    } else if s.wrong_owner {
        // A different process owns the configured port — likely an orphan from
        // a previous session. The service is "down" from ecluse's perspective
        // even if something IS responding.
        match s.listener_pid {
            Some(pid) => format!("\u{2717} wrong owner (PID {})", pid),
            None => "\u{2717} wrong owner".into(),
        }
    } else if let Some(actual) = s.actual_port {
        // Discovery found the service alive on a different port than the one
        // ecluse assigned. Reporting a bare "down" here is what left agents
        // guessing (and reaching for `kill`) in the 2026-06-09 incident.
        match &s.conflicting_slot {
            Some((slot, _)) => format!("\u{2717} wrong port {} (slot {})", actual, slot),
            None => format!("\u{2717} wrong port {}", actual),
        }
    } else if s.healthy {
        "\u{2713} up".into()
    } else {
        "\u{2717} down".into()
    }
}

#[derive(Tabled)]
struct StatusRowTmux {
    #[tabled(rename = "SERVICE")]
    service: String,
    #[tabled(rename = "TYPE")]
    kind: String,
    #[tabled(rename = "EXPECTED")]
    port: String,
    #[tabled(rename = "ACTUAL")]
    actual: String,
    #[tabled(rename = "STATUS")]
    status: String,
    #[tabled(rename = "WINDOW")]
    window: String,
}

#[derive(Tabled)]
struct StatusRowNohup {
    #[tabled(rename = "SERVICE")]
    service: String,
    #[tabled(rename = "TYPE")]
    kind: String,
    #[tabled(rename = "EXPECTED")]
    port: String,
    #[tabled(rename = "ACTUAL")]
    actual: String,
    #[tabled(rename = "STATUS")]
    status: String,
    #[tabled(rename = "PID")]
    pid: String,
}

#[derive(Tabled)]
struct StatusRowNone {
    #[tabled(rename = "SERVICE")]
    service: String,
    #[tabled(rename = "TYPE")]
    kind: String,
    #[tabled(rename = "EXPECTED")]
    port: String,
    #[tabled(rename = "ACTUAL")]
    actual: String,
    #[tabled(rename = "STATUS")]
    status: String,
}

fn cmd_status(args: cli::StatusArgs) -> Result<()> {
    let (config, root) = config::Config::find_and_load()?;
    let guard = state::StateGuard::acquire_shared(&root)?;

    let slug = resolve_slug_from_args(args.slug.as_deref(), &guard.state, "ecluse status <slug>")?;
    let session = guard
        .state
        .find_session(&slug)
        .ok_or_else(|| error::EcluseError::SessionNotFound(slug.clone()))?
        .clone();
    ensure_session_settled(&session)?;

    // Build per-service health status.
    let native_svcs: Vec<&config::ServiceConfig> = config
        .services
        .iter()
        .filter(|s| s.run == config::ServiceRun::Native)
        .collect();

    let docker_svcs: Vec<&config::ServiceConfig> = config
        .services
        .iter()
        .filter(|s| s.run == config::ServiceRun::Docker)
        .collect();

    // Docker: find running containers.
    let docker_matches = if !docker_svcs.is_empty() {
        sync::find_docker_services(
            &docker_svcs,
            &modes::compose_project_name(&config, &session.slug),
        )
    } else {
        vec![]
    };

    // One snapshot shared by every service row.
    let snap = discover::snapshot();
    // Slot → slug, so a cross-slot port can name the session that owns it.
    let slot_owners: std::collections::HashMap<u8, String> = guard
        .state
        .sessions
        .iter()
        .map(|s| (s.slot, s.slug.clone()))
        .collect();

    let mut statuses: Vec<ServiceStatus> = Vec::new();

    for svc in &native_svcs {
        // The expected port is the source of truth — it's what ecluse allocated and
        // wrote into .env.ecluse. Never substitute a "discovered" port here:
        // if the process tree contains a listener on a different port (e.g. a
        // child task spawned its own server), trusting that port would make
        // `status` lie about what's actually wired up.
        let expected_port = native_expected_port(&config, &session, svc);

        // Identity first: the session's own pid file (token-verified) or tmux
        // window decides health — never a host-wide scan that can misattribute
        // a neighbor's process. `recorded_pid` is what the pid file says (shown
        // in the PID column); `root` is that pid only if it is still the same
        // process incarnation, so a recycled pid never has ports attributed.
        let pid_file = root
            .join(".ecluse")
            .join("pids")
            .join(&session.slug)
            .join(format!("{}.pid", svc.name));
        let recorded_pid = process::read_pid_file(&pid_file).map(|(pid, _)| pid);
        let tree_root = service_tree_root(&pid_file, session.tmux_session.as_deref(), &svc.name);
        let (pid, port) = (recorded_pid, expected_port);
        let tmux_window = if matches!(session.process_manager, Some(process::ProcessManager::Tmux))
        {
            Some(svc.name.clone())
        } else {
            None
        };
        // Port-allocation-only services (no command) are never spawned by
        // ecluse — don't report them as down.
        let managed = svc.command.is_some();

        let disc = discover_service(&snap, tree_root, pid.is_some() || tree_root.is_some(), port);
        let wrong_owner = managed && disc.wrong_owner;
        let listener_pid = if managed { disc.listener_pid } else { None };
        let healthy_with_owner_check = disc.healthy && !wrong_owner;

        let (actual_port, conflicting_slot) = if managed {
            let conflict = disc.wrong_port.and_then(|a| {
                discover::owning_slot(
                    a,
                    svc.host_port_base(),
                    config.slot_stride,
                    config.max_slots,
                )
                .filter(|slot| *slot != session.slot)
                .map(|slot| (slot, slot_owners.get(&slot).cloned()))
            });
            (disc.wrong_port, conflict)
        } else {
            (None, None)
        };
        let listening_ports = if managed { disc.tree_ports } else { vec![] };

        statuses.push(ServiceStatus {
            name: svc.name.clone(),
            kind: "native",
            port,
            healthy: healthy_with_owner_check || !managed,
            managed,
            pid,
            tmux_window,
            listener_pid,
            wrong_owner,
            actual_port,
            conflicting_slot,
            listening_ports,
        });
    }

    for svc in &docker_svcs {
        let found_port = docker_matches
            .iter()
            .find(|(name, _)| name == &svc.name)
            .map(|(_, p)| *p);
        let healthy = found_port.is_some();
        let port = found_port.or_else(|| session.port_overrides.get(&svc.name).copied());
        statuses.push(ServiceStatus {
            name: svc.name.clone(),
            kind: "docker",
            port,
            healthy,
            managed: true,
            pid: None,
            tmux_window: None,
            listener_pid: None,
            wrong_owner: false,
            // Docker publishes ports through the daemon, not a host process
            // tree, so process-tree discovery doesn't apply. `find_docker_services`
            // already reports the real published port.
            actual_port: None,
            conflicting_slot: None,
            listening_ports: vec![],
        });
    }

    let all_healthy = statuses.iter().all(|s| !s.managed || s.healthy);

    if args.json {
        let services_json: Vec<serde_json::Value> = statuses
            .iter()
            .map(|s| {
                serde_json::json!({
                    "name": s.name,
                    "type": s.kind,
                    "port": s.port,
                    "healthy": s.healthy,
                    "managed": s.managed,
                    "pid": s.pid,
                    "tmux_window": s.tmux_window,
                    "listener_pid": s.listener_pid,
                    "wrong_owner": s.wrong_owner,
                    "actual_port": s.actual_port,
                    "listening_ports": s.listening_ports,
                    "port_mismatch": s.actual_port.is_some(),
                    "conflicting_slot": s.conflicting_slot.as_ref().map(|(slot, owner)| {
                        serde_json::json!({ "slot": slot, "session": owner })
                    }),
                    "hint": mismatch_hint(&session.slug, s),
                })
            })
            .collect();
        let out = serde_json::json!({
            "slug": session.slug,
            "slot": session.slot,
            "all_healthy": all_healthy,
            "tmux_session": session.tmux_session,
            "services": services_json,
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else if !args.quiet {
        let mut meta: Vec<(&str, String)> = vec![
            ("Slug", session.slug.clone()),
            ("Slot", session.slot.to_string()),
            ("Worktree", session.worktree_path.clone()),
        ];
        if let Some(ref ts) = session.tmux_session {
            meta.push(("Tmux", ts.clone()));
        }
        let label_width = meta.iter().map(|(k, _)| k.len()).max().unwrap_or(0);
        for (label, value) in &meta {
            println!("  {:>width$}  {}", label, value, width = label_width);
        }
        println!();

        if statuses.is_empty() {
            println!("No services defined in .ecluse.toml.");
        } else {
            let port_str = |s: &ServiceStatus| -> String {
                s.port.map(|p| p.to_string()).unwrap_or_else(|| "-".into())
            };
            match session.process_manager {
                Some(process::ProcessManager::Tmux) => {
                    let rows: Vec<StatusRowTmux> = statuses
                        .iter()
                        .map(|s| StatusRowTmux {
                            service: s.name.clone(),
                            kind: s.kind.to_string(),
                            port: port_str(s),
                            actual: actual_str(s),
                            status: status_str(s),
                            window: s.tmux_window.clone().unwrap_or_else(|| "-".into()),
                        })
                        .collect();
                    println!("{}", Table::new(rows));
                }
                Some(process::ProcessManager::Nohup) => {
                    let rows: Vec<StatusRowNohup> = statuses
                        .iter()
                        .map(|s| StatusRowNohup {
                            service: s.name.clone(),
                            kind: s.kind.to_string(),
                            port: port_str(s),
                            actual: actual_str(s),
                            status: status_str(s),
                            pid: s.pid.map(|p| p.to_string()).unwrap_or_else(|| "-".into()),
                        })
                        .collect();
                    println!("{}", Table::new(rows));
                }
                _ => {
                    let rows: Vec<StatusRowNone> = statuses
                        .iter()
                        .map(|s| StatusRowNone {
                            service: s.name.clone(),
                            kind: s.kind.to_string(),
                            port: port_str(s),
                            actual: actual_str(s),
                            status: status_str(s),
                        })
                        .collect();
                    println!("{}", Table::new(rows));
                }
            }
            println!();

            let down_count = statuses.iter().filter(|s| s.managed && !s.healthy).count();
            if down_count > 0 {
                eprintln!(
                    "{} service{} down",
                    down_count,
                    if down_count == 1 { "" } else { "s" }
                );
            }

            // Explain every wrong-port service and name the safe remedy. This
            // is the whole point of discovery: an agent that reads "wrong
            // port, run down/up" doesn't invent a theory and reach for `kill`.
            let log = log::StepLogger::new(false);
            for s in &statuses {
                if let Some(hint) = mismatch_hint(&session.slug, s) {
                    log.warn(&hint);
                }
            }
        }
    }

    if !all_healthy {
        std::process::exit(1);
    }

    Ok(())
}
