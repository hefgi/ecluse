use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::Path;

pub fn build_env(
    slot: u8,
    slug: &str,
    offset: u16,
    mode: &str,
    app_port: Option<u16>,
    data_services: &[(String, u16)],
) -> HashMap<String, String> {
    let mut env = HashMap::new();
    env.insert("ECLUSE_SLOT".into(), slot.to_string());
    env.insert("ECLUSE_SLUG".into(), slug.to_string());
    env.insert("ECLUSE_OFFSET".into(), offset.to_string());
    env.insert("ECLUSE_MODE".into(), mode.to_string());

    if let Some(port) = app_port {
        env.insert("PORT".into(), port.to_string());
        env.insert("ECLUSE_APP_PORT".into(), port.to_string());
    }

    for (name, port) in data_services {
        let key = service_env_key(name);
        match name.as_str() {
            n if n.contains("postgres") || n.contains("pg") || n == "db" => {
                env.insert(
                    "DATABASE_URL".into(),
                    format!("postgres://localhost:{}/{}", port, n),
                );
            }
            n if n.contains("redis") => {
                env.insert("REDIS_URL".into(), format!("redis://localhost:{}", port));
            }
            _ => {}
        }
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

    #[test]
    fn basic_env_vars_always_present() {
        let env = build_env(2, "my-task", 200, "host", None, &[]);
        assert_eq!(env["ECLUSE_SLOT"], "2");
        assert_eq!(env["ECLUSE_SLUG"], "my-task");
        assert_eq!(env["ECLUSE_OFFSET"], "200");
        assert_eq!(env["ECLUSE_MODE"], "host");
    }

    #[test]
    fn app_port_sets_port_and_ecluse_app_port() {
        let env = build_env(1, "feat", 100, "host", Some(3100), &[]);
        assert_eq!(env["PORT"], "3100");
        assert_eq!(env["ECLUSE_APP_PORT"], "3100");
    }

    #[test]
    fn no_port_when_app_port_none() {
        let env = build_env(1, "feat", 100, "container", None, &[]);
        assert!(!env.contains_key("PORT"));
        assert!(!env.contains_key("ECLUSE_APP_PORT"));
    }

    #[test]
    fn data_service_postgres_sets_database_url() {
        let env = build_env(1, "feat", 100, "hybrid", None, &[("postgres".into(), 5532)]);
        assert_eq!(env["DATABASE_URL"], "postgres://localhost:5532/postgres");
        assert_eq!(env["ECLUSE_POSTGRES_PORT"], "5532");
    }

    #[test]
    fn data_service_redis_sets_redis_url() {
        let env = build_env(1, "feat", 100, "hybrid", None, &[("redis".into(), 6479)]);
        assert_eq!(env["REDIS_URL"], "redis://localhost:6479");
        assert_eq!(env["ECLUSE_REDIS_PORT"], "6479");
    }

    #[test]
    fn data_service_unknown_sets_port_only() {
        let env = build_env(1, "feat", 100, "hybrid", None, &[("mailhog".into(), 8125)]);
        assert_eq!(env["ECLUSE_MAILHOG_PORT"], "8125");
        assert!(!env.contains_key("DATABASE_URL"));
        assert!(!env.contains_key("REDIS_URL"));
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
        let env = build_env(1, "feat", 100, "host", Some(3100), &[]);
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
}

pub fn write_env_file(worktree: &Path, env: &HashMap<String, String>) -> Result<()> {
    let mut lines: Vec<String> = env.iter().map(|(k, v)| format!("{}={}", k, v)).collect();
    lines.sort();
    let content = lines.join("\n") + "\n";
    std::fs::write(worktree.join(".env.ecluse"), content)
        .with_context(|| format!("failed to write .env.ecluse in {}", worktree.display()))
}
