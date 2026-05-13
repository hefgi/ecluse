use anyhow::{Context, Result};
use indexmap::IndexMap;
use std::collections::HashMap;
use std::path::Path;

/// Build the env map for a session.
///
/// `named_ports` comes from two sources depending on mode:
///   - host/hybrid: from `config.ports` (name → index) resolved to absolute ports
///   - container: from compose service port mappings (name → actual host port)
///
/// For named ports, each entry generates `ECLUSE_<NAME>_PORT`.
/// The first entry also sets `PORT` (primary service alias).
/// `data_services` carries compose-derived ports in container/hybrid mode and
/// are merged in after named ports.
pub fn build_env(
    slot: u8,
    slug: &str,
    offset: u16,
    mode: &str,
    named_ports: &IndexMap<String, u16>,
    data_services: &[(String, u16)],
) -> HashMap<String, String> {
    let mut env = HashMap::new();
    env.insert("ECLUSE_SLOT".into(), slot.to_string());
    env.insert("ECLUSE_SLUG".into(), slug.to_string());
    env.insert("ECLUSE_OFFSET".into(), offset.to_string());
    env.insert("ECLUSE_MODE".into(), mode.to_string());

    // Named ports from [ports] config — order preserved via IndexMap
    let mut first = true;
    for (name, port) in named_ports {
        let key = service_env_key(name);
        env.insert(format!("ECLUSE_{}_PORT", key), port.to_string());
        if first {
            // Primary service → PORT alias for framework compatibility
            env.insert("PORT".into(), port.to_string());
            first = false;
        }
    }

    // Data services from compose (container/hybrid) — ECLUSE_<NAME>_PORT only
    for (name, port) in data_services {
        let key = service_env_key(name);
        env.insert(format!("ECLUSE_{}_PORT", key), port.to_string());
    }

    env
}

/// Resolve config [ports] (name → index) to absolute port numbers for a slot.
pub fn resolve_named_ports(
    config_ports: &IndexMap<String, u8>,
    base_port: u16,
    offset: u16,
) -> IndexMap<String, u16> {
    config_ports
        .iter()
        .map(|(name, &idx)| {
            let port = base_port + offset + idx as u16;
            (name.clone(), port)
        })
        .collect()
}

/// Like `resolve_named_ports` but falls back to a single "app" port at index 0
/// when no [ports] table is defined — preserving backward compatibility.
pub fn effective_named_ports(
    config_ports: &IndexMap<String, u8>,
    base_port: u16,
    offset: u16,
) -> IndexMap<String, u16> {
    if config_ports.is_empty() {
        let mut m = IndexMap::new();
        m.insert("app".to_string(), base_port + offset);
        m
    } else {
        resolve_named_ports(config_ports, base_port, offset)
    }
}

