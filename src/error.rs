#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_invalid_message_contains_slug() {
        let e = EcluseError::SlugInvalid("BAD_SLUG!".into());
        assert!(e.to_string().contains("BAD_SLUG!"));
    }

    #[test]
    fn slots_exhausted_message_contains_count() {
        let e = EcluseError::SlotsExhausted(8);
        assert!(e.to_string().contains("8"));
    }

    #[test]
    fn session_exists_message_contains_slug() {
        let e = EcluseError::SessionExists("my-feat".into());
        assert!(e.to_string().contains("my-feat"));
    }

    #[test]
    fn session_not_found_message_contains_slug() {
        let e = EcluseError::SessionNotFound("ghost".into());
        assert!(e.to_string().contains("ghost"));
    }

    #[test]
    fn lock_timeout_message() {
        let e = EcluseError::LockTimeout;
        assert!(e.to_string().contains("10s"));
    }

    #[test]
    fn state_corrupt_message_contains_detail() {
        let e = EcluseError::StateCorrupt("unexpected EOF".into());
        assert!(e.to_string().contains("unexpected EOF"));
    }

    #[test]
    fn config_missing_message_suggests_init() {
        let e = EcluseError::ConfigMissing;
        assert!(e.to_string().contains("ecluse init"));
    }

    #[test]
    fn config_invalid_message_contains_detail() {
        let e = EcluseError::ConfigInvalid("bad field".into());
        assert!(e.to_string().contains("bad field"));
    }

    #[test]
    fn compose_file_not_found_message_contains_path() {
        let e = EcluseError::ComposeFileNotFound("/some/path".into());
        assert!(e.to_string().contains("/some/path"));
    }

    #[test]
    fn compose_parse_failed_message_contains_path() {
        let e = EcluseError::ComposeParseFailed("/other/path".into());
        assert!(e.to_string().contains("/other/path"));
    }

    #[test]
    fn docker_failed_message_contains_stderr() {
        let e = EcluseError::DockerFailed {
            stderr: "connection refused".into(),
        };
        assert!(e.to_string().contains("connection refused"));
    }

    #[test]
    fn port_in_use_message_contains_port_and_pid() {
        let e = EcluseError::PortInUse {
            port: 3100,
            pid: 42,
        };
        let msg = e.to_string();
        assert!(msg.contains("3100"));
        assert!(msg.contains("42"));
    }

    #[test]
    fn port_exhausted_message_contains_service_and_nominal() {
        let e = EcluseError::PortExhausted {
            service: "api".into(),
            nominal: 3001,
            range: 10,
        };
        let msg = e.to_string();
        assert!(msg.contains("api"));
        assert!(msg.contains("3001"));
        assert!(msg.contains("10"));
    }

    #[test]
    fn app_label_missing_message() {
        let e = EcluseError::AppLabelMissing;
        assert!(e.to_string().contains("ecluse.role=app"));
    }

    #[test]
    fn hook_failed_message_contains_cmd_and_code() {
        let e = EcluseError::HookFailed {
            cmd: "npm install".into(),
            code: 1,
        };
        let msg = e.to_string();
        assert!(msg.contains("npm install"));
        assert!(msg.contains('1'));
    }

    #[test]
    fn not_a_git_repo_message() {
        let e = EcluseError::NotAGitRepo;
        assert!(e.to_string().contains("git"));
    }

    #[test]
    fn mode_invalid_message_contains_mode_and_valid_list() {
        let e = EcluseError::ModeInvalid("garbage".into());
        let msg = e.to_string();
        assert!(msg.contains("garbage"));
        assert!(msg.contains("valid:"));
    }
}

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

    #[error(
        "no free port found for service '{service}' (tried {range} alternatives starting at {nominal}); \
         free a port or increase port_search_range"
    )]
    PortExhausted {
        service: String,
        nominal: u16,
        range: u8,
    },

    #[error("compose file has no service labeled ecluse.role=app; all services treated as data")]
    AppLabelMissing,

    #[error("hook failed with exit code {code}: {cmd}")]
    HookFailed { cmd: String, code: i32 },

    #[error("not inside a git repository")]
    NotAGitRepo,

    #[error("unknown mode '{0}'; valid: container, host, hybrid")]
    ModeInvalid(String),
}
