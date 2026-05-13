mod cli;
mod compose;
mod config;
mod detect;
mod docker;
mod env;
mod error;
mod hooks;
mod modes;
mod slot;
mod state;
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
    }
}

// ── init ─────────────────────────────────────────────────────────────────────

fn cmd_init(args: cli::InitArgs) -> Result<()> {
    let cwd = std::env::current_dir().context("could not determine current directory")?;
    worktree::WorktreeManager::verify_git_repo(&cwd)?;

    let mode: config::Mode = if let Some(m) = &args.mode {
        m.parse()?
    } else {
        // Run detection
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
                Some(m) => println!("Recommended mode: {} ({})", m, result.confidence),
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

    let config_path = cwd.join(".ecluse.toml");
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
    cfg.save(&cwd)?;

    // Create .ecluse/ and suggest .gitignore
    let ecluse_dir = cwd.join(".ecluse");
    std::fs::create_dir_all(&ecluse_dir)?;

    let gitignore_path = cwd.join(".gitignore");
    let should_add = if gitignore_path.exists() {
        let content = std::fs::read_to_string(&gitignore_path)?;
        !content.lines().any(|l| l.trim() == ".ecluse/")
    } else {
        true
    };

    if should_add {
        if args.yes {
            append_gitignore(&gitignore_path)?;
        } else {
            print!("Add .ecluse/ to .gitignore? [Y/n] ");
            io::stdout().flush()?;
            let mut input = String::new();
            io::stdin().read_line(&mut input)?;
            if input.trim().is_empty() || input.trim().eq_ignore_ascii_case("y") {
                append_gitignore(&gitignore_path)?;
            }
        }
    }

    println!();
    println!("Initialized ecluse in {} (mode: {})", cwd.display(), mode);
    println!("Config written to .ecluse.toml");
    println!();
    println!("Next steps:");
    println!("  ecluse up <slug>          create a new isolated session");

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

fn cmd_up(args: cli::UpArgs) -> Result<()> {
    validate_slug(&args.slug)?;
    let (config, root) = config::Config::find_and_load()?;

    // Validate config before acquiring the state lock
    let warnings = validate::validate_config(&config)?;
    for w in &warnings {
        eprintln!("warning: {}", w);
    }

    let mut guard = state::StateGuard::acquire(&root)?;

    if guard.state.find_session(&args.slug).is_some() {
        return Err(error::EcluseError::SessionExists(args.slug.clone()).into());
    }

    let allocator = slot::SlotAllocator::new(&config, &guard.state);
    let slot = allocator.allocate_next()?;

    let branch = args
        .branch
        .clone()
        .unwrap_or_else(|| format!("{}/{}", config.prefix, args.slug));

    let handler = modes::get_handler(&config);

    let session = handler.bring_up(&args.slug, slot, &branch, &config, &root, args.watch)?;

    if args.json {
        print_up_json(&session, &root)?;
    } else {
        print_up_summary(&session, &config);
    }

    guard.state.add_session(session);
    guard.commit()?;

    Ok(())
}

fn print_up_summary(session: &state::Session, _config: &config::Config) {
    println!();
    println!("Session:    {} (slot {})", session.slug, session.slot);
    println!("Worktree:   {}", session.worktree_path);
    println!("Mode:       {}", session.mode);
    println!("Branch:     {}", session.branch);

    match &session.mode {
        config::Mode::Container => {
            if let Some(port) = session.app_port {
                println!("App URL:    http://localhost:{}", port);
            }
            println!(
                "Project:    {}",
                session.compose_project.as_deref().unwrap_or("-")
            );
        }
        config::Mode::Host | config::Mode::Hybrid => {
            if let Some(port) = session.app_port {
                println!("App port:   {}", port);
            }
            println!();
            println!("Next step:");
            println!("  ecluse shell {}", session.slug);
            println!("  <your dev command>  # e.g. npm run dev, bin/dev, bin/rails server");
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
    let (config, root) = config::Config::find_and_load()?;
    let mut guard = state::StateGuard::acquire(&root)?;

    let session = guard
        .state
        .find_session(&args.slug)
        .ok_or_else(|| error::EcluseError::SessionNotFound(args.slug.clone()))?
        .clone();

    let handler = modes::get_handler(&config);
    handler.bring_down(&session, &config, &root, args.keep_volumes)?;

    guard.state.remove_session(&args.slug);
    guard.commit()?;

    if args.keep_branch {
        tracing::debug!(
            "--keep-branch is a no-op in v0; branch '{}' is kept",
            session.branch
        );
    }

    println!("Session '{}' torn down.", args.slug);
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
    #[tabled(rename = "PORT")]
    port: String,
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
        .map(|s| SessionRow {
            slug: s.slug.clone(),
            mode: s.mode.to_string(),
            slot: s.slot,
            port: s
                .app_port
                .map(|p| p.to_string())
                .unwrap_or_else(|| "-".into()),
            branch: s.branch.clone(),
            started: s.started_at[..16].replace('T', " "),
        })
        .collect();

    println!("{}", Table::new(rows));
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

    // Parse .env.ecluse into key=value pairs
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
    let (config, root) = config::Config::find_and_load()?;

    let warnings = validate::validate_config(&config)?;

    for w in &warnings {
        eprintln!("warning: {}", w);
    }

    println!("Config at {} is valid.", root.join(".ecluse.toml").display());
    println!(
        "  max_slots:         {}",
        config.max_slots
    );
    println!(
        "  strict_port:       {}",
        config.strict_port
    );
    println!(
        "  port_search_range: {}",
        config.port_search_range
    );

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
        let header_parts: Vec<String> = config.services.iter().map(|s| format!("{:>20}", s.name)).collect();
        println!("  {:>6}  {}", "slot", header_parts.join("  "));
        for slot in 1..=config.max_slots {
            let port_parts: Vec<String> = config.services.iter().map(|s| format!("{:>20}", s.port(slot))).collect();
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
    Ok(())
}
