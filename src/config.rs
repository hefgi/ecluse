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

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ServiceRun {
    Native,
    Docker,
}

#[allow(clippy::derivable_impls)]
impl Default for ServiceRun {
    fn default() -> Self {
        ServiceRun::Native
    }
}

impl std::fmt::Display for ServiceRun {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServiceRun::Native => write!(f, "native"),
            ServiceRun::Docker => write!(f, "docker"),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServiceConfig {
    pub name: String,
    pub base_port: u16,
    #[serde(default)]
    pub run: ServiceRun,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compose: Option<String>,
    /// Shell command to spawn on `ecluse up`. Only used for native services
    /// when process_manager is configured in ~/.config/ecluse/config.toml.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
}

impl ServiceConfig {
    pub fn port(&self, slot: u8) -> u16 {
        self.base_port + slot as u16
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub mode: Mode,
    #[serde(default = "default_max_slots")]
    pub max_slots: u8,
    #[serde(default = "default_prefix")]
    pub prefix: String,
    #[serde(default = "default_worktree_dir")]
    pub worktree_dir: String,
    #[serde(default = "default_app_label")]
    pub app_label: String,
    #[serde(default = "default_app_label_value")]
    pub app_label_value: String,
    /// When true, fail immediately on port collision instead of searching for a free port.
    #[serde(default, skip_serializing_if = "is_false")]
    pub strict_port: bool,
    /// Number of alternative ports to try per service when a port is already in use.
    /// Each candidate is `nominal + i * max_slots` to avoid stealing another slot's port.
    /// Guard: port_search_range * max_slots must not exceed the gap between adjacent services.
    #[serde(default = "default_port_search_range")]
    pub port_search_range: u8,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub services: Vec<ServiceConfig>,
    #[serde(default, skip_serializing_if = "HookConfig::is_empty")]
    pub hooks: HookConfig,
}

impl Config {
    pub fn native_services(&self) -> Vec<&ServiceConfig> {
        self.services
            .iter()
            .filter(|s| s.run == ServiceRun::Native)
            .collect()
    }

    pub fn docker_services(&self) -> Vec<&ServiceConfig> {
        self.services
            .iter()
            .filter(|s| s.run == ServiceRun::Docker)
            .collect()
    }
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
fn default_port_search_range() -> u8 {
    10
}
fn is_false(v: &bool) -> bool {
    !v
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
        assert_eq!(config.prefix, "ecluse");
        assert!(config.services.is_empty());
    }

    #[test]
    fn config_loads_full_toml() {
        let dir = TempDir::new().unwrap();
        write_toml(
            &dir,
            r#"
mode = "hybrid"
max_slots = 4
prefix = "myapp"
worktree_dir = ".wt"
app_label = "role"
app_label_value = "web"
"#,
        );
        let config = Config::load(dir.path()).unwrap();
        assert_eq!(config.mode, Mode::Hybrid);
        assert_eq!(config.max_slots, 4);
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
            prefix: "test".into(),
            worktree_dir: ".ecluse/worktrees".into(),
            app_label: "ecluse.role".into(),
            app_label_value: "app".into(),
            strict_port: false,
            port_search_range: 10,
            services: vec![],
            hooks: HookConfig::default(),
        };
        original.save(dir.path()).unwrap();
        let loaded = Config::load(dir.path()).unwrap();
        assert_eq!(loaded.mode, Mode::Hybrid);
        assert_eq!(loaded.max_slots, 6);
        assert_eq!(loaded.prefix, "test");
    }

    #[test]
    fn config_strict_port_defaults_to_false() {
        let dir = TempDir::new().unwrap();
        write_toml(&dir, "mode = \"host\"\n");
        let config = Config::load(dir.path()).unwrap();
        assert!(!config.strict_port);
    }

    #[test]
    fn config_port_search_range_defaults_to_10() {
        let dir = TempDir::new().unwrap();
        write_toml(&dir, "mode = \"host\"\n");
        let config = Config::load(dir.path()).unwrap();
        assert_eq!(config.port_search_range, 10);
    }

