use thiserror::Error;

#[derive(Error, Debug)]
#[allow(dead_code)]
pub enum EcluseError {
    #[error("invalid slug '{0}': must match ^[a-z0-9][a-z0-9-]{{0,30}}[a-z0-9]$")]
    SlugInvalid(String),

    #[error("all {0} slots are in use; run `ecluse down <slug>` to free one")]
    SlotsExhausted(u8),

    #[error("session '{0}' already exists; use a different slug")]
    SessionExists(String),

    #[error("session '{0}' not found")]
    SessionNotFound(String),

    #[error("timed out waiting for state lock after 10s; another ecluse process may be running")]
    LockTimeout,

    #[error("state file is corrupt: {0}")]
    StateCorrupt(String),

    #[error("config file not found; run `ecluse init` first")]
    ConfigMissing,

    #[error("config is invalid: {0}")]
    ConfigInvalid(String),

    #[error("docker-compose.yml not found at {0}")]
    ComposeFileNotFound(String),

    #[error("failed to parse compose file: {0}")]
    ComposeParseFailed(String),

    #[error("docker command failed: {stderr}")]
    DockerFailed { stderr: String },

    #[error("port {port} is already in use by PID {pid}; stop that process first")]
    PortInUse { port: u16, pid: u32 },

    #[error("compose file has no service labeled ecluse.role=app; all services treated as data")]
    AppLabelMissing,

    #[error("hook failed with exit code {code}: {cmd}")]
    HookFailed { cmd: String, code: i32 },

    #[error("not inside a git repository")]
    NotAGitRepo,

    #[error("unknown mode '{0}'; valid: container, host, hybrid")]
    ModeInvalid(String),
}
