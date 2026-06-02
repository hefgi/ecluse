use anyhow::{Context, Result};
use indexmap::IndexMap;
use std::collections::HashMap;
use std::path::Path;

use crate::config::ServiceConfig;

/// Build the env map for a session.
///
/// `native_ports` maps service name → host port for native services.
/// The first entry also sets `PORT` (primary service alias for framework compatibility).
/// `docker_ports` maps service name → host port for docker services (ECLUSE_<NAME>_PORT only).
/// `native_configs` is used to apply `port_env` aliases declared on each native service.
///
/// `ECLUSE_SLOT`, `ECLUSE_SLUG`, `ECLUSE_MODE` are always present.
pub fn build_env(
    slot: u8,
    slug: &str,
    mode: &str,
    native_ports: &IndexMap<String, u16>,
    docker_ports: &[(String, u16)],
    native_configs: &[&ServiceConfig],
) -> HashMap<String, String> {
    let mut env = HashMap::new();
    env.insert("ECLUSE_SLOT".into(), slot.to_string());
    env.insert("ECLUSE_SLUG".into(), slug.to_string());
    env.insert("ECLUSE_MODE".into(), mode.to_string());

    // Native ports: generate ECLUSE_<NAME>_PORT + PORT alias for first
    let mut first = true;
    for (name, port) in native_ports {
        let key = service_env_key(name);
        env.insert(format!("ECLUSE_{}_PORT", key), port.to_string());
        if first {
            // Primary service → PORT alias for framework compatibility
            env.insert("PORT".into(), port.to_string());
            first = false;
        }

        // port_env aliases: set each declared var name to this service's port
        if let Some(svc) = native_configs.iter().find(|s| s.name == *name) {
            for alias in &svc.port_env {
                env.insert(alias.clone(), port.to_string());
            }

            // debug_port: emit ECLUSE_<NAME>_DEBUG_PORT = debug_port + slot
            if let Some(debug_port) = svc.debug_port_for_slot(slot) {
                env.insert(format!("ECLUSE_{}_DEBUG_PORT", key), debug_port.to_string());
            }
        }
    }

    // Docker ports: ECLUSE_<NAME>_PORT only
    for (name, port) in docker_ports {
        let key = service_env_key(name);
        env.insert(format!("ECLUSE_{}_PORT", key), port.to_string());
    }

    env
}

