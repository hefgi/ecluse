mod cli;
mod compose;
mod config;
mod detect;
mod docker;
mod env;
mod error;
mod hooks;
mod log;
mod modes;
mod process;
mod slot;
mod state;
mod sync;
mod validate;
mod worktree;

use anyhow::{Context, Result};
use clap::Parser;
use std::io::{self, Write};
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
    }
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
        services: vec![],
        hooks: config::HookConfig::default(),
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
        // 33 chars — one over max
        let slug = "a".repeat(33);
        assert!(validate_slug(&slug).is_err());
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
}

fn validate_slug(slug: &str) -> Result<()> {
    let re = regex_lite::Regex::new(r"^[a-z0-9][a-z0-9\-]{0,30}[a-z0-9]$").unwrap();
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

    validate_slug(&args.slug)?;

    log.step("Loading config...");
    let (config, root) = config::Config::find_and_load()?;
    log.detail(&format!("mode: {}", config.mode));

    let warnings = validate::validate_config(&config)?;
    for w in &warnings {
        log.warn(w);
    }

    let global = process::load_global_config()?;
    validate::validate_process_manager(&global.process_manager)?;

    let mut guard = state::StateGuard::acquire(&root)?;

    if guard.state.find_session(&args.slug).is_some() {
        return Err(error::EcluseError::SessionExists(args.slug.clone()).into());
    }

    log.step("Allocating slot...");
    let allocator = slot::SlotAllocator::new(&config, &guard.state);
    let slot = allocator.allocate_next()?;
    log.detail(&format!("slot {slot}"));

    let branch = args
        .branch
        .clone()
        .unwrap_or_else(|| format!("{}/{}", config.prefix, args.slug));
    validate_branch(&branch)?;

    let handler = modes::get_handler(&config);

    let port_overrides: std::collections::HashMap<String, u16> =
        args.port_overrides.iter().cloned().collect();

    let service_filter: Option<std::collections::HashSet<String>> = match args.services.as_deref() {
        None | Some([]) => None,
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
            Some(set)
        }
    };

    let session = handler.bring_up(
        &args.slug,
        slot,
        &branch,
        &config,
        &root,
        args.watch,
        args.reuse_worktree,
        &port_overrides,
        service_filter.as_ref(),
        &log,
    )?;

    if args.json {
        print_up_json(&session, &root)?;
    } else {
        print_up_summary(&session, &config, &log);
    }

    guard.state.add_session(session);
    guard.commit()?;

    Ok(())
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
    if env_file.exists() {
        for line in std::fs::read_to_string(&env_file)?.lines() {
            if let Some((k, v)) = line.split_once('=') {
                env_vars.insert(k.to_string(), serde_json::Value::String(v.to_string()));
            }
        }
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

    let mut guard = state::StateGuard::acquire(&root)?;

    log.step(&format!("Loading session '{}'...", args.slug));
    let session = guard
        .state
        .find_session(&args.slug)
        .ok_or_else(|| error::EcluseError::SessionNotFound(args.slug.clone()))?
        .clone();
    log.detail(&format!("slot {}, mode: {}", session.slot, session.mode));

    let handler = modes::get_handler(&config);
    handler.bring_down(
        &session,
        &config,
        &root,
        args.keep_volumes,
        args.keep_worktree,
        &log,
    )?;

    guard.state.remove_session(&args.slug);
    guard.commit()?;

    if args.keep_branch {
        eprintln!(
            "warning: --keep-branch has no effect; branches are never deleted by ecluse down (branch '{}' is kept)",
            session.branch
        );
    }

    println!();
    if args.keep_worktree {
        log.success(&format!(
            "Session '{}' torn down (worktree kept at {}).",
            args.slug, session.worktree_path
        ));
    } else {
        log.success(&format!("Session '{}' torn down.", args.slug));
    }
    println!();

    Ok(())
}

