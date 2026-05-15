use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ComposeFile {
    #[serde(default)]
    pub services: HashMap<String, Service>,
    #[serde(default)]
    pub volumes: HashMap<String, serde_yaml::Value>,
    #[serde(default)]
    pub networks: HashMap<String, serde_yaml::Value>,
    #[serde(flatten)]
    pub other: HashMap<String, serde_yaml::Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Service {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub build: Option<serde_yaml::Value>,
    #[serde(default)]
    pub ports: Vec<serde_yaml::Value>,
    #[serde(default)]
    pub volumes: Vec<serde_yaml::Value>,
    #[serde(default)]
    pub labels: serde_yaml::Value,
    #[serde(flatten)]
    pub other: HashMap<String, serde_yaml::Value>,
}

pub fn parse(path: &Path) -> Result<ComposeFile> {
    let content = std::fs::read_to_string(path).with_context(|| {
        crate::error::EcluseError::ComposeFileNotFound(path.display().to_string())
    })?;
    serde_yaml::from_str(&content)
        .with_context(|| crate::error::EcluseError::ComposeParseFailed(path.display().to_string()))
}

/// Returns service names labeled ecluse.role=app
pub fn app_services(compose: &ComposeFile, label_key: &str, label_value: &str) -> Vec<String> {
    compose
        .services
        .iter()
        .filter_map(|(name, svc)| {
            if has_label(&svc.labels, label_key, label_value) {
                Some(name.clone())
            } else {
                None
            }
        })
        .collect()
}

/// Returns service names NOT labeled as app
pub fn data_services(compose: &ComposeFile, label_key: &str, label_value: &str) -> Vec<String> {
    compose
        .services
        .keys()
        .filter(|name| {
            let svc = &compose.services[*name];
            !has_label(&svc.labels, label_key, label_value)
        })
        .cloned()
        .collect()
}

fn has_label(labels: &serde_yaml::Value, key: &str, value: &str) -> bool {
    match labels {
        serde_yaml::Value::Mapping(map) => {
            for (k, v) in map {
                let k_str = k.as_str().unwrap_or("");
                let v_str = v.as_str().unwrap_or("");
                if k_str == key && v_str == value {
                    return true;
                }
            }
            false
        }
        serde_yaml::Value::Sequence(seq) => {
            for item in seq {
                if let Some(s) = item.as_str() {
                    if s == format!("{}={}", key, value) {
                        return true;
                    }
                }
            }
            false
        }
        _ => false,
    }
}

/// Generate a compose overlay that offsets all host ports by `offset`
/// and namespaces named volumes with `suffix`.
pub fn generate_overlay(
    compose: &ComposeFile,
    offset: u16,
    suffix: &str,
    services_to_include: Option<&[String]>,
) -> Result<String> {
    let mut overlay_services: HashMap<String, serde_yaml::Value> = HashMap::new();
    let mut overlay_volumes: HashMap<String, serde_yaml::Value> = HashMap::new();

    for (name, svc) in &compose.services {
        if let Some(inc) = services_to_include {
            if !inc.contains(name) {
                continue;
            }
        }

        let mut svc_override: HashMap<String, serde_yaml::Value> = HashMap::new();

        // Rewrite ports
        if !svc.ports.is_empty() {
            let new_ports: Vec<serde_yaml::Value> =
                svc.ports.iter().map(|p| rewrite_port(p, offset)).collect();
            svc_override.insert("ports".into(), serde_yaml::Value::Sequence(new_ports));
        }

        // Namespace volumes
        if !svc.volumes.is_empty() {
            let new_vols: Vec<serde_yaml::Value> = svc
                .volumes
                .iter()
                .map(|v| namespace_volume(v, suffix))
                .collect();
            svc_override.insert("volumes".into(), serde_yaml::Value::Sequence(new_vols));
        }

        if !svc_override.is_empty() {
            let mut map = serde_yaml::Mapping::new();
            for (k, v) in svc_override {
                map.insert(serde_yaml::Value::String(k), v);
            }
            overlay_services.insert(name.clone(), serde_yaml::Value::Mapping(map));
        }
    }

    // Declare top-level volumes
    for vol_name in compose.volumes.keys() {
        let new_name = format!("{}_{}", vol_name, suffix);
        overlay_volumes.insert(new_name, serde_yaml::Value::Null);
    }

    let mut root = serde_yaml::Mapping::new();

    if !overlay_services.is_empty() {
        let mut svc_map = serde_yaml::Mapping::new();
        for (k, v) in overlay_services {
            svc_map.insert(serde_yaml::Value::String(k), v);
        }
        root.insert(
            serde_yaml::Value::String("services".into()),
            serde_yaml::Value::Mapping(svc_map),
        );
    }

    if !overlay_volumes.is_empty() {
        let mut vol_map = serde_yaml::Mapping::new();
        for (k, v) in overlay_volumes {
            vol_map.insert(serde_yaml::Value::String(k), v);
        }
        root.insert(
            serde_yaml::Value::String("volumes".into()),
            serde_yaml::Value::Mapping(vol_map),
        );
    }

    serde_yaml::to_string(&serde_yaml::Value::Mapping(root))
        .context("failed to serialize overlay YAML")
}

/// Generate a compose overlay that sets explicit host ports from a map of
/// service_name → host_port, and namespaces named volumes with `suffix`.
/// This is used in the per-service base_port model.
pub fn generate_overlay_with_ports(
    compose: &ComposeFile,
    port_overrides: &HashMap<String, u16>,
    suffix: &str,
    services_to_include: Option<&[String]>,
) -> Result<String> {
    let mut overlay_services: HashMap<String, serde_yaml::Value> = HashMap::new();
    let mut overlay_volumes: HashMap<String, serde_yaml::Value> = HashMap::new();

    for (name, svc) in &compose.services {
        if let Some(inc) = services_to_include {
            if !inc.contains(name) {
                continue;
            }
        }

        let mut svc_override: HashMap<String, serde_yaml::Value> = HashMap::new();

        // Rewrite ports using explicit host port if provided, otherwise keep as-is
        if !svc.ports.is_empty() {
            let host_port = port_overrides.get(name).copied();
            let new_ports: Vec<serde_yaml::Value> = svc
                .ports
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    if i == 0 {
                        if let Some(hp) = host_port {
                            return rewrite_port_to_host(p, hp);
                        }
                    }
                    p.clone()
                })
                .collect();
            svc_override.insert("ports".into(), serde_yaml::Value::Sequence(new_ports));
        }

        // Namespace volumes
        if !svc.volumes.is_empty() {
            let new_vols: Vec<serde_yaml::Value> = svc
                .volumes
                .iter()
                .map(|v| namespace_volume(v, suffix))
                .collect();
            svc_override.insert("volumes".into(), serde_yaml::Value::Sequence(new_vols));
        }

        if !svc_override.is_empty() {
            let mut map = serde_yaml::Mapping::new();
            for (k, v) in svc_override {
                map.insert(serde_yaml::Value::String(k), v);
            }
            overlay_services.insert(name.clone(), serde_yaml::Value::Mapping(map));
        }
    }

    // Declare top-level volumes
    for vol_name in compose.volumes.keys() {
        let new_name = format!("{}_{}", vol_name, suffix);
        overlay_volumes.insert(new_name, serde_yaml::Value::Null);
    }

    let mut root = serde_yaml::Mapping::new();

    if !overlay_services.is_empty() {
        let mut svc_map = serde_yaml::Mapping::new();
        for (k, v) in overlay_services {
            svc_map.insert(serde_yaml::Value::String(k), v);
        }
        root.insert(
            serde_yaml::Value::String("services".into()),
            serde_yaml::Value::Mapping(svc_map),
        );
    }

    if !overlay_volumes.is_empty() {
        let mut vol_map = serde_yaml::Mapping::new();
        for (k, v) in overlay_volumes {
            vol_map.insert(serde_yaml::Value::String(k), v);
        }
        root.insert(
            serde_yaml::Value::String("volumes".into()),
            serde_yaml::Value::Mapping(vol_map),
        );
    }

    serde_yaml::to_string(&serde_yaml::Value::Mapping(root))
        .context("failed to serialize overlay YAML")
}