fn service_env_key(name: &str) -> String {
    name.to_uppercase().replace(['-', '.'], "_")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ports(entries: &[(&str, u16)]) -> IndexMap<String, u16> {
        entries
            .iter()
            .map(|(k, v)| (k.to_string(), *v))
            .collect()
    }

    #[test]
    fn basic_env_vars_always_present() {
        let env = build_env(2, "my-task", 200, "host", &IndexMap::new(), &[]);
        assert_eq!(env["ECLUSE_SLOT"], "2");
        assert_eq!(env["ECLUSE_SLUG"], "my-task");
        assert_eq!(env["ECLUSE_OFFSET"], "200");
        assert_eq!(env["ECLUSE_MODE"], "host");
    }

    #[test]
    fn first_named_port_sets_port_alias() {
        let np = ports(&[("api", 3100), ("worker", 3101)]);
        let env = build_env(1, "feat", 100, "host", &np, &[]);
        assert_eq!(env["PORT"], "3100");
        assert_eq!(env["ECLUSE_API_PORT"], "3100");
        assert_eq!(env["ECLUSE_WORKER_PORT"], "3101");
    }

    #[test]
    fn no_port_when_no_named_ports() {
        let env = build_env(1, "feat", 100, "container", &IndexMap::new(), &[]);
        assert!(!env.contains_key("PORT"));
    }

    #[test]
    fn data_services_set_port_vars() {
        let env = build_env(
            1,
            "feat",
            100,
            "hybrid",
            &IndexMap::new(),
            &[("postgres".into(), 5532), ("redis".into(), 6479)],
        );
        assert_eq!(env["ECLUSE_POSTGRES_PORT"], "5532");
        assert_eq!(env["ECLUSE_REDIS_PORT"], "6479");
        // No automatic DATABASE_URL — user sets that via hooks or .env
        assert!(!env.contains_key("DATABASE_URL"));
    }

    #[test]
    fn named_port_and_data_service_coexist() {
        let np = ports(&[("api", 3100)]);
        let env = build_env(
            1,
            "feat",
            100,
            "hybrid",
            &np,
            &[("postgres".into(), 5532)],
        );
        assert_eq!(env["PORT"], "3100");
        assert_eq!(env["ECLUSE_API_PORT"], "3100");
        assert_eq!(env["ECLUSE_POSTGRES_PORT"], "5532");
    }

    #[test]
    fn resolve_named_ports_computes_absolute_ports() {
        let mut cfg: IndexMap<String, u8> = IndexMap::new();
        cfg.insert("api".into(), 0);
        cfg.insert("worker".into(), 1);
        cfg.insert("frontend".into(), 2);
        let resolved = resolve_named_ports(&cfg, 3000, 100);
        assert_eq!(resolved["api"], 3100);
        assert_eq!(resolved["worker"], 3101);
        assert_eq!(resolved["frontend"], 3102);
    }

    #[test]
    fn service_env_key_uppercases_and_replaces_separators() {
        assert_eq!(service_env_key("my-service"), "MY_SERVICE");
        assert_eq!(service_env_key("my.service"), "MY_SERVICE");
        assert_eq!(service_env_key("redis"), "REDIS");
    }

    #[test]
    fn write_env_file_creates_sorted_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let np = ports(&[("api", 3100)]);
        let env = build_env(1, "feat", 100, "host", &np, &[]);
        write_env_file(dir.path(), &env).unwrap();
        let content = std::fs::read_to_string(dir.path().join(".env.ecluse")).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert!(lines.contains(&"ECLUSE_MODE=host"));
        assert!(lines.contains(&"ECLUSE_SLOT=1"));
        assert!(lines.contains(&"PORT=3100"));
        let mut sorted = lines.clone();
        sorted.sort();
        assert_eq!(lines, sorted);
    }

    #[test]
    fn effective_named_ports_empty_config_falls_back_to_app() {
        let cfg: IndexMap<String, u8> = IndexMap::new();
        let ports = super::effective_named_ports(&cfg, 3000, 100);
        assert_eq!(ports.len(), 1);
        assert_eq!(ports["app"], 3100);
    }

    #[test]
    fn effective_named_ports_with_config_uses_config() {
        let mut cfg: IndexMap<String, u8> = IndexMap::new();
        cfg.insert("api".into(), 0);
        cfg.insert("worker".into(), 1);
        let ports = super::effective_named_ports(&cfg, 3000, 200);
        assert_eq!(ports["api"], 3200);
        assert_eq!(ports["worker"], 3201);
        assert!(!ports.contains_key("app"));
    }

    #[test]
    fn resolve_named_ports_zero_offset() {
        let mut cfg: IndexMap<String, u8> = IndexMap::new();
        cfg.insert("api".into(), 0);
        let resolved = super::resolve_named_ports(&cfg, 3000, 0);
        assert_eq!(resolved["api"], 3000);
    }

    #[test]
    fn resolve_named_ports_index_stacks() {
        let mut cfg: IndexMap<String, u8> = IndexMap::new();
        cfg.insert("a".into(), 0);
        cfg.insert("b".into(), 5);
        let resolved = super::resolve_named_ports(&cfg, 3000, 100);
        assert_eq!(resolved["a"], 3100);
        assert_eq!(resolved["b"], 3105);
    }

    #[test]
    fn service_env_key_mixed_separators() {
        // Test via data_services path in build_env
        let env = build_env(
            1,
            "s",
            0,
            "hybrid",
            &IndexMap::new(),
            &[("my-db.local".into(), 5432)],
        );
        assert_eq!(env["ECLUSE_MY_DB_LOCAL_PORT"], "5432");
    }

    #[test]
    fn build_env_multiple_named_ports_port_alias_is_first() {
        let mut np: IndexMap<String, u16> = IndexMap::new();
        np.insert("api".into(), 3100);
        np.insert("worker".into(), 3101);
        np.insert("frontend".into(), 3102);
        let env = build_env(1, "s", 100, "host", &np, &[]);
        // PORT should be set to first entry only
        assert_eq!(env["PORT"], "3100");
    }

    #[test]
    fn build_env_slot_zero_and_empty_slug() {
        let env = build_env(0, "", 0, "host", &IndexMap::new(), &[]);
        assert_eq!(env["ECLUSE_SLOT"], "0");
        assert_eq!(env["ECLUSE_SLUG"], "");
        assert_eq!(env["ECLUSE_OFFSET"], "0");
    }

    #[test]
    fn write_env_file_overwrites_existing() {
        let dir = tempfile::TempDir::new().unwrap();
        // Write first
        let env1 = build_env(1, "a", 100, "host", &IndexMap::new(), &[]);
        write_env_file(dir.path(), &env1).unwrap();
        // Write second (different slug)
        let env2 = build_env(2, "b", 200, "container", &IndexMap::new(), &[]);
        write_env_file(dir.path(), &env2).unwrap();
        let content = std::fs::read_to_string(dir.path().join(".env.ecluse")).unwrap();
        assert!(content.contains("ECLUSE_SLUG=b"));
        assert!(!content.contains("ECLUSE_SLUG=a"));
    }

    #[test]
    fn write_env_file_ends_with_newline() {
        let dir = tempfile::TempDir::new().unwrap();
        let env = build_env(1, "x", 100, "host", &IndexMap::new(), &[]);
        write_env_file(dir.path(), &env).unwrap();
        let content = std::fs::read_to_string(dir.path().join(".env.ecluse")).unwrap();
        assert!(content.ends_with('\n'));
    }
}

pub fn write_env_file(worktree: &Path, env: &HashMap<String, String>) -> Result<()> {
    let mut lines: Vec<String> = env.iter().map(|(k, v)| format!("{}={}", k, v)).collect();
    lines.sort();
    let content = lines.join("\n") + "\n";
    std::fs::write(worktree.join(".env.ecluse"), content)
        .with_context(|| format!("failed to write .env.ecluse in {}", worktree.display()))
}
