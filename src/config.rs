use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Container,
    Host,
    Hybrid,
}

impl std::fmt::Display for Mode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Mode::Container => write!(f, "container"),
            Mode::Host => write!(f, "host"),
            Mode::Hybrid => write!(f, "hybrid"),
        }
    }
}

impl std::str::FromStr for Mode {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s {
            "container" => Ok(Mode::Container),
            "host" => Ok(Mode::Host),
            "hybrid" => Ok(Mode::Hybrid),
            _ => Err(anyhow::anyhow!("unknown mode '{}'; valid: container, host, hybrid", s)),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub mode: Mode,
    #[serde(default = "default_max_slots")]
    pub max_slots: u8,
    #[serde(default = "default_stride")]
    pub stride: u16,
    #[serde(default = "default_prefix")]
    pub prefix: String,
    #[serde(default = "default_worktree_dir")]
    pub worktree_dir: String,
    #[serde(default = "default_app_label")]
    pub app_label: String,
    #[serde(default = "default_app_label_value")]
    pub app_label_value: String,
    #[serde(default, skip_serializing_if = "DatabaseConfig::is_none")]
    pub database: DatabaseConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct DatabaseConfig {
    #[serde(default = "default_db_provider")]
    pub provider: String,
    #[serde(default = "default_db_host")]
    pub host: String,
    #[serde(default = "default_db_port")]
    pub port: u16,
    #[serde(default = "default_db_user")]
    pub user: String,
    #[serde(default)]
    pub base: String,
}

fn default_max_slots() -> u8 { 8 }
fn default_stride() -> u16 { 100 }
fn default_prefix() -> String { "ecluse".into() }
fn default_worktree_dir() -> String { ".ecluse/worktrees".into() }
fn default_app_label() -> String { "ecluse.role".into() }
fn default_app_label_value() -> String { "app".into() }
fn default_db_provider() -> String { "none".into() }
fn default_db_host() -> String { "localhost".into() }
fn default_db_port() -> u16 { 5432 }
fn default_db_user() -> String { "postgres".into() }

impl Config {
    pub fn load(root: &Path) -> Result<Self> {
        let path = root.join(".ecluse.toml");
        let content = std::fs::read_to_string(&path)
            .with_context(|| crate::error::EcluseError::ConfigMissing)?;
        toml::from_str(&content)
            .with_context(|| format!("failed to parse .ecluse.toml: {}", path.display()))
    }

    pub fn save(&self, root: &Path) -> Result<()> {
        let path = root.join(".ecluse.toml");
        let content = toml::to_string_pretty(self)
            .context("failed to serialize config")?;
        std::fs::write(&path, content)
            .with_context(|| format!("failed to write {}", path.display()))
    }

    pub fn find_and_load() -> Result<(Self, PathBuf)> {
        let cwd = std::env::current_dir().context("could not determine current directory")?;
        let mut dir = cwd.as_path();
        loop {
            let candidate = dir.join(".ecluse.toml");
            if candidate.exists() {
                let config = Self::load(dir)?;
                return Ok((config, dir.to_owned()));
            }
            match dir.parent() {
                Some(parent) => dir = parent,
                None => return Err(crate::error::EcluseError::ConfigMissing.into()),
            }
        }
    }

    pub fn is_db_enabled(&self) -> bool {
        self.database.provider == "postgres-host"
    }
}

impl DatabaseConfig {
    pub fn is_none(&self) -> bool {
        self.provider.is_empty() || self.provider == "none"
    }
}