/// Rewrite the first port mapping to use an explicit host port,
/// preserving the container port.
fn rewrite_port_to_host(port: &serde_yaml::Value, host_port: u16) -> serde_yaml::Value {
    match port {
        serde_yaml::Value::String(s) => {
            let parts: Vec<&str> = s.split(':').collect();
            let container_port = match parts.len() {
                1 => parts[0].to_string(),
                2 => parts[1].to_string(),
                3 => parts[2].to_string(),
                _ => return port.clone(),
            };
            serde_yaml::Value::String(format!("{}:{}", host_port, container_port))
        }
        serde_yaml::Value::Number(n) => {
            if let Some(p) = n.as_u64() {
                serde_yaml::Value::String(format!("{}:{}", host_port, p))
            } else {
                port.clone()
            }
        }
        serde_yaml::Value::Mapping(map) => {
            let mut new_map = map.clone();
            if map.contains_key("published") {
                new_map.insert(
                    serde_yaml::Value::String("published".into()),
                    serde_yaml::Value::Number(host_port.into()),
                );
            }
            serde_yaml::Value::Mapping(new_map)
        }
        _ => port.clone(),
    }
}

fn rewrite_port(port: &serde_yaml::Value, offset: u16) -> serde_yaml::Value {
    match port {
        serde_yaml::Value::String(s) => {
            // "3000:3000" or "3000" or "0.0.0.0:3000:3000"
            let new_s = rewrite_port_str(s, offset);
            serde_yaml::Value::String(new_s)
        }
        serde_yaml::Value::Number(n) => {
            if let Some(p) = n.as_u64() {
                let new_p = p as u16 + offset;
                serde_yaml::Value::String(format!("{}:{}", new_p, p))
            } else {
                port.clone()
            }
        }
        serde_yaml::Value::Mapping(map) => {
            let mut new_map = map.clone();
            if let Some(published) = map.get("published") {
                if let Some(p) = published.as_u64() {
                    new_map.insert(
                        serde_yaml::Value::String("published".into()),
                        serde_yaml::Value::Number((p as u16 + offset).into()),
                    );
                }
            }
            serde_yaml::Value::Mapping(new_map)
        }
        _ => port.clone(),
    }
}

