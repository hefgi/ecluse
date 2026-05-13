use anyhow::{Context, Result};
use indexmap::IndexMap;
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
    /// Named ports within a slot. Each entry maps a service name to an index
    /// offset within the stride (0 = base_port + slot*stride, 1 = +1, etc.).
    /// Generates ECLUSE_<NAME>_PORT env vars. The port at index 0 is also
    /// exported as PORT for single-service compatibility.
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub ports: IndexMap<String, u8>,
    #[serde(default, skip_serializing_if = "HookConfig::is_empty")]
    pub hooks: HookConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct HookConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_up: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_down: Option<String>,
}

impl HookConfig {
    pub fn is_empty(&self) -> bool {
        self.on_up.is_none() && self.on_down.is_none()
    }
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
    fn hooks_load_from_toml() {
        let dir = TempDir::new().unwrap();
        write_toml(
            &dir,
            r#"
mode = "host"
[hooks]
on_up = "prisma migrate deploy"
on_down = "psql $DATABASE_URL -c 'DROP DATABASE $ECLUSE_DATABASE'"
"#,
        );
        let config = Config::load(dir.path()).unwrap();
        assert_eq!(config.hooks.on_up.as_deref(), Some("prisma migrate deploy"));
        assert!(config.hooks.on_down.is_some());
    }

    #[test]
    fn hooks_are_optional() {
        let dir = TempDir::new().unwrap();
        write_toml(&dir, "mode = \"host\"\n");
        let config = Config::load(dir.path()).unwrap();
        assert!(config.hooks.on_up.is_none());
        assert!(config.hooks.on_down.is_none());
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
            ports: Default::default(),
            hooks: HookConfig::default(),
        };
        original.save(dir.path()).unwrap();
        let loaded = Config::load(dir.path()).unwrap();
        assert_eq!(loaded.mode, Mode::Hybrid);
        assert_eq!(loaded.max_slots, 6);
        assert_eq!(loaded.stride, 200);
        assert_eq!(loaded.prefix, "test");
    }

    #[test]
    fn config_invalid_toml_returns_error() {
        let dir = TempDir::new().unwrap();
        write_toml(&dir, "mode = invalid_value_not_quoted\n");
        assert!(Config::load(dir.path()).is_err());
    }

    #[test]
    fn config_invalid_mode_value_returns_error() {
        let dir = TempDir::new().unwrap();
        write_toml(&dir, "mode = \"unknown\"\n");
        assert!(Config::load(dir.path()).is_err());
    }

    #[test]
    fn config_ports_load_from_toml() {
        let dir = TempDir::new().unwrap();
        write_toml(
            &dir,
            r#"
mode = "host"
[ports]
api = 0
worker = 1
frontend = 2
"#,
        );
        let config = Config::load(dir.path()).unwrap();
        assert_eq!(config.ports.len(), 3);
        assert_eq!(config.ports["api"], 0);
        assert_eq!(config.ports["worker"], 1);
        assert_eq!(config.ports["frontend"], 2);
    }

    #[test]
    fn hook_is_empty_both_none() {
        let h = HookConfig::default();
        assert!(h.is_empty());
    }

    #[test]
    fn hook_is_not_empty_when_on_up_set() {
        let h = HookConfig {
            on_up: Some("echo hi".into()),
            on_down: None,
        };
        assert!(!h.is_empty());
    }

    #[test]
    fn hook_is_not_empty_when_on_down_set() {
        let h = HookConfig {
            on_up: None,
            on_down: Some("echo bye".into()),
        };
        assert!(!h.is_empty());
    }

    #[test]
    fn config_container_mode_display() {
        let dir = TempDir::new().unwrap();
        write_toml(&dir, "mode = \"container\"\n");
        let config = Config::load(dir.path()).unwrap();
        assert_eq!(config.mode.to_string(), "container");
    }

    #[test]
    fn config_default_worktree_dir() {
        let dir = TempDir::new().unwrap();
        write_toml(&dir, "mode = \"host\"\n");
        let config = Config::load(dir.path()).unwrap();
        assert_eq!(config.worktree_dir, ".ecluse/worktrees");
    }

    #[test]
    fn config_default_app_label_and_value() {
        let dir = TempDir::new().unwrap();
        write_toml(&dir, "mode = \"host\"\n");
        let config = Config::load(dir.path()).unwrap();
        assert_eq!(config.app_label, "ecluse.role");
        assert_eq!(config.app_label_value, "app");
    }

    #[test]
    fn mode_from_str_case_sensitive() {
        assert!("Container".parse::<Mode>().is_err());
        assert!("HOST".parse::<Mode>().is_err());
        assert!("Hybrid".parse::<Mode>().is_err());
    }

    #[test]
    fn find_and_load_errors_without_config() {
        // Can only test when current dir has no .ecluse.toml traversal up
        // Use a temp dir as the cwd substitute — we test via Config::load directly
        let dir = TempDir::new().unwrap();
        let result = Config::load(dir.path());
        assert!(result.is_err());
    }
}
