use anyhow::{Context, Result};
use serde::{Deserialize, Deserializer, Serialize};
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

/// How an `inherit_env` entry is materialized in each worktree.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum InheritEnvMode {
    /// File in the worktree is a symlink to the root file. Edits propagate both ways.
    #[default]
    Symlink,
    /// File is copied from root once. Worktree edits stay local; root edits don't propagate.
    Copy,
}

/// A single `inherit_env` entry: which file to inherit, and how.
///
/// In TOML, accepts either a bare string (defaults to `symlink`) or an object form:
/// ```toml
/// inherit_env = [
///   ".env",                                  # symlink (default)
///   { file = ".env.local", mode = "copy" },  # copied once, then independent
/// ]
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InheritEnvEntry {
    pub file: String,
    pub mode: InheritEnvMode,
}

impl InheritEnvEntry {
    pub fn symlink<S: Into<String>>(file: S) -> Self {
        Self {
            file: file.into(),
            mode: InheritEnvMode::Symlink,
        }
    }

    #[cfg(test)]
    pub fn copy<S: Into<String>>(file: S) -> Self {
        Self {
            file: file.into(),
            mode: InheritEnvMode::Copy,
        }
    }
}

impl Serialize for InheritEnvEntry {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // Bare-string form when mode is the default (symlink), so default configs
        // round-trip as the simple list form. Object form otherwise.
        if self.mode == InheritEnvMode::Symlink {
            serializer.serialize_str(&self.file)
        } else {
            use serde::ser::SerializeStruct;
            let mut s = serializer.serialize_struct("InheritEnvEntry", 2)?;
            s.serialize_field("file", &self.file)?;
            s.serialize_field("mode", &self.mode)?;
            s.end()
        }
    }
}

impl<'de> Deserialize<'de> for InheritEnvEntry {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::{self, MapAccess, Visitor};
        use std::fmt;

        struct EntryVisitor;

        impl<'de> Visitor<'de> for EntryVisitor {
            type Value = InheritEnvEntry;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(
                    f,
                    "a filename string or {{ file = \"...\", mode = \"symlink\" | \"copy\" }}"
                )
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<InheritEnvEntry, E> {
                Ok(InheritEnvEntry::symlink(v))
            }

            fn visit_string<E: de::Error>(self, v: String) -> Result<InheritEnvEntry, E> {
                Ok(InheritEnvEntry::symlink(v))
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<InheritEnvEntry, A::Error> {
                let mut file: Option<String> = None;
                let mut mode: Option<InheritEnvMode> = None;
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "file" => file = Some(map.next_value()?),
                        "mode" => mode = Some(map.next_value()?),
                        other => {
                            return Err(de::Error::unknown_field(other, &["file", "mode"]));
                        }
                    }
                }
                let file = file.ok_or_else(|| de::Error::missing_field("file"))?;
                Ok(InheritEnvEntry {
                    file,
                    mode: mode.unwrap_or_default(),
                })
            }
        }

        deserializer.deserialize_any(EntryVisitor)
    }
}