// ── shutdown ──────────────────────────────────────────────────────────────────

fn cmd_shutdown(args: cli::ShutdownArgs) -> Result<()> {
    let log = log::StepLogger::new(args.quiet);

    log.step("Loading config...");
    let (config, root) = config::Config::find_and_load()?;

    let mut guard = state::StateGuard::acquire(&root)?;

    if guard.state.sessions.is_empty() {
        println!("no active sessions");
        return Ok(());
    }

    let sessions: Vec<state::Session> = guard.state.sessions.clone();
    let total = sessions.len();
    let handler = modes::get_handler(&config);
    let mut failed: Vec<String> = Vec::new();

    for session in sessions {
        log.step(&format!("Tearing down '{}'...", session.slug));
        log.detail(&format!("slot {}, mode: {}", session.slot, session.mode));

        match handler.bring_down(
            &session,
            &config,
            &root,
            args.keep_volumes,
            args.keep_worktrees,
            &log,
        ) {
            Ok(()) => {
                guard.state.remove_session(&session.slug);
                guard.commit()?;
            }
            Err(e) => {
                log.warn(&format!("'{}' failed: {}", session.slug, e));
                failed.push(session.slug.clone());
            }
        }
    }

    println!();
    let torn_down = total - failed.len();
    if failed.is_empty() {
        log.success(&format!(
            "Shutdown complete — {} session{} torn down.",
            torn_down,
            if torn_down == 1 { "" } else { "s" }
        ));
    } else {
        log.warn(&format!(
            "{}/{} session{} torn down; {} failed: {}",
            torn_down,
            total,
            if total == 1 { "" } else { "s" },
            failed.len(),
            failed.join(", ")
        ));
    }
    println!();

    if !failed.is_empty() {
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
    #[tabled(rename = "BRANCH")]
    branch: String,
    #[tabled(rename = "STARTED")]
    started: String,
}

fn cmd_ls(args: cli::LsArgs) -> Result<()> {
    let (_, root) = config::Config::find_and_load()?;
    let guard = state::StateGuard::acquire(&root)?;

    if guard.state.sessions.is_empty() {
        println!("no active sessions");
        return Ok(());
    }

    if args.json {
        let json = serde_json::to_string_pretty(&guard.state.sessions)?;
        println!("{}", json);
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
            SessionRow {
                slug: s.slug.clone(),
                mode: s.mode.to_string(),
                slot: s.slot,
                ports,
                branch: s.branch.clone(),
                started: s.started_at[..16].replace('T', " "),
            }
        })
        .collect();

    println!("{}", Table::new(rows));

    let log = log::StepLogger::new(false);
    for s in &guard.state.sessions {
        for w in process::check_processes_alive(&s.process_manager, &s.spawn_result(), &s.slug) {
            log.warn(&format!("[{}] {}", s.slug, w));
        }
    }

    Ok(())
}

// ── shell ─────────────────────────────────────────────────────────────────────

fn cmd_shell(args: cli::ShellArgs) -> Result<()> {
    let (_, root) = config::Config::find_and_load()?;
    let guard = state::StateGuard::acquire(&root)?;

    let session = guard
        .state
        .find_session(&args.slug)
        .ok_or_else(|| error::EcluseError::SessionNotFound(args.slug.clone()))?
        .clone();

    let worktree = std::path::Path::new(&session.worktree_path);
    let env_file = worktree.join(".env.ecluse");

    let env_vars: Vec<(String, String)> = if env_file.exists() {
        std::fs::read_to_string(&env_file)
            .context("failed to read .env.ecluse")?
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    return None;
                }
                let (k, v) = line.split_once('=')?;
                Some((k.to_string(), v.to_string()))
            })
            .collect()
    } else {
        vec![]
    };

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
                svc.port(1),
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
                .map(|s| format!("{:>20}", s.port(slot)))
                .collect();
            println!("  {:>6}  {}", slot, port_parts.join("  "));
        }
    }

    Ok(())
}

