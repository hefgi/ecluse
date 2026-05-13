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
    type Err = crate::error::EcluseError;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "container" => Ok(Mode::Container),
            "host" => Ok(Mode::Host),
            "hybrid" => Ok(Mode::Hybrid),
            _ => Err(crate::error::EcluseError::ModeInvalid(s.to_string())),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub mode: Mode,
    #[serde(default = "default_max_slots")]
    pub max_slots: u8,
    #[serde(default = "default_base_port")]
    pub base_port: u16,
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

fn default_max_slots() -> u8 {
    8
}
fn default_base_port() -> u16 {
    3000
}
fn default_stride() -> u16 {
    100
}
fn default_prefix() -> String {
    "ecluse".into()
}
fn default_worktree_dir() -> String {
    ".ecluse/worktrees".into()
}
fn default_app_label() -> String {
    "ecluse.role".into()
}
fn default_app_label_value() -> String {
    "app".into()
}
fn default_db_provider() -> String {
    "none".into()
}
fn default_db_host() -> String {
    "localhost".into()
}
fn default_db_port() -> u16 {
    5432
}
fn default_db_user() -> String {
    "postgres".into()
}

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
        let content = toml::to_string_pretty(self).context("failed to serialize config")?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn write_toml(dir: &TempDir, content: &str) {
        let path = dir.path().join(".ecluse.toml");
        let mut f = std::fs::File::create(path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
    }

    #[test]
    fn mode_from_str_valid() {
        assert_eq!("container".parse::<Mode>().unwrap(), Mode::Container);
        assert_eq!("host".parse::<Mode>().unwrap(), Mode::Host);
        assert_eq!("hybrid".parse::<Mode>().unwrap(), Mode::Hybrid);
    }

    #[test]
    fn mode_from_str_invalid() {
        let err = "nope".parse::<Mode>().unwrap_err();
        assert!(err.to_string().contains("nope"));
        assert!(err.to_string().contains("valid:"));
    }

    #[test]
    fn mode_display_roundtrips() {
        for mode in [Mode::Container, Mode::Host, Mode::Hybrid] {
            let s = mode.to_string();
            assert_eq!(s.parse::<Mode>().unwrap(), mode);
        }
    }

    #[test]
    fn config_loads_minimal_toml() {
        let dir = TempDir::new().unwrap();
        write_toml(&dir, "mode = \"host\"\n");
        let config = Config::load(dir.path()).unwrap();
        assert_eq!(config.mode, Mode::Host);
        assert_eq!(config.max_slots, 8);
        assert_eq!(config.stride, 100);
        assert_eq!(config.prefix, "ecluse");
    }

    #[test]
    fn config_loads_full_toml() {
        let dir = TempDir::new().unwrap();
        write_toml(
            &dir,
            r#"
mode = "hybrid"
max_slots = 4
stride = 50
prefix = "myapp"
worktree_dir = ".wt"
app_label = "role"
app_label_value = "web"
"#,
        );
        let config = Config::load(dir.path()).unwrap();
        assert_eq!(config.mode, Mode::Hybrid);
        assert_eq!(config.max_slots, 4);
        assert_eq!(config.stride, 50);
        assert_eq!(config.prefix, "myapp");
    }

    #[test]
    fn config_missing_returns_error() {
        let dir = TempDir::new().unwrap();
        let err = Config::load(dir.path()).unwrap_err();
        assert!(err.to_string().contains("ecluse init"));
    }

    #[test]
    fn is_db_enabled_true_for_postgres_host() {
        let dir = TempDir::new().unwrap();
        write_toml(
            &dir,
            r#"
mode = "host"
[database]
provider = "postgres-host"
base = "myapp"
"#,
        );
        let config = Config::load(dir.path()).unwrap();
        assert!(config.is_db_enabled());
    }

    #[test]
    fn is_db_enabled_false_by_default() {
        let dir = TempDir::new().unwrap();
        write_toml(&dir, "mode = \"host\"\n");
        let config = Config::load(dir.path()).unwrap();
        assert!(!config.is_db_enabled());
    }

    #[test]
    fn config_roundtrips_save_load() {
        let dir = TempDir::new().unwrap();
        let original = Config {
            mode: Mode::Hybrid,
            max_slots: 6,
            base_port: 3000,
            stride: 200,
            prefix: "test".into(),
            worktree_dir: ".ecluse/worktrees".into(),
            app_label: "ecluse.role".into(),
            app_label_value: "app".into(),
            database: DatabaseConfig::default(),
        };
        original.save(dir.path()).unwrap();
        let loaded = Config::load(dir.path()).unwrap();
        assert_eq!(loaded.mode, Mode::Hybrid);
        assert_eq!(loaded.max_slots, 6);
        assert_eq!(loaded.stride, 200);
        assert_eq!(loaded.prefix, "test");
    }
}