fn service_env_key(name: &str) -> String {
    name.to_uppercase().replace(['-', '.'], "_")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ports(entries: &[(&str, u16)]) -> IndexMap<String, u16> {
        entries.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    #[test]
    fn basic_env_vars_always_present() {
        let env = build_env(2, "my-task", "host", &IndexMap::new(), &[], &[]);
        assert_eq!(env["ECLUSE_SLOT"], "2");
        assert_eq!(env["ECLUSE_SLUG"], "my-task");
        assert_eq!(env["ECLUSE_MODE"], "host");
        assert!(!env.contains_key("ECLUSE_OFFSET"));
    }

    #[test]
    fn first_native_port_sets_port_alias() {
        let np = ports(&[("api", 8001), ("worker", 8002)]);
        let env = build_env(1, "feat", "host", &np, &[], &[]);
        assert_eq!(env["PORT"], "8001");
        assert_eq!(env["ECLUSE_API_PORT"], "8001");
        assert_eq!(env["ECLUSE_WORKER_PORT"], "8002");
    }

    #[test]
    fn no_port_when_no_native_ports() {
        let env = build_env(1, "feat", "container", &IndexMap::new(), &[], &[]);
        assert!(!env.contains_key("PORT"));
    }

    #[test]
    fn docker_ports_set_port_vars_no_port_alias() {
        let env = build_env(
            1,
            "feat",
            "hybrid",
            &IndexMap::new(),
            &[("postgres".into(), 5433), ("redis".into(), 6380)],
            &[],
        );
        assert_eq!(env["ECLUSE_POSTGRES_PORT"], "5433");
        assert_eq!(env["ECLUSE_REDIS_PORT"], "6380");
        assert!(!env.contains_key("PORT"));
    }

    #[test]
    fn native_and_docker_ports_coexist() {
        let np = ports(&[("api", 8001)]);
        let env = build_env(1, "feat", "hybrid", &np, &[("postgres".into(), 5433)], &[]);
        assert_eq!(env["PORT"], "8001");
        assert_eq!(env["ECLUSE_API_PORT"], "8001");
        assert_eq!(env["ECLUSE_POSTGRES_PORT"], "5433");
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
        let np = ports(&[("api", 8001)]);
        let env = build_env(1, "feat", "host", &np, &[], &[]);
        write_env_file(dir.path(), &env).unwrap();
        let content = std::fs::read_to_string(dir.path().join(".env.ecluse")).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert!(lines.contains(&"ECLUSE_MODE=host"));
        assert!(lines.contains(&"ECLUSE_SLOT=1"));
        assert!(lines.contains(&"PORT=8001"));
        let mut sorted = lines.clone();
        sorted.sort();
        assert_eq!(lines, sorted);
    }

    #[test]
    fn fallback_app_service_port() {
        // Simulate fallback: single "app" service at 3000 + slot
        let np = ports(&[("app", 3001)]);
        let env = build_env(1, "feat", "host", &np, &[], &[]);
        assert_eq!(env["PORT"], "3001");
        assert_eq!(env["ECLUSE_APP_PORT"], "3001");
    }

    #[test]
    fn build_env_multiple_native_ports_port_alias_is_first() {
        let mut np: IndexMap<String, u16> = IndexMap::new();
        np.insert("api".into(), 8001);
        np.insert("worker".into(), 8002);
        np.insert("frontend".into(), 3001);
        let env = build_env(1, "s", "host", &np, &[], &[]);
        assert_eq!(env["PORT"], "8001");
    }

    #[test]
    fn build_env_slot_zero_and_empty_slug() {
        let env = build_env(0, "", "host", &IndexMap::new(), &[], &[]);
        assert_eq!(env["ECLUSE_SLOT"], "0");
        assert_eq!(env["ECLUSE_SLUG"], "");
    }

    #[test]
    fn write_env_file_overwrites_existing() {
        let dir = tempfile::TempDir::new().unwrap();
        let env1 = build_env(1, "a", "host", &IndexMap::new(), &[], &[]);
        write_env_file(dir.path(), &env1).unwrap();
        let env2 = build_env(2, "b", "container", &IndexMap::new(), &[], &[]);
        write_env_file(dir.path(), &env2).unwrap();
        let content = std::fs::read_to_string(dir.path().join(".env.ecluse")).unwrap();
        assert!(content.contains("ECLUSE_SLUG=b"));
        assert!(!content.contains("ECLUSE_SLUG=a"));
    }

    #[test]
    fn write_env_file_ends_with_newline() {
        let dir = tempfile::TempDir::new().unwrap();
        let env = build_env(1, "x", "host", &IndexMap::new(), &[], &[]);
        write_env_file(dir.path(), &env).unwrap();
        let content = std::fs::read_to_string(dir.path().join(".env.ecluse")).unwrap();
        assert!(content.ends_with('\n'));
    }

    #[test]
    fn service_env_key_mixed_separators() {
        let env = build_env(
            1,
            "s",
            "hybrid",
            &IndexMap::new(),
            &[("my-db.local".into(), 5432)],
            &[],
        );
        assert_eq!(env["ECLUSE_MY_DB_LOCAL_PORT"], "5432");
    }

    #[test]
    fn no_offset_env_var() {
        let env = build_env(1, "feat", "host", &IndexMap::new(), &[], &[]);
        assert!(!env.contains_key("ECLUSE_OFFSET"));
    }

    #[test]
    fn port_env_single_alias_is_set() {
        use crate::config::{ServiceConfig, ServiceRun};
        let svc = ServiceConfig {
            name: "api".into(),
            base_port: 3000,
            run: ServiceRun::Native,
            compose: None,
            command: None,
            port_env: vec!["DJANGO_PORT".into()],
            debug_port: None,
            extra_ports: vec![],
        };
        let np = ports(&[("api", 3001)]);
        let env = build_env(1, "s", "host", &np, &[], &[&svc]);
        assert_eq!(env["DJANGO_PORT"], "3001");
        assert_eq!(env["ECLUSE_API_PORT"], "3001");
    }

    #[test]
    fn port_env_multiple_aliases_all_set() {
        use crate::config::{ServiceConfig, ServiceRun};
        let svc = ServiceConfig {
            name: "api".into(),
            base_port: 3000,
            run: ServiceRun::Native,
            compose: None,
            command: None,
            port_env: vec!["DJANGO_PORT".into(), "APP_PORT".into()],
            debug_port: None,
            extra_ports: vec![],
        };
        let np = ports(&[("api", 3001)]);
        let env = build_env(1, "s", "host", &np, &[], &[&svc]);
        assert_eq!(env["DJANGO_PORT"], "3001");
        assert_eq!(env["APP_PORT"], "3001");
    }

    #[test]
    fn port_env_empty_vec_injects_nothing_extra() {
        use crate::config::{ServiceConfig, ServiceRun};
        let svc = ServiceConfig {
            name: "api".into(),
            base_port: 3000,
            run: ServiceRun::Native,
            compose: None,
            command: None,
            port_env: vec![],
            debug_port: None,
            extra_ports: vec![],
        };
        let np = ports(&[("api", 3001)]);
        let env = build_env(1, "s", "host", &np, &[], &[&svc]);
        assert_eq!(env.get("ECLUSE_API_PORT").map(|s| s.as_str()), Some("3001"));
        assert!(!env.contains_key("DJANGO_PORT"));
    }

    #[test]
    fn per_service_base_port_slot_arithmetic() {
        // api base_port=8000, slot=1 → 8001; slot=2 → 8002
        let np1 = ports(&[("api", 8001)]);
        let env1 = build_env(1, "s1", "host", &np1, &[], &[]);
        assert_eq!(env1["ECLUSE_API_PORT"], "8001");

        let np2 = ports(&[("api", 8002)]);
        let env2 = build_env(2, "s2", "host", &np2, &[], &[]);
        assert_eq!(env2["ECLUSE_API_PORT"], "8002");
    }

    #[test]
    fn debug_port_emits_inspect_env_var() {
        use crate::config::{ServiceConfig, ServiceRun};
        let svc = ServiceConfig {
            name: "app".into(),
            base_port: 7100,
            run: ServiceRun::Native,
            compose: None,
            command: None,
            port_env: vec![],
            debug_port: Some(9229),
        };
        let np = ports(&[("app", 7101)]);
        let env = build_env(1, "s", "host", &np, &[], &[&svc]);
        assert_eq!(env["ECLUSE_APP_DEBUG_PORT"], "9230");
    }

    #[test]
    fn debug_port_slot_arithmetic() {
        use crate::config::{ServiceConfig, ServiceRun};
        let svc = ServiceConfig {
            name: "app".into(),
            base_port: 7100,
            run: ServiceRun::Native,
            compose: None,
            command: None,
            port_env: vec![],
            debug_port: Some(9229),
        };
        let np2 = ports(&[("app", 7102)]);
        let env2 = build_env(2, "s2", "host", &np2, &[], &[&svc]);
        assert_eq!(env2["ECLUSE_APP_DEBUG_PORT"], "9231");
    }

    #[test]
    fn no_debug_port_means_no_inspect_env_var() {
        use crate::config::{ServiceConfig, ServiceRun};
        let svc = ServiceConfig {
            name: "api".into(),
            base_port: 4444,
            run: ServiceRun::Native,
            compose: None,
            command: None,
            port_env: vec![],
            debug_port: None,
            extra_ports: vec![],
        };
        let np = ports(&[("api", 4445)]);
        let env = build_env(1, "s", "host", &np, &[], &[&svc]);
        assert!(!env.contains_key("ECLUSE_API_DEBUG_PORT"));
    }
}

pub fn write_env_file(worktree: &Path, env: &HashMap<String, String>) -> Result<()> {
    let mut lines: Vec<String> = env.iter().map(|(k, v)| format!("{}={}", k, v)).collect();
    lines.sort();
    let content = lines.join("\n") + "\n";
    std::fs::write(worktree.join(".env.ecluse"), content)
        .with_context(|| format!("failed to write .env.ecluse in {}", worktree.display()))
}
