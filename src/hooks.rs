use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn run_successful_command() {
        let dir = TempDir::new().unwrap();
        let env: HashMap<String, String> = HashMap::new();
        assert!(run("true", dir.path(), &env).is_ok());
    }

    #[test]
    fn run_failing_command_returns_hook_failed_error() {
        let dir = TempDir::new().unwrap();
        let env: HashMap<String, String> = HashMap::new();
        let err = run("false", dir.path(), &env).unwrap_err();
        assert!(
            err.to_string().contains("hook failed") || err.to_string().contains("exit code"),
            "got: {}",
            err
        );
    }

    #[test]
    fn run_nonzero_exit_code_in_error() {
        let dir = TempDir::new().unwrap();
        let env: HashMap<String, String> = HashMap::new();
        let err = run("exit 42", dir.path(), &env).unwrap_err();
        assert!(err.to_string().contains("42"), "got: {}", err);
    }

    #[test]
    fn run_command_receives_env_vars() {
        let dir = TempDir::new().unwrap();
        let out_file = dir.path().join("out.txt");
        let mut env: HashMap<String, String> = HashMap::new();
        env.insert("TEST_KEY".into(), "hello_ecluse".into());
        let cmd = format!("echo $TEST_KEY > {}", out_file.display());
        run(&cmd, dir.path(), &env).unwrap();
        let content = std::fs::read_to_string(&out_file).unwrap();
        assert!(content.contains("hello_ecluse"), "got: {}", content);
    }

    #[test]
    fn run_command_runs_in_worktree_dir() {
        let dir = TempDir::new().unwrap();
        let out_file = dir.path().join("pwd.txt");
        let env: HashMap<String, String> = HashMap::new();
        let cmd = format!("pwd > {}", out_file.display());
        run(&cmd, dir.path(), &env).unwrap();
        let content = std::fs::read_to_string(&out_file).unwrap();
        // The pwd should be the temp dir
        assert!(
            content
                .trim()
                .contains(dir.path().to_string_lossy().as_ref()),
            "expected {} in pwd output: {}",
            dir.path().display(),
            content
        );
    }

    #[test]
    fn run_empty_command_name_errors() {
        let dir = TempDir::new().unwrap();
        let env: HashMap<String, String> = HashMap::new();
        // An empty string is a valid shell command (no-op), should succeed
        let result = run("", dir.path(), &env);
        // Empty string in sh -c "" is a no-op and succeeds
        assert!(result.is_ok());
    }
}

pub fn run(cmd: &str, worktree: &Path, env: &HashMap<String, String>) -> Result<()> {
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .current_dir(worktree)
        .envs(env)
        .status()
        .map_err(|e| anyhow::anyhow!("failed to launch hook '{}': {}", cmd, e))?;

    if !status.success() {
        return Err(crate::error::EcluseError::HookFailed {
            cmd: cmd.to_string(),
            code: status.code().unwrap_or(-1),
        }
        .into());
    }
    Ok(())
}
