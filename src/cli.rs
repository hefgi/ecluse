use clap::{Args, Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "ecluse",
    version,
    about = "Ephemeral local environments for coding agents — any stack.",
    long_about = None
)]
pub struct Cli {
    #[arg(long, global = true, help = "Enable verbose debug output")]
    pub verbose: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Initialize ecluse in the current git repository
    Init(InitArgs),
    /// Create a new isolated worktree session
    Up(UpArgs),
    /// Tear down a session and release its resources
    Down(DownArgs),
    /// List active sessions
    Ls(LsArgs),
    /// Open a shell inside a session's worktree with its env loaded
    Shell(ShellArgs),
    /// Print a session's environment variables (worktree path + all env vars)
    Env(EnvArgs),
    /// Validate .ecluse.toml — check port ranges, service gaps, and search range safety
    Validate(ValidateArgs),
    /// Tear down all active sessions
    Shutdown(ShutdownArgs),
    /// Register a manually-started environment by discovering its running processes
    Sync(SyncArgs),
    /// Hard-reset to clean state: kill all sessions, orphaned tmux sessions, and orphaned containers
    Flush(FlushArgs),
    /// Show health status of services for a session
    Status(StatusArgs),
}

#[derive(Args)]
pub struct InitArgs {
    /// Isolation mode (container, host, hybrid)
    #[arg(long, short)]
    pub mode: Option<String>,

    /// Show full signal score breakdown
    #[arg(long)]
    pub explain: bool,

    /// Accept recommended settings without prompting
    #[arg(long, short)]
    pub yes: bool,

    /// Maximum number of concurrent slots
    #[arg(long, default_value = "8")]
    pub max_slots: u8,

    /// Prefix for compose project names and branches
    #[arg(long, default_value = "ecluse")]
    pub prefix: String,

    /// Suppress step output
    #[arg(long)]
    pub quiet: bool,
}

#[derive(Args)]
pub struct UpArgs {
    /// Branch name or slug for this session — auto-detected from current git branch when omitted.
    /// Branch names with '/' are accepted: 'feat/sc-123-foo' → slug 'feat-sc-123-foo'.
    pub slug: Option<String>,

    /// Enable compose watch mode
    #[arg(long)]
    pub watch: bool,

    /// Output session info as JSON (useful for agents)
    #[arg(long)]
    pub json: bool,

    /// Reuse an existing worktree instead of creating a new one
    #[arg(long)]
    pub reuse_worktree: bool,

    /// Override a service port: --port <name>=<value> (repeatable)
    #[arg(long = "port", value_name = "NAME=PORT", value_parser = parse_port_override)]
    pub port_overrides: Vec<(String, u16)>,

    /// Only bring up these services (comma-separated). Omit to start all.
    #[arg(long, value_delimiter = ',', value_name = "NAME")]
    pub services: Option<Vec<String>>,

    /// Skip these services entirely (comma-separated); combinable with --force
    #[arg(long, value_delimiter = ',', value_name = "NAME")]
    pub skip: Option<Vec<String>>,

    /// Kill all running services before starting — full restart
    #[arg(long)]
    pub force: bool,

    /// Skip symlinking inherited env files (inherit_env in .ecluse.toml) — for CI/agents
    #[arg(long)]
    pub no_inherit_env: bool,

    /// Suppress step output (implied by --json)
    #[arg(long)]
    pub quiet: bool,
}

#[derive(Args)]
pub struct DownArgs {
    /// Session slug to tear down (auto-detected from cwd when omitted)
    pub slug: Option<String>,

    /// Keep named volumes (do not pass -v to docker compose down)
    #[arg(long)]
    pub keep_volumes: bool,

    /// Keep the git branch after tearing down the worktree (no-op in v0; branches are never deleted)
    #[arg(long)]
    pub keep_branch: bool,

    /// Tear down services but keep the git worktree on disk
    #[arg(long)]
    pub keep_worktree: bool,

    /// Skip the worktree deletion prompt and delete the worktree (for CI/agents)
    #[arg(long)]
    pub delete_worktree: bool,

    /// Suppress step output
    #[arg(long)]
    pub quiet: bool,
}

fn parse_port_override(s: &str) -> Result<(String, u16), String> {
    let (name, port_str) = s
        .split_once('=')
        .ok_or_else(|| format!("expected NAME=PORT, got '{}'", s))?;
    let port: u16 = port_str
        .parse()
        .map_err(|_| format!("invalid port '{}': must be a number 1-65535", port_str))?;
    if port == 0 {
        return Err("port must be >= 1".into());
    }
    Ok((name.to_string(), port))
}

#[derive(Args)]
pub struct LsArgs {
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct ShellArgs {
    /// Session slug to enter (auto-detected from cwd when omitted)
    pub slug: Option<String>,
}

#[derive(Args)]
pub struct EnvArgs {
    /// Session slug — omit when already inside a worktree
    pub slug: Option<String>,
}

#[derive(Args)]
pub struct ShutdownArgs {
    /// Keep named volumes (do not pass -v to docker compose down)
    #[arg(long)]
    pub keep_volumes: bool,

    /// Tear down services but keep git worktrees on disk
    #[arg(long)]
    pub keep_worktrees: bool,

    /// Skip the per-worktree deletion prompt and delete all worktrees (for CI/agents)
    #[arg(long)]
    pub delete_worktrees: bool,

    /// Suppress step output
    #[arg(long)]
    pub quiet: bool,
}

#[derive(Args)]
pub struct SyncArgs {
    /// Session slug — used to locate the worktree and name the session (auto-detected from cwd when omitted)
    pub slug: Option<String>,

    /// Output session info as JSON (useful for agents)
    #[arg(long)]
    pub json: bool,

    /// Suppress step output
    #[arg(long)]
    pub quiet: bool,
}

#[derive(Args)]
pub struct FlushArgs {
    /// Skip confirmation prompt
    #[arg(long, short)]
    pub yes: bool,

    /// Suppress step output
    #[arg(long)]
    pub quiet: bool,
}

#[derive(Args)]
pub struct StatusArgs {
    /// Session slug (auto-detected from cwd if omitted)
    pub slug: Option<String>,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
    /// Suppress table output (only exit code matters)
    #[arg(long)]
    pub quiet: bool,
}

#[derive(Args)]
pub struct ValidateArgs {
    /// Also print the port allocation table for all slots
    #[arg(long)]
    pub ports: bool,

    /// Suppress step output
    #[arg(long)]
    pub quiet: bool,
}
