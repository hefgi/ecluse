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