// ── env ───────────────────────────────────────────────────────────────────────

fn cmd_env(args: cli::EnvArgs) -> Result<()> {
    let (_, root) = config::Config::find_and_load()?;
    let guard = state::StateGuard::acquire(&root)?;

    let session = match args.slug {
        Some(ref slug) => guard
            .state
            .find_session(slug)
            .ok_or_else(|| error::EcluseError::SessionNotFound(slug.clone()))?
            .clone(),
        None => {
            let cwd = std::env::current_dir().context("could not determine current directory")?;
            guard
                .state
                .sessions
                .iter()
                .find(|s| {
                    let wt = std::path::Path::new(&s.worktree_path);
                    cwd.starts_with(wt)
                })
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "not inside an ecluse worktree; run from a worktree or pass a slug"
                    )
                })?
                .clone()
        }
    };

    let env_file = std::path::Path::new(&session.worktree_path).join(".env.ecluse");

    let mut env_vars: serde_json::Map<String, serde_json::Value> = serde_json::Map::new();
    if env_file.exists() {
        for line in std::fs::read_to_string(&env_file)?.lines() {
            if let Some((k, v)) = line.split_once('=') {
                env_vars.insert(k.to_string(), serde_json::Value::String(v.to_string()));
            }
        }
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

    validate_slug(&args.slug)?;

    log.step("Loading config...");
    let (config, root) = config::Config::find_and_load()?;
    log.detail(&format!("mode: {}", config.mode));

    // Determine worktree path: prefer the standard ecluse location, fall back to cwd.
    let wt_manager = worktree::WorktreeManager::new(root.clone());
    let canonical_path = wt_manager.worktree_path(&config, &args.slug);

    let worktree_path = if canonical_path.exists() {
        canonical_path
    } else {
        let cwd = std::env::current_dir().context("could not determine current directory")?;
        // Accept cwd if it looks like it's inside this repo's worktree area.
        if cwd.starts_with(&root) || cwd.to_str().is_some_and(|s| s.contains(&args.slug)) {
            cwd
        } else {
            return Err(error::EcluseError::WorktreeNotFound {
                slug: args.slug.clone(),
            }
            .into());
        }
    };

    log.detail(&format!("worktree: {}", worktree_path.display()));

    // Acquire state lock.
    let mut guard = state::StateGuard::acquire(&root)?;
    let existing = guard.state.find_session(&args.slug).cloned();
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
        sync::find_docker_services(&docker_svcs, &args.slug)
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
        let pid_path = sync::write_pid_file(&ecluse_dir, &args.slug, &m.service_name, m.pid)?;
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
    let env_map = env::build_env(
        slot,
        &args.slug,
        &config.mode.to_string(),
        &native_ports,
        &docker_matches,
        &native_svcs,
    );
    env::write_env_file(&worktree_path, &env_map)?;

    let app_port = native_matches
        .first()
        .and_then(|m| m.port)
        .or_else(|| docker_matches.first().map(|(_, p)| *p));

    let session = state::Session {
        slug: args.slug.clone(),
        mode: config.mode.clone(),
        slot,
        branch,
        worktree_path: worktree_path.display().to_string(),
        app_port,
        port_overrides,
        process_manager: Some(process::ProcessManager::Nohup),
        pid_files,
        log_dir: None,
        compose_project: None,
        overlay_file: None,
        overlay_files: vec![],
        started_at: chrono::Utc::now().to_rfc3339(),
        tmux_session: None,
        services_subset: None,
    };

    if update_mode {
        guard.state.remove_session(&args.slug);
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
            let handler = modes::get_handler(&config);
            for session in sessions {
                log.detail(&format!("  down '{}'", session.slug));
                if let Err(e) = handler.bring_down(&session, &config, &root, false, false, &log) {
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
    for subdir in &["pids", "logs", "overlays"] {
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
