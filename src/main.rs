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
        cli::Command::Status(args) => cmd_status(args),
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

fn resolve_slug_from_args(
    arg: Option<&str>,
    guard: &state::StateGuard,
    hint: &str,
) -> Result<String> {
    match arg {
        Some(s) => {
            validate_slug(s)?;
            Ok(s.to_string())
        }
        None => {
            let cwd = std::env::current_dir().context("could not determine current directory")?;
            guard
                .state
                .sessions
                .iter()
                .find(|s| cwd.starts_with(std::path::Path::new(&s.worktree_path)))
                .map(|s| s.slug.clone())
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "not inside an ecluse worktree; pass a slug or cd into a worktree\n  hint: {}",
                        hint
                    )
                })
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
        WorktreeRemovalChoice::Stop => Err(anyhow::anyhow!("aborted")),
        WorktreeRemovalChoice::KeepWorktree => Ok(true),
        WorktreeRemovalChoice::DeleteWorktree => Ok(false),
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

    // Resolve slug: from arg or auto-detect from cwd.
    if args.slug.is_none() {
        log.step("Looking for existing session...");
    }
    let slug = resolve_slug_from_args(args.slug.as_deref(), &guard, "ecluse up <slug>")?;

    // Resume path: session already exists — restart/skip services idempotently.
    if let Some(existing) = guard.state.find_session(&slug).cloned() {
        log.step("Looking for existing session...");
        log.detail(&format!(
            "found session '{}' (slot {}) — reusing worktree",
            slug, existing.slot
        ));
        return cmd_up_resume(existing, args, config, root, guard, log);
    }

    // New session path (unchanged).
    log.step("Allocating slot...");
    let allocator = slot::SlotAllocator::new(&config, &guard.state);
    let slot = allocator.allocate_next()?;
    log.detail(&format!("slot {slot}"));

    let branch = args
        .branch
        .clone()
        .unwrap_or_else(|| format!("{}/{}", config.prefix, slug));
    validate_branch(&branch)?;

    let handler = modes::get_handler(&config);

    let port_overrides: std::collections::HashMap<String, u16> =
        args.port_overrides.iter().cloned().collect();

    let service_filter: Option<std::collections::HashSet<String>> =
        parse_service_filter(&args.services, &config)?;

    let session = handler.bring_up(
        &slug,
        slot,
        &branch,
        &config,
        &root,
        args.watch,
        args.reuse_worktree,
        &port_overrides,
        service_filter.as_ref(),
        &std::collections::HashSet::new(),
        &std::collections::HashMap::new(),
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
fn cmd_up_resume(
    existing: state::Session,
    args: cli::UpArgs,
    config: config::Config,
    root: std::path::PathBuf,
    mut guard: state::StateGuard,
    log: log::StepLogger,
) -> Result<()> {
    let worktree = std::path::Path::new(&existing.worktree_path);
    let handler = modes::get_handler(&config);

    // Build explicit --skip set.
    let explicit_skip: std::collections::HashSet<String> = args
        .skip
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .cloned()
        .collect();

    // Validate --skip names.
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

    let mut skip_services: std::collections::HashSet<String> = explicit_skip.clone();

    if args.force {
        // Kill all non-skipped services.
        log.step("--force: killing services on allocated ports...");
        force_kill_session_services(&existing, &config, &explicit_skip, &log);
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

        let discovered = if !native_svcs.is_empty() {
            sync::find_processes_in_worktree(worktree)
        } else {
            vec![]
        };
        let native_matches = sync::match_services(&native_svcs, &discovered);
        let docker_matches = if !docker_svcs.is_empty() {
            sync::find_docker_services(&docker_svcs, &existing.slug)
        } else {
            vec![]
        };

        for svc in &native_svcs {
            if explicit_skip.contains(&svc.name) {
                log.detail(&format!("{}: skipped (--skip)", svc.name));
                continue;
            }
            let alive = native_matches
                .iter()
                .find(|m| m.service_name == svc.name)
                .map(|m| process::pid_alive(m.pid))
                .unwrap_or(false);
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

    if to_start == 0 && !args.force {
        log.step("All services already running — nothing to do.");
        if args.json {
            print_up_json(&existing, &root)?;
        } else {
            print_up_summary(&existing, &config, &log);
        }
        return Ok(());
    }

    let port_overrides: std::collections::HashMap<String, u16> =
        args.port_overrides.iter().cloned().collect();
    let service_filter = parse_service_filter(&args.services, &config)?;

    let updated_session = handler.bring_up(
        &existing.slug,
        existing.slot,
        &existing.branch,
        &config,
        &root,
        args.watch,
        true, // always reuse-worktree on resume
        &port_overrides,
        service_filter.as_ref(),
        &skip_services,
        &existing.port_overrides,
        &log,
    )?;

    if !args.quiet && !args.json {
        let started = to_start;
        println!();
        log.success(&format!(
            "{} service{} started, {} skipped",
            started,
            if started == 1 { "" } else { "s" },
            skipped_count
        ));
    }

    if args.json {
        print_up_json(&updated_session, &root)?;
    }

    // Replace session in state with refreshed version.
    guard.state.remove_session(&existing.slug);
    guard.state.add_session(updated_session);
    guard.commit()?;

    Ok(())
}

/// Kill all non-skipped services for a session.
/// Native: kill by PID files, then by port (lsof). Docker: docker stop by container name.
fn force_kill_session_services(
    session: &state::Session,
    config: &config::Config,
    skip: &std::collections::HashSet<String>,
    log: &log::StepLogger,
) {
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

    // Kill by port for residual native processes only.
    // Docker services are stopped via docker stop — never kill their host port
    // by PID, as the listening process may be the container runtime itself
    // (e.g. OrbStack) rather than the container.
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
        // lsof -ti TCP:<port> returns PIDs; kill -9 each
        let output = std::process::Command::new("lsof")
            .args(["-ti", &format!("TCP:{}", port), "-sTCP:LISTEN"])
            .output();
        if let Ok(out) = output {
            let stdout = String::from_utf8_lossy(&out.stdout);
            for pid_str in stdout.split_whitespace() {
                if let Ok(pid) = pid_str.trim().parse::<u32>() {
                    log.detail(&format!(
                        "killed process {} on port {} ({})",
                        pid, port, svc_name
                    ));
                    let _ = std::process::Command::new("kill")
                        .args(["-9", pid_str.trim()])
                        .status();
                }
            }
        }
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

    let slug = resolve_slug_from_args(args.slug.as_deref(), &guard, "ecluse down <slug>")?;

    log.step(&format!("Loading session '{slug}'..."));
    let session = guard
        .state
        .find_session(&slug)
        .ok_or_else(|| error::EcluseError::SessionNotFound(slug.clone()))?
        .clone();
    log.detail(&format!("slot {}, mode: {}", session.slot, session.mode));

    let keep_worktree = resolve_worktree_keep(
        std::path::Path::new(&session.worktree_path),
        args.keep_worktree,
        args.delete_worktree,
    )?;

    let handler = modes::get_handler(&config);
    handler.bring_down(
        &session,
        &config,
        &root,
        args.keep_volumes,
        keep_worktree,
        &log,
    )?;

    guard.state.remove_session(&slug);
    guard.commit()?;

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

        match handler.bring_down(&session, &config, &root, args.keep_volumes, keep_wt, &log) {
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
    #[tabled(rename = "TMUX")]
    tmux: String,
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
                tmux: s.tmux_session.clone().unwrap_or_default(),
                branch: s.branch.clone(),
                started: s.started_at[..16].replace('T', " "),
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
    }
    if !any_tmux {
        use tabled::settings::object::Columns;
        use tabled::settings::Disable;
        // TMUX is column index 4 (SLUG=0, MODE=1, SLOT=2, PORTS=3, TMUX=4)
        table.with(Disable::column(Columns::single(4)));
    }
    println!("{}", table);

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

    let slug = resolve_slug_from_args(args.slug.as_deref(), &guard, "ecluse shell <slug>")?;

    let session = guard
        .state
        .find_session(&slug)
        .ok_or_else(|| error::EcluseError::SessionNotFound(slug.clone()))?
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

    let slug = resolve_slug_from_args(args.slug.as_deref(), &guard, "ecluse env <slug>")?;
    let session = guard
        .state
        .find_session(&slug)
        .ok_or_else(|| error::EcluseError::SessionNotFound(slug.clone()))?
        .clone();

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
                let cwd =
                    std::env::current_dir().context("could not determine current directory")?;
                if cwd.starts_with(&root) || cwd.to_str().is_some_and(|c| c.contains(s.as_str())) {
                    cwd
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
        sync::find_docker_services(&docker_svcs, &slug)
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
    let env_map = env::build_env(
        slot,
        &slug,
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
        slug: slug.clone(),
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

// ── status ────────────────────────────────────────────────────────────────────

struct ServiceStatus {
    name: String,
    kind: &'static str,
    port: Option<u16>,
    healthy: bool,
    pid: Option<u32>,
}

#[derive(Tabled)]
struct StatusRow {
    #[tabled(rename = "SERVICE")]
    service: String,
    #[tabled(rename = "TYPE")]
    kind: String,
    #[tabled(rename = "PORT")]
    port: String,
    #[tabled(rename = "STATUS")]
    status: String,
    #[tabled(rename = "PID")]
    pid: String,
}

fn cmd_status(args: cli::StatusArgs) -> Result<()> {
    let (config, root) = config::Config::find_and_load()?;
    let guard = state::StateGuard::acquire(&root)?;

    let slug = resolve_slug_from_args(args.slug.as_deref(), &guard, "ecluse status <slug>")?;
    let session = guard
        .state
        .find_session(&slug)
        .ok_or_else(|| error::EcluseError::SessionNotFound(slug.clone()))?
        .clone();

    let worktree = std::path::Path::new(&session.worktree_path);

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

    // Discover processes once for all native services.
    let discovered = if !native_svcs.is_empty() {
        sync::find_processes_in_worktree(worktree)
    } else {
        vec![]
    };

    let native_matches = sync::match_services(&native_svcs, &discovered);

    // Docker: find running containers.
    let docker_matches = if !docker_svcs.is_empty() {
        sync::find_docker_services(&docker_svcs, &session.slug)
    } else {
        vec![]
    };

    let mut statuses: Vec<ServiceStatus> = Vec::new();

    for svc in &native_svcs {
        let matched = native_matches.iter().find(|m| m.service_name == svc.name);
        let (healthy, pid, port) = match matched {
            Some(m) => {
                let alive = process::pid_alive(m.pid);
                let p = m
                    .port
                    .or_else(|| session.port_overrides.get(&svc.name).copied());
                (alive, Some(m.pid), p)
            }
            None => {
                // Fall back to PID file if sync was used.
                let pid_file = root
                    .join(".ecluse")
                    .join("pids")
                    .join(&session.slug)
                    .join(format!("{}.pid", svc.name));
                if pid_file.exists() {
                    if let Ok(content) = std::fs::read_to_string(&pid_file) {
                        if let Ok(pid) = content.trim().parse::<u32>() {
                            let alive = process::pid_alive(pid);
                            let p = session.port_overrides.get(&svc.name).copied();
                            (alive, Some(pid), p)
                        } else {
                            (false, None, session.port_overrides.get(&svc.name).copied())
                        }
                    } else {
                        (false, None, session.port_overrides.get(&svc.name).copied())
                    }
                } else {
                    (false, None, session.port_overrides.get(&svc.name).copied())
                }
            }
        };
        statuses.push(ServiceStatus {
            name: svc.name.clone(),
            kind: "native",
            port,
            healthy,
            pid,
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
            pid: None,
        });
    }

    let all_healthy = statuses.iter().all(|s| s.healthy);

    if args.json {
        let services_json: Vec<serde_json::Value> = statuses
            .iter()
            .map(|s| {
                serde_json::json!({
                    "name": s.name,
                    "type": s.kind,
                    "port": s.port,
                    "healthy": s.healthy,
                    "pid": s.pid,
                })
            })
            .collect();
        let out = serde_json::json!({
            "slug": session.slug,
            "slot": session.slot,
            "all_healthy": all_healthy,
            "services": services_json,
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else if !args.quiet {
        println!(
            "Session: {}  slot={}  worktree={}",
            session.slug, session.slot, session.worktree_path
        );
        println!();

        if statuses.is_empty() {
            println!("No services defined in .ecluse.toml.");
        } else {
            let rows: Vec<StatusRow> = statuses
                .iter()
                .map(|s| StatusRow {
                    service: s.name.clone(),
                    kind: s.kind.to_string(),
                    port: s.port.map(|p| p.to_string()).unwrap_or_else(|| "-".into()),
                    status: if s.healthy {
                        "\u{2713} up".into()
                    } else {
                        "\u{2717} down".into()
                    },
                    pid: s.pid.map(|p| p.to_string()).unwrap_or_else(|| "-".into()),
                })
                .collect();
            println!("{}", Table::new(rows));
            println!();

            let down_count = statuses.iter().filter(|s| !s.healthy).count();
            if down_count > 0 {
                eprintln!(
                    "{} service{} down",
                    down_count,
                    if down_count == 1 { "" } else { "s" }
                );
            }
        }
    }

    if !all_healthy {
        std::process::exit(1);
    }

    Ok(())
}