/// A secondary port allocation on a service: allocated as `base_port + slot` on the host,
/// injected into the process environment under `port_env`.
///
/// For docker services, the overlay publishes `(base_port+slot) → container_port`.
/// `container_port` defaults to `base_port` when omitted. Set it explicitly when the
/// in-container listener port differs from the host range base — e.g. to publish Postgres
/// (which listens on 5432 inside the container) on host port 11533:
/// ```toml
/// extra_ports = [{ base_port = 11532, port_env = "PGPORT", container_port = 5432 }]
/// ```
/// This publishes `11533→5432` and sets `PGPORT=11533` in the env.
///
/// When `container_port` is set, the primary `base_port` of the parent service is **not**
/// published to the host — only the extra_ports entries are. This lets you keep the
/// canonical port (5432, 3306) unpublished while exposing a high-numbered host range.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct ExtraPort {
    pub base_port: u16,
    pub port_env: String,
    /// Container-side port for the docker overlay mapping. Defaults to `base_port`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container_port: Option<u16>,
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
    /// Extra env var names to set to this service's allocated port.
    /// Accepts a single string or an array of strings.
    /// e.g. port_env = "DJANGO_PORT" or port_env = ["DJANGO_PORT", "APP_PORT"]
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "deserialize_string_or_vec"
    )]
    pub port_env: Vec<String>,
    /// Deprecated — use `extra_ports` instead.
    /// Kept for backward compatibility; maps to `ECLUSE_<NAME>_DEBUG_PORT`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debug_port: Option<u16>,
    /// Additional port allocations for this service. Each entry is allocated as
    /// `base_port + slot` and injected under `port_env`. Use for debugger ports,
    /// auxiliary listeners, or any secondary port the service exposes.
    ///
    /// Example:
    /// ```toml
    /// extra_ports = [{ base_port = 9229, port_env = "NODE_INSPECT_PORT" }]
    /// ```
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_ports: Vec<ExtraPort>,
    /// Whether the service's primary port is published to the host.
    /// Defaults to the legacy implicit rule: published unless any extra_port
    /// sets `container_port`. Set it explicitly — the implicit rule is
    /// deprecated (`ecluse validate` warns).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publish_primary: Option<bool>,
    /// Host-side port range base for docker services. When set, the overlay publishes
    /// `(host_port+slot) → base_port` instead of `(base_port+slot) → base_port`.
    /// Defaults to `base_port` when omitted — zero behavior change for existing configs.
    ///
    /// Use this when the container listens on a well-known port (e.g. 5432) but you
    /// want host ports in a high-numbered range (e.g. 11532+slot):
    /// ```toml
    /// base_port = 5432    # container-internal port
    /// host_port = 11532   # host range base; slot 1 → 11533
    /// ```
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_port: Option<u16>,
}

fn deserialize_string_or_vec<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::{self, SeqAccess, Visitor};
    use std::fmt;

    struct StringOrVec;

    impl<'de> Visitor<'de> for StringOrVec {
        type Value = Vec<String>;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            write!(f, "a string or array of strings")
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<Vec<String>, E> {
            Ok(vec![v.to_owned()])
        }

        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<String>, A::Error> {
            let mut out = vec![];
            while let Some(s) = seq.next_element::<String>()? {
                out.push(s);
            }
            Ok(out)
        }
    }

    deserializer.deserialize_any(StringOrVec)
}

impl ServiceConfig {
    /// Returns the base used for host-side port allocation.
    /// When `host_port` is set, that is used; otherwise falls back to `base_port`.
    pub fn host_port_base(&self) -> u16 {
        self.host_port.unwrap_or(self.base_port)
    }

    /// Compute the allocated port for `slot` using the given `slot_stride`.
    /// `port = host_port_base + slot * slot_stride`. Use stride=1 for default behavior.
    pub fn port(&self, slot: u8, slot_stride: u8) -> u16 {
        let stride = slot_stride.max(1) as u16;
        self.host_port_base()
            .saturating_add((slot as u16).saturating_mul(stride))
    }

    /// Host port for a secondary (`extra_ports`) allocation — same per-slot
    /// spacing rule as primary ports so `slot_stride` means one thing.
    pub fn extra_port_for_slot(base: u16, slot: u8, slot_stride: u8) -> u16 {
        let stride = slot_stride.max(1) as u16;
        base.saturating_add((slot as u16).saturating_mul(stride))
    }

    /// Returns all extra port allocations as `(host_base_port, env_var_name)` pairs.
    /// Merges `extra_ports` (new) with `debug_port` (legacy) so all callers go through one path.
    pub fn all_extra_ports(&self) -> Vec<(u16, String)> {
        let mut result: Vec<(u16, String)> = self
            .extra_ports
            .iter()
            .map(|ep| (ep.base_port, ep.port_env.clone()))
            .collect();
        if let Some(dp) = self.debug_port {
            let key = format!("ECLUSE_{}_DEBUG_PORT", self.name.to_uppercase());
            if !result.iter().any(|(_, env)| env == &key) {
                result.push((dp, key));
            }
        }
        result
    }