    #[test]
    fn config_strict_port_can_be_set() {
        let dir = TempDir::new().unwrap();
        write_toml(&dir, "mode = \"host\"\nstrict_port = true\n");
        let config = Config::load(dir.path()).unwrap();
        assert!(config.strict_port);
    }

    #[test]
    fn config_port_search_range_can_be_set() {
        let dir = TempDir::new().unwrap();
        write_toml(&dir, "mode = \"host\"\nport_search_range = 5\n");
        let config = Config::load(dir.path()).unwrap();
        assert_eq!(config.port_search_range, 5);
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
    fn config_services_load_from_toml() {
        let dir = TempDir::new().unwrap();
        write_toml(
            &dir,
            r#"
mode = "hybrid"

[[services]]
name = "api"
run = "native"
base_port = 8000

[[services]]
name = "postgres"
run = "docker"
base_port = 5432
"#,
        );
        let config = Config::load(dir.path()).unwrap();
        assert_eq!(config.services.len(), 2);
        assert_eq!(config.services[0].name, "api");
        assert_eq!(config.services[0].base_port, 8000);
        assert_eq!(config.services[0].run, ServiceRun::Native);
        assert_eq!(config.services[1].name, "postgres");
        assert_eq!(config.services[1].base_port, 5432);
        assert_eq!(config.services[1].run, ServiceRun::Docker);
    }

    #[test]
    fn service_config_port_computes_correctly() {
        let svc = ServiceConfig {
            name: "api".into(),
            base_port: 8000,
            run: ServiceRun::Native,
            compose: None,
            command: None,
        };
        assert_eq!(svc.port(1), 8001);
        assert_eq!(svc.port(2), 8002);
        assert_eq!(svc.port(8), 8008);
    }

    #[test]
    fn config_native_services_filters_correctly() {
        let config = Config {
            mode: Mode::Hybrid,
            max_slots: 8,
            prefix: "ecluse".into(),
            worktree_dir: ".ecluse/worktrees".into(),
            app_label: "ecluse.role".into(),
            app_label_value: "app".into(),
            strict_port: false,
            port_search_range: 10,
            services: vec![
                ServiceConfig {
                    name: "api".into(),
                    base_port: 8000,
                    run: ServiceRun::Native,
                    compose: None,
                    command: None,
                },
                ServiceConfig {
                    name: "postgres".into(),
                    base_port: 5432,
                    run: ServiceRun::Docker,
                    compose: None,
                    command: None,
                },
            ],
            hooks: HookConfig::default(),
        };
        let native = config.native_services();
        assert_eq!(native.len(), 1);
        assert_eq!(native[0].name, "api");

        let docker = config.docker_services();
        assert_eq!(docker.len(), 1);
        assert_eq!(docker[0].name, "postgres");
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
        let dir = TempDir::new().unwrap();
        let result = Config::load(dir.path());
        assert!(result.is_err());
    }

    #[test]
    fn service_command_is_optional() {
        let dir = TempDir::new().unwrap();
        write_toml(
            &dir,
            "mode = \"host\"\n[[services]]\nname = \"api\"\nbase_port = 3000\n",
        );
        let config = Config::load(dir.path()).unwrap();
        assert!(config.services[0].command.is_none());
    }

    #[test]
    fn service_command_loads_from_toml() {
        let dir = TempDir::new().unwrap();
        write_toml(
            &dir,
            "mode = \"host\"\n[[services]]\nname = \"api\"\nbase_port = 3000\ncommand = \"npm run dev\"\n",
        );
        let config = Config::load(dir.path()).unwrap();
        assert_eq!(config.services[0].command.as_deref(), Some("npm run dev"));
    }

    #[test]
    fn service_run_defaults_to_native() {
        let dir = TempDir::new().unwrap();
        write_toml(
            &dir,
            r#"
mode = "host"

[[services]]
name = "app"
base_port = 3000
"#,
        );
        let config = Config::load(dir.path()).unwrap();
        assert_eq!(config.services[0].run, ServiceRun::Native);
    }
}
