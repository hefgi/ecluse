mod cli;
mod compose;
mod config;
mod detect;
mod docker;
mod env;
mod error;
mod modes;
mod postgres;
mod slot;
mod state;
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
        base_port: args.base_port,
        stride: args.stride,
        prefix: args.prefix.clone(),
        worktree_dir: ".ecluse/worktrees".into(),
        app_label: "ecluse.role".into(),
        app_label_value: "app".into(),
        database: config::DatabaseConfig::default(),
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

    let mut guard = state::StateGuard::acquire(&root)?;

    if guard.state.find_session(&args.slug).is_some() {
        return Err(error::EcluseError::SessionExists(args.slug.clone()).into());
    }

    let allocator = slot::SlotAllocator::new(&config, &guard.state);
    let slot = allocator.allocate_next()?;
    let offset = allocator.offset(slot);

    let branch = args
        .branch
        .clone()
        .unwrap_or_else(|| format!("{}/{}", config.prefix, args.slug));

    let handler = modes::get_handler(&config);

    let session = handler.bring_up(
        &args.slug, slot, offset, &branch, &config, &root, args.watch,
    )?;

    print_up_summary(&session, &config);

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
            if let Some(db) = &session.database_name {
                println!("Database:   {}", db);
            }
            println!();
            println!("Next step:");
            println!("  cd {} && source .env.ecluse", session.worktree_path);
            println!("  <your dev command>  # e.g. npm run dev, bin/dev, bin/rails server");
        }
    }
    println!();
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
    handler.bring_down(
        &session,
        &config,
        &root,
        args.keep_volumes,
        args.keep_database,
    )?;

    guard.state.remove_session(&args.slug);
    guard.commit()?;

    if args.keep_branch {
        // v0: branches are never deleted anyway; flag noted for future use
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
    #[tabled(rename = "DATABASE")]
    database: String,
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
            database: s.database_name.clone().unwrap_or_else(|| "-".into()),
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

    println!("Entering ecluse session '{}' (slot {}).", session.slug, session.slot);
    println!("Type 'exit' to leave.\n");

    let status = std::process::Command::new(&shell)
        .current_dir(worktree)
        .envs(env_vars)
        .status()
        .with_context(|| format!("failed to launch shell: {}", shell))?;

    std::process::exit(status.code().unwrap_or(0));
}
