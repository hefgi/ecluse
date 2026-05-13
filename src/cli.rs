use clap::{Args, Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "ecluse",
    version,
    about = "Per-worktree isolation. Pick what you need isolated.",
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
    /// Show embedded skill documentation
    Skills(SkillsArgs),
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

    /// Port stride per slot
    #[arg(long, default_value = "100")]
    pub stride: u16,

    /// Prefix for compose project names and branches
    #[arg(long, default_value = "ecluse")]
    pub prefix: String,
}

#[derive(Args)]
pub struct UpArgs {
    /// Slug for this session (lowercase letters, numbers, hyphens)
    pub slug: String,

    /// Branch to use (creates ecluse/<slug> from HEAD if not specified)
    #[arg(long)]
    pub branch: Option<String>,

    /// Enable compose watch mode
    #[arg(long)]
    pub watch: bool,
}

#[derive(Args)]
pub struct DownArgs {
    /// Session slug to tear down
    pub slug: String,

    /// Keep named volumes (do not pass -v to docker compose down)
    #[arg(long)]
    pub keep_volumes: bool,

    /// Keep the provisioned database (do not DROP DATABASE)
    #[arg(long)]
    pub keep_database: bool,

    /// Keep the git branch after tearing down the worktree (no-op in v0; branches are never deleted)
    #[arg(long)]
    pub keep_branch: bool,
}

#[derive(Args)]
pub struct LsArgs {
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct SkillsArgs {
    #[command(subcommand)]
    pub command: Option<SkillsCommand>,
}

#[derive(Subcommand)]
pub enum SkillsCommand {
    /// List all available skills
    List,
    /// Print a skill to stdout
    Show {
        /// Skill name
        name: String,
    },
    /// Write all skills to .ecluse/skills/ in the current repo
    Install,
}
