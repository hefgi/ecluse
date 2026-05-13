use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::Path;

pub fn build_env(
    slot: u8,
    offset: u16,
    mode: &str,
    app_port: Option<u16>,
    database_name: Option<&str>,
    data_services: &[(String, u16)],
) -> HashMap<String, String> {
    let mut env = HashMap::new();
    env.insert("ECLUSE_SLOT".into(), slot.to_string());
    env.insert("ECLUSE_OFFSET".into(), offset.to_string());
    env.insert("ECLUSE_MODE".into(), mode.to_string());

    if let Some(port) = app_port {
        env.insert("PORT".into(), port.to_string());
        env.insert("ECLUSE_APP_PORT".into(), port.to_string());
    }

    if let Some(db) = database_name {
        env.insert(
            "DATABASE_URL".into(),
            format!("postgres://localhost/{}", db),
        );
        env.insert("ECLUSE_DATABASE".into(), db.to_string());
    }

    for (name, port) in data_services {
        let key = service_env_key(name);
        match name.as_str() {
            n if n.contains("postgres") || n.contains("pg") || n == "db" => {
                env.insert("DATABASE_URL".into(), format!("postgres://localhost:{}/{}", port, n));
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

pub fn write_env_file(worktree: &Path, env: &HashMap<String, String>) -> Result<()> {
    let mut lines: Vec<String> = env
        .iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect();
    lines.sort();
    let content = lines.join("\n") + "\n";
    std::fs::write(worktree.join(".env.ecluse"), content)
        .with_context(|| format!("failed to write .env.ecluse in {}", worktree.display()))
}
