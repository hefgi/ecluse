use anyhow::{Context, Result};
use std::process::Command;

use crate::config::DatabaseConfig;

pub struct PgClient {
    pub host: String,
    pub port: u16,
    pub user: String,
}

impl PgClient {
    pub fn from_config(cfg: &DatabaseConfig) -> Self {
        Self {
            host: cfg.host.clone(),
            port: cfg.port,
            user: cfg.user.clone(),
        }
    }

    pub fn create_db(&self, name: &str) -> Result<()> {
        let out = self.run_psql(&["-c", &format!("CREATE DATABASE \"{}\";", name)])?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            return Err(crate::error::EcluseError::DatabaseCreateFailed { stderr }.into());
        }
        Ok(())
    }

    pub fn drop_db(&self, name: &str) -> Result<()> {
        let out = self.run_psql(&["-c", &format!("DROP DATABASE IF EXISTS \"{}\";", name)])?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            tracing::warn!("failed to drop database {}: {}", name, stderr);
        }
        Ok(())
    }

    pub fn is_reachable(&self) -> bool {
        // Use a short TCP connect timeout via psql
        let out = Command::new("psql")
            .args([
                "-h",
                &self.host,
                "-p",
                &self.port.to_string(),
                "-U",
                &self.user,
                "-c",
                "SELECT 1",
                "--connect-timeout",
                "1",
                "-q",
                "-t",
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output();
        out.map(|o| o.status.success()).unwrap_or(false)
    }

    pub fn db_name(base: &str, slug: &str) -> String {
        format!("{}_{}", base, slug.replace('-', "_"))
    }

    fn run_psql(&self, args: &[&str]) -> Result<std::process::Output> {
        Command::new("psql")
            .args([
                "-h",
                &self.host,
                "-p",
                &self.port.to_string(),
                "-U",
                &self.user,
            ])
            .args(args)
            .output()
            .context("failed to run psql")
    }
}