fn rewrite_port_str(s: &str, offset: u16) -> String {
    let parts: Vec<&str> = s.split(':').collect();
    match parts.len() {
        1 => {
            // container port only — add host mapping with offset
            if let Ok(p) = parts[0].parse::<u16>() {
                format!("{}:{}", p + offset, p)
            } else {
                s.to_string()
            }
        }
        2 => {
            // host:container
            if let Ok(hp) = parts[0].parse::<u16>() {
                format!("{}:{}", hp + offset, parts[1])
            } else {
                s.to_string()
            }
        }
        3 => {
            // ip:host:container
            if let Ok(hp) = parts[1].parse::<u16>() {
                format!("{}:{}:{}", parts[0], hp + offset, parts[2])
            } else {
                s.to_string()
            }
        }
        _ => s.to_string(),
    }
}

fn namespace_volume(vol: &serde_yaml::Value, suffix: &str) -> serde_yaml::Value {
    match vol {
        serde_yaml::Value::String(s) => {
            // "vol_name:/path" or "./bind:/path"
            let parts: Vec<&str> = s.splitn(2, ':').collect();
            if parts.len() == 2 {
                let src = parts[0];
                // Only namespace named volumes (not bind mounts starting with . or /)
                if !src.starts_with('.') && !src.starts_with('/') {
                    return serde_yaml::Value::String(format!("{}_{}:{}", src, suffix, parts[1]));
                }
            }
            vol.clone()
        }
        serde_yaml::Value::Mapping(map) => {
            let mut new_map = map.clone();
            if let Some(src) = map.get("source").and_then(|v| v.as_str()) {
                let vtype = map.get("type").and_then(|v| v.as_str()).unwrap_or("volume");
                if vtype == "volume" {
                    new_map.insert(
                        serde_yaml::Value::String("source".into()),
                        serde_yaml::Value::String(format!("{}_{}", src, suffix)),
                    );
                }
            }
            serde_yaml::Value::Mapping(new_map)
        }
        _ => vol.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tempfile::TempDir;

    fn null_svc(ports: &[&str], volumes: &[&str]) -> Service {
        Service {
            image: None,
            build: None,
            ports: ports
                .iter()
                .map(|p| serde_yaml::Value::String(p.to_string()))
                .collect(),
            volumes: volumes
                .iter()
                .map(|v| serde_yaml::Value::String(v.to_string()))
                .collect(),
            labels: serde_yaml::Value::Null,
            other: HashMap::new(),
        }
    }

    fn mapping_labels(pairs: &[(&str, &str)]) -> serde_yaml::Value {
        let mut map = serde_yaml::Mapping::new();
        for (k, v) in pairs {
            map.insert(
                serde_yaml::Value::String(k.to_string()),
                serde_yaml::Value::String(v.to_string()),
            );
        }
        serde_yaml::Value::Mapping(map)
    }

    fn sequence_labels(items: &[&str]) -> serde_yaml::Value {
        serde_yaml::Value::Sequence(
            items
                .iter()
                .map(|s| serde_yaml::Value::String(s.to_string()))
                .collect(),
        )
    }

    fn make_compose(services: Vec<(&str, Service)>) -> ComposeFile {
        ComposeFile {
            services: services
                .into_iter()
                .map(|(n, s)| (n.to_string(), s))
                .collect(),
            volumes: HashMap::new(),
            networks: HashMap::new(),
            other: HashMap::new(),
        }
    }

    // ── parse ─────────────────────────────────────────────────────────────────

    #[test]
    fn parse_valid_compose_file() {
        let dir = TempDir::new().unwrap();
        let yaml = "services:\n  app:\n    build: .\n    ports:\n      - \"3000:3000\"\n  db:\n    image: postgres:15\nvolumes:\n  db_data:\n";
        std::fs::write(dir.path().join("docker-compose.yml"), yaml).unwrap();
        let cf = parse(&dir.path().join("docker-compose.yml")).unwrap();
        assert!(cf.services.contains_key("app"));
        assert!(cf.services.contains_key("db"));
        assert!(cf.volumes.contains_key("db_data"));
    }

    #[test]
    fn parse_missing_file_errors() {
        let dir = TempDir::new().unwrap();
        let err = parse(&dir.path().join("docker-compose.yml")).unwrap_err();
        assert!(
            err.to_string().contains("not found") || err.to_string().contains("docker-compose.yml")
        );
    }

    #[test]
    fn parse_invalid_yaml_errors() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("docker-compose.yml"), "{ bad: [yaml:").unwrap();
        let err = parse(&dir.path().join("docker-compose.yml")).unwrap_err();
        assert!(err.to_string().contains("parse") || err.to_string().contains("compose"));
    }

    // ── resolve_service_compose ───────────────────────────────────────────────

    fn docker_svc(compose: Option<&str>) -> crate::config::ServiceConfig {
        crate::config::ServiceConfig {
            name: "db".into(),
            base_port: 5432,
            run: crate::config::ServiceRun::Docker,
            compose: compose.map(|s| s.to_string()),
            command: None,
            port_env: vec![],
        }
    }

    #[test]
    fn resolve_service_compose_no_field_returns_root_compose() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("docker-compose.yml"), "").unwrap();
        let svc = docker_svc(None);
        let result = super::resolve_service_compose(dir.path(), &svc);
        assert!(result.is_some());
        assert!(result.unwrap().ends_with("docker-compose.yml"));
    }

    #[test]
    fn resolve_service_compose_no_field_no_root_returns_none() {
        let dir = TempDir::new().unwrap();
        let svc = docker_svc(None);
        assert!(super::resolve_service_compose(dir.path(), &svc).is_none());
    }

    #[test]
    fn resolve_service_compose_explicit_existing_path_returned() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir(dir.path().join("infra")).unwrap();
        std::fs::write(dir.path().join("infra/docker-compose.yml"), "").unwrap();
        let svc = docker_svc(Some("infra/docker-compose.yml"));
        let result = super::resolve_service_compose(dir.path(), &svc);
        assert!(result.is_some());
        assert!(result.unwrap().ends_with("infra/docker-compose.yml"));
    }

    #[test]
    fn resolve_service_compose_explicit_missing_path_returns_none() {
        let dir = TempDir::new().unwrap();
        let svc = docker_svc(Some("nonexistent/docker-compose.yml"));
        assert!(super::resolve_service_compose(dir.path(), &svc).is_none());
    }

    // ── find_compose_file ─────────────────────────────────────────────────────

    #[test]
    fn find_compose_docker_compose_yml() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("docker-compose.yml"), "").unwrap();
        let found = find_compose_file(dir.path()).unwrap();
        assert!(found.to_string_lossy().contains("docker-compose.yml"));
    }

    #[test]
    fn find_compose_compose_yaml() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("compose.yaml"), "").unwrap();
        assert!(find_compose_file(dir.path()).is_some());
    }

    #[test]
    fn find_compose_compose_yml() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("compose.yml"), "").unwrap();
        assert!(find_compose_file(dir.path()).is_some());
    }

    #[test]
    fn find_compose_docker_compose_yaml() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("docker-compose.yaml"), "").unwrap();
        assert!(find_compose_file(dir.path()).is_some());
    }

    #[test]
    fn find_compose_none_when_absent() {
        let dir = TempDir::new().unwrap();
        assert!(find_compose_file(dir.path()).is_none());
    }

    // ── app_services / data_services ──────────────────────────────────────────

    #[test]
    fn app_services_mapping_labels_detected() {
        let cf = make_compose(vec![
            (
                "web",
                Service {
                    labels: mapping_labels(&[("ecluse.role", "app")]),
                    ..null_svc(&[], &[])
                },
            ),
            ("db", null_svc(&[], &[])),
        ]);
        let apps = app_services(&cf, "ecluse.role", "app");
        assert_eq!(apps, vec!["web"]);
    }

    #[test]
    fn app_services_sequence_labels_detected() {
        let cf = make_compose(vec![
            (
                "web",
                Service {
                    labels: sequence_labels(&["ecluse.role=app", "other=val"]),
                    ..null_svc(&[], &[])
                },
            ),
            ("db", null_svc(&[], &[])),
        ]);
        let apps = app_services(&cf, "ecluse.role", "app");
        assert_eq!(apps, vec!["web"]);
    }

    #[test]
    fn app_services_no_match_returns_empty() {
        let cf = make_compose(vec![("db", null_svc(&[], &[]))]);
        assert!(app_services(&cf, "ecluse.role", "app").is_empty());
    }

    #[test]
    fn app_services_null_labels_not_matched() {
        let cf = make_compose(vec![("web", null_svc(&[], &[]))]);
        assert!(app_services(&cf, "ecluse.role", "app").is_empty());
    }

    #[test]
    fn app_services_wrong_value_not_matched() {
        let cf = make_compose(vec![(
            "web",
            Service {
                labels: mapping_labels(&[("ecluse.role", "worker")]),
                ..null_svc(&[], &[])
            },
        )]);
        assert!(app_services(&cf, "ecluse.role", "app").is_empty());
    }

    #[test]
    fn data_services_excludes_app_labeled() {
        let cf = make_compose(vec![
            (
                "web",
                Service {
                    labels: mapping_labels(&[("ecluse.role", "app")]),
                    ..null_svc(&[], &[])
                },
            ),
            ("db", null_svc(&[], &[])),
            ("redis", null_svc(&[], &[])),
        ]);
        let data = data_services(&cf, "ecluse.role", "app");
        assert!(!data.contains(&"web".to_string()));
        assert!(data.contains(&"db".to_string()));
        assert!(data.contains(&"redis".to_string()));
    }

    #[test]
    fn data_services_all_when_no_app_label() {
        let cf = make_compose(vec![
            ("db", null_svc(&[], &[])),
            ("redis", null_svc(&[], &[])),
        ]);
        assert_eq!(data_services(&cf, "ecluse.role", "app").len(), 2);
    }

    // ── service_host_port ─────────────────────────────────────────────────────

    #[test]
    fn service_host_port_host_container_string() {
        let s = null_svc(&["3000:3000"], &[]);
        assert_eq!(service_host_port(&s, 100), Some(3100));
    }

    #[test]
    fn service_host_port_container_only_string() {
        let s = null_svc(&["3000"], &[]);
        assert_eq!(service_host_port(&s, 100), Some(3100));
    }

    #[test]
    fn service_host_port_ip_host_container_string() {
        let s = null_svc(&["0.0.0.0:5432:5432"], &[]);
        assert_eq!(service_host_port(&s, 100), Some(5532));
    }

    #[test]
    fn service_host_port_number_value() {
        let s = Service {
            ports: vec![serde_yaml::Value::Number(serde_yaml::Number::from(8080u64))],
            ..null_svc(&[], &[])
        };
        assert_eq!(service_host_port(&s, 100), Some(8180));
    }

    #[test]
    fn service_host_port_zero_offset() {
        let s = null_svc(&["5000:5000"], &[]);
        assert_eq!(service_host_port(&s, 0), Some(5000));
    }

    #[test]
    fn service_host_port_no_ports_none() {
        let s = null_svc(&[], &[]);
        assert_eq!(service_host_port(&s, 100), None);
    }

    // ── rewrite_port_str (via overlay) ────────────────────────────────────────

    #[test]
    fn overlay_port_string_host_container_rewritten() {
        let cf = make_compose(vec![("app", null_svc(&["3000:3000"], &[]))]);
        let yaml = generate_overlay(&cf, 100, "slug", None).unwrap();
        assert!(yaml.contains("3100:3000"), "got: {}", yaml);
    }

    #[test]
    fn overlay_port_container_only_rewritten_with_host() {
        let cf = make_compose(vec![("app", null_svc(&["8080"], &[]))]);
        let yaml = generate_overlay(&cf, 100, "slug", None).unwrap();
        assert!(yaml.contains("8180:8080"), "got: {}", yaml);
    }

    #[test]
    fn overlay_port_ip_host_container_rewritten() {
        let cf = make_compose(vec![("app", null_svc(&["0.0.0.0:3000:3000"], &[]))]);
        let yaml = generate_overlay(&cf, 100, "slug", None).unwrap();
        assert!(yaml.contains("3100"), "got: {}", yaml);
    }

    #[test]
    fn overlay_port_numeric_rewritten() {
        let s = Service {
            ports: vec![serde_yaml::Value::Number(serde_yaml::Number::from(3000u64))],
            ..null_svc(&[], &[])
        };
        let cf = make_compose(vec![("app", s)]);
        let yaml = generate_overlay(&cf, 100, "slug", None).unwrap();
        assert!(yaml.contains("3100"), "got: {}", yaml);
    }

    #[test]
    fn overlay_port_mapping_published_rewritten() {
        let mut pm = serde_yaml::Mapping::new();
        pm.insert(
            serde_yaml::Value::String("published".into()),
            serde_yaml::Value::Number(3000u64.into()),
        );
        pm.insert(
            serde_yaml::Value::String("target".into()),
            serde_yaml::Value::Number(3000u64.into()),
        );
        let s = Service {
            ports: vec![serde_yaml::Value::Mapping(pm)],
            ..null_svc(&[], &[])
        };
        let cf = make_compose(vec![("app", s)]);
        let yaml = generate_overlay(&cf, 100, "slug", None).unwrap();
        assert!(yaml.contains("3100"), "got: {}", yaml);
    }

    #[test]
    fn overlay_port_mapping_without_published_unchanged() {
        let mut pm = serde_yaml::Mapping::new();
        pm.insert(
            serde_yaml::Value::String("target".into()),
            serde_yaml::Value::Number(3000u64.into()),
        );
        let s = Service {
            ports: vec![serde_yaml::Value::Mapping(pm)],
            ..null_svc(&[], &[])
        };
        let cf = make_compose(vec![("app", s)]);
        // Should not crash even without "published" key
        let yaml = generate_overlay(&cf, 100, "slug", None).unwrap();
        assert!(!yaml.is_empty());
    }

    #[test]
    fn overlay_port_non_numeric_host_passthrough() {
        // "named-port:3000" — host is not a number, should pass through unchanged
        let cf = make_compose(vec![("app", null_svc(&["named-port:3000"], &[]))]);
        let yaml = generate_overlay(&cf, 100, "slug", None).unwrap();
        assert!(yaml.contains("named-port:3000"), "got: {}", yaml);
    }

    // ── volume namespacing (via overlay) ──────────────────────────────────────

    #[test]
    fn overlay_named_volume_namespaced() {
        let cf = make_compose(vec![(
            "db",
            null_svc(&[], &["db_data:/var/lib/postgresql/data"]),
        )]);
        let yaml = generate_overlay(&cf, 0, "myslug", None).unwrap();
        assert!(yaml.contains("db_data_myslug"), "got: {}", yaml);
    }

    #[test]
    fn overlay_bind_mount_dot_not_namespaced() {
        let cf = make_compose(vec![("app", null_svc(&[], &["./src:/app/src"]))]);
        let yaml = generate_overlay(&cf, 0, "myslug", None).unwrap();
        assert!(
            yaml.contains("./src:/app/src") || yaml.contains("'./src:/app/src'"),
            "got: {}",
            yaml
        );
        assert!(!yaml.contains("./src_myslug"), "got: {}", yaml);
    }

    #[test]
    fn overlay_bind_mount_absolute_not_namespaced() {
        let cf = make_compose(vec![("app", null_svc(&[], &["/data:/data"]))]);
        let yaml = generate_overlay(&cf, 0, "myslug", None).unwrap();
        assert!(!yaml.contains("/data_myslug"), "got: {}", yaml);
    }

    #[test]
    fn overlay_volume_mapping_type_volume_namespaced() {
        let mut vm = serde_yaml::Mapping::new();
        vm.insert(
            serde_yaml::Value::String("type".into()),
            serde_yaml::Value::String("volume".into()),
        );
        vm.insert(
            serde_yaml::Value::String("source".into()),
            serde_yaml::Value::String("db_data".into()),
        );
        vm.insert(
            serde_yaml::Value::String("target".into()),
            serde_yaml::Value::String("/data".into()),
        );
        let s = Service {
            volumes: vec![serde_yaml::Value::Mapping(vm)],
            ..null_svc(&[], &[])
        };
        let cf = make_compose(vec![("db", s)]);
        let yaml = generate_overlay(&cf, 0, "myslug", None).unwrap();
        assert!(yaml.contains("db_data_myslug"), "got: {}", yaml);
    }

    #[test]
    fn overlay_volume_mapping_type_bind_not_namespaced() {
        let mut vm = serde_yaml::Mapping::new();
        vm.insert(
            serde_yaml::Value::String("type".into()),
            serde_yaml::Value::String("bind".into()),
        );
        vm.insert(
            serde_yaml::Value::String("source".into()),
            serde_yaml::Value::String("./src".into()),
        );
        vm.insert(
            serde_yaml::Value::String("target".into()),
            serde_yaml::Value::String("/app".into()),
        );
        let s = Service {
            volumes: vec![serde_yaml::Value::Mapping(vm)],
            ..null_svc(&[], &[])
        };
        let cf = make_compose(vec![("app", s)]);
        let yaml = generate_overlay(&cf, 0, "myslug", None).unwrap();
        assert!(!yaml.contains("./src_myslug"), "got: {}", yaml);
    }

    #[test]
    fn overlay_top_level_volumes_declared_with_suffix() {
        let s = null_svc(&[], &["db_data:/data"]);
        let mut cf = make_compose(vec![("db", s)]);
        cf.volumes.insert("db_data".into(), serde_yaml::Value::Null);
        let yaml = generate_overlay(&cf, 0, "slug", None).unwrap();
        assert!(yaml.contains("db_data_slug"), "got: {}", yaml);
    }

    #[test]
    fn overlay_services_to_include_filters_correctly() {
        let cf = make_compose(vec![
            ("app", null_svc(&["3000:3000"], &[])),
            ("db", null_svc(&["5432:5432"], &[])),
        ]);
        let yaml = generate_overlay(&cf, 100, "slug", Some(&["db".to_string()])).unwrap();
        assert!(yaml.contains("5532"), "got: {}", yaml);
        assert!(!yaml.contains("3100"), "got: {}", yaml);
    }

    #[test]
    fn overlay_empty_compose_no_crash() {
        let cf = make_compose(vec![]);
        let yaml = generate_overlay(&cf, 100, "slug", None).unwrap();
        // no panic, valid yaml (may be empty or just "---\n")
        let _: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap_or(serde_yaml::Value::Null);
    }

    #[test]
    fn overlay_zero_offset_keeps_original_port() {
        let cf = make_compose(vec![("app", null_svc(&["3000:3000"], &[]))]);
        let yaml = generate_overlay(&cf, 0, "slug", None).unwrap();
        assert!(yaml.contains("3000:3000"), "got: {}", yaml);
    }
}

