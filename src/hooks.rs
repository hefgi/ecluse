use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;

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