    /// Returns extra port entries as `(host_base_port, container_port)` pairs for overlay generation.
    /// `container_port` is `ep.container_port` when set, otherwise `ep.base_port`.
    pub fn extra_port_mappings(&self) -> Vec<(u16, u16)> {
        self.extra_ports
            .iter()
            .map(|ep| (ep.base_port, ep.container_port.unwrap_or(ep.base_port)))
            .collect()
    }

    /// True when the primary `base_port` must not be published to the host
    /// (only the extra_ports entries are). `publish_primary` wins when set;
    /// otherwise the legacy implicit rule applies: suppressed when any
    /// extra_port has an explicit `container_port`.
    pub fn suppress_primary_publish(&self) -> bool {
        match self.publish_primary {
            Some(publish) => !publish,
            None => self
                .extra_ports
                .iter()
                .any(|ep| ep.container_port.is_some()),
        }
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
    /// Each candidate is `nominal + i * max_slots * slot_stride` to avoid stealing another slot's port.
    /// Guard: port_search_range * max_slots * slot_stride must not exceed the gap between adjacent services.
    #[serde(default = "default_port_search_range")]
    pub port_search_range: u8,
    /// Spacing between ports of adjacent slots. With `slot_stride = 10`, slots 1/2/3
    /// get ports `base+10`, `base+20`, `base+30` instead of `base+1`, `base+2`, `base+3`.
    /// Wider spacing reduces the chance of agents misidentifying adjacent-slot ports
    /// as their own. Defaults to 1 (no change for existing configs).
    #[serde(
        default = "default_slot_stride",
        skip_serializing_if = "is_default_slot_stride"
    )]
    pub slot_stride: u8,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub services: Vec<ServiceConfig>,
    #[serde(default, skip_serializing_if = "HookConfig::is_empty")]
    pub hooks: HookConfig,
    /// Files to inherit from the main worktree root into each new worktree at `ecluse up` time.
    /// Each entry is either a bare string (defaults to `mode = "symlink"`) or an object
    /// `{ file = "...", mode = "symlink" | "copy" }`. Defaults to symlinking
    /// `.env` and `.env.local`. Set to `[]` to opt out entirely.
    ///
    /// - `symlink` (default): file in the worktree is a symlink to root; edits to either
    ///   side affect both. Good for shared secrets that should stay in sync.
    /// - `copy`: file is copied from root once on first `ecluse up`; future edits in the
    ///   worktree stay local. Good for per-worktree feature flags / overrides.
    #[serde(default = "default_inherit_env", skip_serializing_if = "Vec::is_empty")]
    pub inherit_env: Vec<InheritEnvEntry>,
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

#[derive(Debug, Clone, Serialize, Default)]
pub struct HookConfig {
    /// Runs before any infrastructure is created (no env vars available yet).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pre_up: Option<String>,
    /// Runs after ports are allocated and .env.ecluse is written, but before
    /// native services are spawned. Full ECLUSE_* env is available — use this
    /// to derive URLs or write extra vars that services need at startup.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pre_spawn: Option<String>,
    /// Runs after all services are up and the process manager has spawned native services.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_up: Option<String>,
    /// Runs before services are killed or containers are stopped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pre_down: Option<String>,
    /// Runs after all services are stopped and the worktree is removed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_down: Option<String>,
}

impl HookConfig {
    pub fn is_empty(&self) -> bool {
        self.pre_up.is_none()
            && self.pre_spawn.is_none()
            && self.post_up.is_none()
            && self.pre_down.is_none()
            && self.post_down.is_none()
    }
}