/// Extract host-side port for a service (first port mapping, after offset)
pub fn service_host_port(svc: &Service, offset: u16) -> Option<u16> {
    for port in &svc.ports {
        let base = match port {
            serde_yaml::Value::String(s) => {
                let parts: Vec<&str> = s.split(':').collect();
                match parts.len() {
                    2 => parts[0].parse::<u16>().ok(),
                    3 => parts[1].parse::<u16>().ok(),
                    1 => parts[0].parse::<u16>().ok(),
                    _ => None,
                }
            }
            serde_yaml::Value::Number(n) => n.as_u64().map(|p| p as u16),
            _ => None,
        };
        if let Some(p) = base {
            return Some(p + offset);
        }
    }
    None
}

/// Resolve the compose file for a docker service.
/// If the service has an explicit `compose` path, resolve it relative to `root`.
/// Otherwise fall back to `find_compose_file(root)`.
pub fn resolve_service_compose(
    root: &Path,
    svc: &crate::config::ServiceConfig,
) -> Option<std::path::PathBuf> {
    if let Some(ref rel) = svc.compose {
        let p = root.join(rel);
        // Reject paths that escape the repo root via ../ or symlinks
        let canonical_p = p.canonicalize().ok()?;
        let canonical_root = root.canonicalize().ok()?;
        if !canonical_p.starts_with(&canonical_root) {
            return None;
        }
        if canonical_p.exists() {
            Some(canonical_p)
        } else {
            None
        }
    } else {
        find_compose_file(root)
    }
}

/// Find the compose file — checks docker-compose.yml, compose.yaml, compose.yml
pub fn find_compose_file(root: &Path) -> Option<std::path::PathBuf> {
    for name in &[
        "docker-compose.yml",
        "docker-compose.yaml",
        "compose.yaml",
        "compose.yml",
    ] {
        let p = root.join(name);
        if p.exists() {
            return Some(p);
        }
    }
    None
}