impl<'de> serde::Deserialize<'de> for HookConfig {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Raw {
            #[serde(default)]
            pre_up: Option<String>,
            #[serde(default)]
            pre_spawn: Option<String>,
            #[serde(default)]
            post_up: Option<String>,
            #[serde(default)]
            pre_down: Option<String>,
            #[serde(default)]
            post_down: Option<String>,
            // Deprecated aliases — on_up maps to pre_up, on_down maps to pre_down.
            #[serde(default)]
            on_up: Option<String>,
            #[serde(default)]
            on_down: Option<String>,
        }
        let raw = Raw::deserialize(deserializer)?;
        Ok(HookConfig {
            pre_up: raw.pre_up.or(raw.on_up),
            pre_spawn: raw.pre_spawn,
            post_up: raw.post_up,
            pre_down: raw.pre_down.or(raw.on_down),
            post_down: raw.post_down,
        })
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
fn default_slot_stride() -> u8 {
    1
}
fn is_default_slot_stride(v: &u8) -> bool {
    *v == 1
}
fn default_inherit_env() -> Vec<InheritEnvEntry> {
    vec![
        InheritEnvEntry::symlink(".env"),
        InheritEnvEntry::symlink(".env.local"),
    ]
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
        // Use git to find the main worktree root so that running ecluse from
        // inside an ecluse-managed worktree (which also contains .ecluse.toml)
        // doesn't accidentally treat the worktree as the project root.
        if let Ok(root) = crate::worktree::WorktreeManager::main_worktree_root(&cwd) {
            if root.join(".ecluse.toml").exists() {
                let config = Self::load(&root)?;
                return Ok((config, root));
            }
            // In a git repo but no .ecluse.toml — give a context-aware hint.
            let is_linked_worktree = crate::worktree::is_inside_git_worktree(&cwd);
            if is_linked_worktree {
                return Err(anyhow::anyhow!(
                    "no .ecluse.toml found; run `ecluse init` in the main repo root ({})",
                    root.display()
                ));
            } else {
                return Err(anyhow::anyhow!(
                    "no .ecluse.toml found; run `ecluse init` to set up ecluse for this repo"
                ));
            }
        }
        // Fall back to filesystem walk for non-git directories.
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
pre_up = "echo pre_up"
post_up = "prisma migrate deploy"
pre_down = "psql $DATABASE_URL -c 'DROP DATABASE $ECLUSE_DATABASE'"
post_down = "echo post_down"
"#,
        );
        let config = Config::load(dir.path()).unwrap();
        assert_eq!(config.hooks.pre_up.as_deref(), Some("echo pre_up"));
        assert_eq!(
            config.hooks.post_up.as_deref(),
            Some("prisma migrate deploy")
        );
        assert!(config.hooks.pre_down.is_some());
        assert!(config.hooks.post_down.is_some());
    }

    #[test]
    fn hooks_deprecated_on_up_maps_to_pre_up() {
        let dir = TempDir::new().unwrap();
        write_toml(
            &dir,
            r#"
mode = "host"
[hooks]
on_up = "prisma migrate deploy"
on_down = "echo bye"
"#,
        );
        let config = Config::load(dir.path()).unwrap();
        assert_eq!(
            config.hooks.pre_up.as_deref(),
            Some("prisma migrate deploy")
        );
        assert_eq!(config.hooks.pre_down.as_deref(), Some("echo bye"));
        assert!(config.hooks.post_up.is_none());
        assert!(config.hooks.post_down.is_none());
    }

    #[test]
    fn hooks_are_optional() {
        let dir = TempDir::new().unwrap();
        write_toml(&dir, "mode = \"host\"\n");
        let config = Config::load(dir.path()).unwrap();
        assert!(config.hooks.pre_up.is_none());
        assert!(config.hooks.post_up.is_none());
        assert!(config.hooks.pre_down.is_none());
        assert!(config.hooks.post_down.is_none());
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
            slot_stride: 1,
            services: vec![],
            hooks: HookConfig::default(),
            inherit_env: vec![],
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
            port_env: vec![],
            debug_port: None,
            extra_ports: vec![],
            publish_primary: None,
            host_port: None,
        };
        assert_eq!(svc.port(1, 1), 8001);
        assert_eq!(svc.port(2, 1), 8002);
        assert_eq!(svc.port(8, 1), 8008);
    }

    #[test]
    fn debug_port_loads_from_toml() {
        let dir = TempDir::new().unwrap();
        write_toml(
            &dir,
            "mode = \"host\"\n[[services]]\nname = \"app\"\nbase_port = 7100\ncommand = \"vite\"\ndebug_port = 9229\n",
        );
        let config = Config::load(dir.path()).unwrap();
        assert_eq!(config.services[0].debug_port, Some(9229));
    }

    #[test]
    fn debug_port_defaults_to_none() {
        let dir = TempDir::new().unwrap();
        write_toml(
            &dir,
            "mode = \"host\"\n[[services]]\nname = \"app\"\nbase_port = 7100\ncommand = \"vite\"\n",
        );
        let config = Config::load(dir.path()).unwrap();
        assert_eq!(config.services[0].debug_port, None);
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
            slot_stride: 1,
            services: vec![
                ServiceConfig {
                    name: "api".into(),
                    base_port: 8000,
                    run: ServiceRun::Native,
                    compose: None,
                    command: None,
                    port_env: vec![],
                    debug_port: None,
                    extra_ports: vec![],
                    publish_primary: None,
                    host_port: None,
                },
                ServiceConfig {
                    name: "postgres".into(),
                    base_port: 5432,
                    run: ServiceRun::Docker,
                    compose: None,
                    command: None,
                    port_env: vec![],
                    debug_port: None,
                    extra_ports: vec![],
                    publish_primary: None,
                    host_port: None,
                },
            ],
            hooks: HookConfig::default(),
            inherit_env: vec![],
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
    fn hook_is_not_empty_when_pre_up_set() {
        let h = HookConfig {
            pre_up: Some("echo hi".into()),
            ..Default::default()
        };
        assert!(!h.is_empty());
    }

    #[test]
    fn hook_is_not_empty_when_post_up_set() {
        let h = HookConfig {
            post_up: Some("migrate".into()),
            ..Default::default()
        };
        assert!(!h.is_empty());
    }

    #[test]
    fn hook_is_not_empty_when_pre_down_set() {
        let h = HookConfig {
            pre_down: Some("echo bye".into()),
            ..Default::default()
        };
        assert!(!h.is_empty());
    }

    #[test]
    fn hook_is_not_empty_when_post_down_set() {
        let h = HookConfig {
            post_down: Some("echo done".into()),
            ..Default::default()
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
    fn service_port_env_defaults_to_empty() {
        let dir = TempDir::new().unwrap();
        write_toml(
            &dir,
            "mode = \"host\"\n[[services]]\nname = \"api\"\nbase_port = 3000\n",
        );
        let config = Config::load(dir.path()).unwrap();
        assert!(config.services[0].port_env.is_empty());
    }

    #[test]
    fn service_port_env_loads_single_string() {
        let dir = TempDir::new().unwrap();
        write_toml(
            &dir,
            "mode = \"host\"\n[[services]]\nname = \"api\"\nbase_port = 3000\nport_env = \"DJANGO_PORT\"\n",
        );
        let config = Config::load(dir.path()).unwrap();
        assert_eq!(config.services[0].port_env, vec!["DJANGO_PORT"]);
    }

    #[test]
    fn service_port_env_loads_array() {
        let dir = TempDir::new().unwrap();
        write_toml(
            &dir,
            "mode = \"host\"\n[[services]]\nname = \"api\"\nbase_port = 3000\nport_env = [\"DJANGO_PORT\", \"APP_PORT\"]\n",
        );
        let config = Config::load(dir.path()).unwrap();
        assert_eq!(config.services[0].port_env, vec!["DJANGO_PORT", "APP_PORT"]);
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

    #[test]
    fn host_port_defaults_to_none() {
        let dir = TempDir::new().unwrap();
        write_toml(
            &dir,
            "mode = \"hybrid\"\n[[services]]\nname = \"postgres\"\nrun = \"docker\"\nbase_port = 5432\n",
        );
        let config = Config::load(dir.path()).unwrap();
        assert_eq!(config.services[0].host_port, None);
    }

    #[test]
    fn host_port_loads_from_toml() {
        let dir = TempDir::new().unwrap();
        write_toml(
            &dir,
            "mode = \"hybrid\"\n[[services]]\nname = \"postgres\"\nrun = \"docker\"\nbase_port = 5432\nhost_port = 11532\n",
        );
        let config = Config::load(dir.path()).unwrap();
        assert_eq!(config.services[0].host_port, Some(11532));
    }

    #[test]
    fn host_port_base_falls_back_to_base_port_when_unset() {
        let svc = ServiceConfig {
            name: "postgres".into(),
            base_port: 5432,
            run: ServiceRun::Docker,
            compose: None,
            command: None,
            port_env: vec![],
            debug_port: None,
            extra_ports: vec![],
            publish_primary: None,
            host_port: None,
        };
        assert_eq!(svc.host_port_base(), 5432);
        assert_eq!(svc.port(1, 1), 5433);
    }

    #[test]
    fn host_port_base_uses_host_port_when_set() {
        let svc = ServiceConfig {
            name: "postgres".into(),
            base_port: 5432,
            run: ServiceRun::Docker,
            compose: None,
            command: None,
            port_env: vec![],
            debug_port: None,
            extra_ports: vec![],
            publish_primary: None,
            host_port: Some(11532),
        };
        assert_eq!(svc.host_port_base(), 11532);
        assert_eq!(svc.port(1, 1), 11533);
        assert_eq!(svc.port(2, 1), 11534);
    }

    #[test]
    fn port_with_stride_spaces_slots_by_stride() {
        let svc = ServiceConfig {
            name: "api".into(),
            base_port: 3000,
            run: ServiceRun::Native,
            compose: None,
            command: None,
            port_env: vec![],
            debug_port: None,
            extra_ports: vec![],
            publish_primary: None,
            host_port: None,
        };
        assert_eq!(svc.port(1, 10), 3010);
        assert_eq!(svc.port(2, 10), 3020);
        assert_eq!(svc.port(3, 10), 3030);
    }

    #[test]
    fn port_with_stride_zero_is_treated_as_one() {
        let svc = ServiceConfig {
            name: "api".into(),
            base_port: 3000,
            run: ServiceRun::Native,
            compose: None,
            command: None,
            port_env: vec![],
            debug_port: None,
            extra_ports: vec![],
            publish_primary: None,
            host_port: None,
        };
        // Stride of 0 would zero out the slot offset; we clamp to 1 to keep ports unique.
        assert_eq!(svc.port(1, 0), 3001);
        assert_eq!(svc.port(2, 0), 3002);
    }

    #[test]
    fn slot_stride_loads_from_toml() {
        let dir = TempDir::new().unwrap();
        write_toml(&dir, "mode = \"host\"\nslot_stride = 10\n");
        let config = Config::load(dir.path()).unwrap();
        assert_eq!(config.slot_stride, 10);
    }

    #[test]
    fn slot_stride_defaults_to_one() {
        let dir = TempDir::new().unwrap();
        write_toml(&dir, "mode = \"host\"\n");
        let config = Config::load(dir.path()).unwrap();
        assert_eq!(config.slot_stride, 1);
    }

    // ── extra ports & publish_primary ─────────────────────────────────────────

    #[test]
    fn extra_port_for_slot_honors_stride() {
        assert_eq!(ServiceConfig::extra_port_for_slot(9000, 1, 1), 9001);
        assert_eq!(ServiceConfig::extra_port_for_slot(9000, 2, 1), 9002);
        assert_eq!(ServiceConfig::extra_port_for_slot(9000, 1, 10), 9010);
        assert_eq!(ServiceConfig::extra_port_for_slot(9000, 3, 10), 9030);
    }

    #[test]
    fn publish_primary_overrides_implicit_suppression() {
        let mut svc = ServiceConfig {
            name: "db".into(),
            base_port: 5432,
            run: ServiceRun::Docker,
            compose: None,
            command: None,
            port_env: vec![],
            debug_port: None,
            extra_ports: vec![ExtraPort {
                base_port: 11532,
                port_env: "PGPORT".into(),
                container_port: Some(5432),
            }],
            publish_primary: None,
            host_port: None,
        };
        // Implicit rule: container_port set → suppressed.
        assert!(svc.suppress_primary_publish());
        // Explicit publish_primary wins in both directions.
        svc.publish_primary = Some(true);
        assert!(!svc.suppress_primary_publish());
        svc.publish_primary = Some(false);
        svc.extra_ports.clear();
        assert!(svc.suppress_primary_publish());
    }

    // ── inherit_env ───────────────────────────────────────────────────────────

    #[test]
    fn inherit_env_default_is_symlinked_env_and_env_local() {
        let dir = TempDir::new().unwrap();
        write_toml(&dir, "mode = \"host\"\n");
        let config = Config::load(dir.path()).unwrap();
        assert_eq!(
            config.inherit_env,
            vec![
                InheritEnvEntry::symlink(".env"),
                InheritEnvEntry::symlink(".env.local"),
            ]
        );
    }

    #[test]
    fn inherit_env_bare_string_defaults_to_symlink() {
        let dir = TempDir::new().unwrap();
        write_toml(&dir, "mode = \"host\"\ninherit_env = [\".env\"]\n");
        let config = Config::load(dir.path()).unwrap();
        assert_eq!(config.inherit_env, vec![InheritEnvEntry::symlink(".env")]);
    }

    #[test]
    fn inherit_env_object_form_loads_correctly() {
        let dir = TempDir::new().unwrap();
        write_toml(
            &dir,
            "mode = \"host\"\ninherit_env = [{ file = \".env.local\", mode = \"copy\" }]\n",
        );
        let config = Config::load(dir.path()).unwrap();
        assert_eq!(
            config.inherit_env,
            vec![InheritEnvEntry::copy(".env.local")]
        );
    }

    #[test]
    fn inherit_env_mixed_forms_load_correctly() {
        let dir = TempDir::new().unwrap();
        write_toml(
            &dir,
            "mode = \"host\"\ninherit_env = [\".env\", { file = \".env.local\", mode = \"copy\" }]\n",
        );
        let config = Config::load(dir.path()).unwrap();
        assert_eq!(
            config.inherit_env,
            vec![
                InheritEnvEntry::symlink(".env"),
                InheritEnvEntry::copy(".env.local"),
            ]
        );
    }

    #[test]
    fn inherit_env_object_without_mode_defaults_to_symlink() {
        let dir = TempDir::new().unwrap();
        write_toml(
            &dir,
            "mode = \"host\"\ninherit_env = [{ file = \".env\" }]\n",
        );
        let config = Config::load(dir.path()).unwrap();
        assert_eq!(config.inherit_env, vec![InheritEnvEntry::symlink(".env")]);
    }

    #[test]
    fn inherit_env_invalid_mode_returns_error() {
        let dir = TempDir::new().unwrap();
        write_toml(
            &dir,
            "mode = \"host\"\ninherit_env = [{ file = \".env\", mode = \"hardlink\" }]\n",
        );
        assert!(Config::load(dir.path()).is_err());
    }

    #[test]
    fn inherit_env_empty_list_opts_out() {
        let dir = TempDir::new().unwrap();
        write_toml(&dir, "mode = \"host\"\ninherit_env = []\n");
        let config = Config::load(dir.path()).unwrap();
        assert!(config.inherit_env.is_empty());
    }

    #[test]
    fn inherit_env_symlink_entry_serializes_as_bare_string() {
        let entry = InheritEnvEntry::symlink(".env");
        let s = toml::to_string(&entry).unwrap_or_else(|_| {
            // Direct serialization of a single entry isn't directly representable;
            // verify the round-trip through a Config instead.
            String::new()
        });
        // Round-trip: a Config with default inherit_env serializes the entries
        // as bare strings (since they're all symlink mode).
        let config = Config {
            mode: Mode::Host,
            max_slots: 8,
            prefix: "ecluse".into(),
            worktree_dir: ".ecluse/worktrees".into(),
            app_label: "ecluse.role".into(),
            app_label_value: "app".into(),
            strict_port: false,
            port_search_range: 10,
            slot_stride: 1,
            services: vec![],
            hooks: HookConfig::default(),
            inherit_env: vec![
                InheritEnvEntry::symlink(".env"),
                InheritEnvEntry::copy(".env.local"),
            ],
        };
        let toml_str = toml::to_string(&config).unwrap();
        // Symlink entry: bare string.
        assert!(toml_str.contains("\".env\""));
        // Copy entry: object form with explicit mode.
        assert!(toml_str.contains("mode = \"copy\""));
        let _ = s; // silence unused warning if alternative path didn't populate it
    }
}
