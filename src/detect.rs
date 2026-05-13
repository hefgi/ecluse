use std::path::Path;
use std::time::Duration;

use crate::config::Mode;

#[derive(Debug)]
pub struct DetectionResult {
    pub recommended: Option<Mode>,
    pub confidence: Confidence,
    pub scores: ModeScores,
    pub signals: Vec<Signal>,
    pub unsupported_reason: Option<String>,
}

#[derive(Debug)]
pub enum Confidence {
    High,
    Medium,
    Low,
    None,
}

impl std::fmt::Display for Confidence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Confidence::High => write!(f, "high"),
            Confidence::Medium => write!(f, "medium"),
            Confidence::Low => write!(f, "low"),
            Confidence::None => write!(f, "none"),
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct ModeScores {
    pub container: i32,
    pub host: i32,
    pub hybrid: i32,
}

#[derive(Debug)]
pub struct Signal {
    pub description: String,
    pub container_delta: i32,
    pub host_delta: i32,
    pub hybrid_delta: i32,
}

impl Signal {
    fn new(desc: &str, c: i32, h: i32, hy: i32) -> Self {
        Self {
            description: desc.to_string(),
            container_delta: c,
            host_delta: h,
            hybrid_delta: hy,
        }
    }
}

pub fn detect(root: &Path) -> DetectionResult {
    // Check unsupported first
    if root.join("flake.nix").exists() {
        return DetectionResult {
            recommended: None,
            confidence: Confidence::None,
            scores: ModeScores::default(),
            signals: vec![],
            unsupported_reason: Some(
                "Found flake.nix — Nix flake repos are not supported. \
                Use your flake's devShell for environment isolation."
                    .into(),
            ),
        };
    }
    for bazel in &["WORKSPACE", "BUILD.bazel", "MODULE.bazel"] {
        if root.join(bazel).exists() {
            return DetectionResult {
                recommended: None,
                confidence: Confidence::None,
                scores: ModeScores::default(),
                signals: vec![],
                unsupported_reason: Some(format!(
                    "Found {} — Bazel repos are not supported. Use Bazel's native sandbox.",
                    bazel
                )),
            };
        }
    }

    let mut scores = ModeScores::default();
    let mut signals: Vec<Signal> = Vec::new();

    // Docker not available — heavy penalty
    if !crate::docker::is_available() {
        let sig = Signal::new("Docker not installed or daemon not running", -10, 0, -10);
        scores.container += sig.container_delta;
        scores.hybrid += sig.hybrid_delta;
        signals.push(sig);
    }

    // Compose file present
    let compose_path = crate::compose::find_compose_file(root);
    if let Some(ref cp) = compose_path {
        let sig = Signal::new("docker-compose.yml or compose.yaml at repo root", 2, 0, 2);
        scores.container += sig.container_delta;
        scores.hybrid += sig.hybrid_delta;
        signals.push(sig);

        // Parse compose for deeper signals
        if let Ok(compose) = crate::compose::parse(cp) {
            // Any service has build: . (app builds from repo)
            let has_app_build = compose.services.values().any(|svc| match &svc.build {
                Some(serde_yaml::Value::String(s)) => s == "." || s.starts_with("./"),
                Some(serde_yaml::Value::Mapping(m)) => m
                    .get("context")
                    .and_then(|v| v.as_str())
                    .map(|s| s == "." || s.starts_with("./"))
                    .unwrap_or(false),
                _ => false,
            });
            if has_app_build {
                let sig = Signal::new(
                    "Compose has a service with build: . (app builds from repo root)",
                    3,
                    0,
                    0,
                );
                scores.container += sig.container_delta;
                signals.push(sig);
            }

            // All services match known data images
            let data_images = [
                "postgres",
                "redis",
                "mysql",
                "mongo",
                "rabbitmq",
                "mailhog",
                "minio",
                "elasticsearch",
                "nats",
                "kafka",
                "clickhouse",
                "memcached",
            ];
            let all_data = !compose.services.is_empty()
                && compose.services.values().all(|svc| {
                    svc.image
                        .as_deref()
                        .map(|img| {
                            let img_lower = img.to_lowercase();
                            data_images.iter().any(|d| img_lower.contains(d))
                        })
                        .unwrap_or(false)
                });
            if all_data {
                let sig = Signal::new(
                    "Compose services all match known data images (postgres, redis, etc.)",
                    -2,
                    0,
                    5,
                );
                scores.container += sig.container_delta;
                scores.hybrid += sig.hybrid_delta;
                signals.push(sig);
            }

            // Any service has ecluse.role: app label
            let has_app_label =
                !crate::compose::app_services(&compose, "ecluse.role", "app").is_empty();
            if has_app_label {
                let sig = Signal::new("Compose has a service with label ecluse.role=app", 0, 0, 10);
                scores.hybrid += sig.hybrid_delta;
                signals.push(sig);
            }

            // Compose has bind mounts
            let has_bind_mounts = compose.services.values().any(|svc| {
                svc.volumes.iter().any(|v| {
                    v.as_str()
                        .map(|s| s.starts_with("./") || s.starts_with("/"))
                        .unwrap_or(false)
                })
            });
            if has_bind_mounts {
                let sig = Signal::new("Compose has bind mounts of source into containers", 2, 0, 0);
                scores.container += sig.container_delta;
                signals.push(sig);
            }

            // Compose has watch blocks
            let has_watch = compose
                .services
                .values()
                .any(|svc| svc.other.contains_key("develop") || svc.other.contains_key("watch"));
            if has_watch {
                let sig = Signal::new("Compose has watch: blocks", 2, 0, 1);
                scores.container += sig.container_delta;
                scores.hybrid += sig.hybrid_delta;
                signals.push(sig);
            }
        }
    } else {
        let sig = Signal::new("No compose file found", -5, 4, -5);
        scores.container += sig.container_delta;
        scores.host += sig.host_delta;
        scores.hybrid += sig.hybrid_delta;
        signals.push(sig);
    }

    // .devcontainer
    if root
        .join(".devcontainer")
        .join("devcontainer.json")
        .exists()
    {
        let sig = Signal::new(".devcontainer/devcontainer.json exists", 4, 0, 0);
        scores.container += sig.container_delta;
        signals.push(sig);
    }

    // bin/dev
    let bin_dev = root.join("bin").join("dev");
    if bin_dev.exists() {
        let sig = Signal::new("bin/dev exists and is executable", 0, 3, 2);
        scores.host += sig.host_delta;
        scores.hybrid += sig.hybrid_delta;
        signals.push(sig);
    }

    // Procfile.dev
    if root.join("Procfile.dev").exists() {
        let sig = Signal::new("Procfile.dev exists", 0, 3, 2);
        scores.host += sig.host_delta;
        scores.hybrid += sig.hybrid_delta;
        signals.push(sig);
    }

    // package.json with non-docker dev script
    if root.join("package.json").exists() {
        if let Ok(pkg) = std::fs::read_to_string(root.join("package.json")) {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&pkg) {
                let dev_script = json
                    .get("scripts")
                    .and_then(|s| s.get("dev"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let has_docker = dev_script.contains("docker") || dev_script.contains("compose");
                if !dev_script.is_empty() && !has_docker {
                    let sig = Signal::new(
                        "package.json has a dev script that does not call docker/compose",
                        0,
                        2,
                        2,
                    );
                    scores.host += sig.host_delta;
                    scores.hybrid += sig.hybrid_delta;
                    signals.push(sig);
                }
            }
        }
    }

    // Gemfile + bin/rails
    if root.join("Gemfile").exists() && root.join("bin").join("rails").exists() {
        let sig = Signal::new("Gemfile + bin/rails present (Rails app)", 0, 2, 2);
        scores.host += sig.host_delta;
        scores.hybrid += sig.hybrid_delta;
        signals.push(sig);
    }

    // Version managers
    for f in &[
        ".tool-versions",
        ".nvmrc",
        ".python-version",
        ".ruby-version",
        "mise.toml",
    ] {
        if root.join(f).exists() {
            let sig = Signal::new(&format!("{} present (version manager)", f), 0, 1, 1);
            scores.host += sig.host_delta;
            scores.hybrid += sig.hybrid_delta;
            signals.push(sig);
            break;
        }
    }

    // README hybrid pattern
    if let Ok(readme) = find_readme(root) {
        if readme_has_hybrid_pattern(&readme) {
            let sig = Signal::new(
                "README mentions 'docker compose up' followed by 'bin/dev' or 'npm run dev'",
                0,
                0,
                3,
            );
            scores.hybrid += sig.hybrid_delta;
            signals.push(sig);
        }
    }

    // Host Postgres probe (non-blocking with timeout)
    // We do this check last since it involves a network probe
    if probe_postgres_5432() {
        let sig = Signal::new("Host Postgres reachable on localhost:5432", 0, 1, 0);
        scores.host += sig.host_delta;
        signals.push(sig);
    }

    // Determine winner
    let max_score = scores.container.max(scores.host).max(scores.hybrid);
    let recommended = if max_score <= 0 {
        None
    } else if scores.container == max_score {
        Some(Mode::Container)
    } else if scores.hybrid == max_score {
        Some(Mode::Hybrid)
    } else {
        Some(Mode::Host)
    };

    // Determine runner-up gap
    let mut sorted = [
        ("container", scores.container),
        ("host", scores.host),
        ("hybrid", scores.hybrid),
    ];
    sorted.sort_by_key(|b| std::cmp::Reverse(b.1));
    let gap = sorted[0].1 - sorted[1].1;

    let confidence = match (recommended.is_some(), gap) {
        (false, _) => Confidence::None,
        (true, g) if g >= 4 => Confidence::High,
        (true, g) if g >= 2 => Confidence::Medium,
        _ => Confidence::Low,
    };

    DetectionResult {
        recommended,
        confidence,
        scores,
        signals,
        unsupported_reason: None,
    }
}

fn find_readme(root: &Path) -> anyhow::Result<String> {
    for name in &["README.md", "README.rst", "README"] {
        let p = root.join(name);
        if p.exists() {
            return Ok(std::fs::read_to_string(p)?);
        }
    }
    Ok(String::new())
}

fn readme_has_hybrid_pattern(content: &str) -> bool {
    let lines: Vec<&str> = content.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        if line.contains("docker compose up") {
            let window = &lines[i..std::cmp::min(i + 10, lines.len())];
            if window.iter().any(|l| {
                l.contains("bin/dev") || l.contains("npm run dev") || l.contains("yarn dev")
            }) {
                return true;
            }
        }
    }
    false
}

fn probe_postgres_5432() -> bool {
    use std::net::TcpStream;
    TcpStream::connect_timeout(
        &"127.0.0.1:5432".parse().unwrap(),
        Duration::from_millis(500),
    )
    .is_ok()
}

pub fn print_detection_result(result: &DetectionResult) {
    println!("\nMode detection results:\n");
    println!("  Signals found:");
    for sig in &result.signals {
        let parts: Vec<String> = [
            if sig.container_delta != 0 {
                format!("{:+} container", sig.container_delta)
            } else {
                String::new()
            },
            if sig.host_delta != 0 {
                format!("{:+} host", sig.host_delta)
            } else {
                String::new()
            },
            if sig.hybrid_delta != 0 {
                format!("{:+} hybrid", sig.hybrid_delta)
            } else {
                String::new()
            },
        ]
        .iter()
        .filter(|s| !s.is_empty())
        .cloned()
        .collect();
        println!("    + {}  [{}]", sig.description, parts.join(", "));
    }

    println!();
    println!("  Scores:");
    println!("    container: {}", result.scores.container);
    println!("    host:      {}", result.scores.host);
    println!("    hybrid:    {}", result.scores.hybrid);
    println!();

    match &result.recommended {
        None => println!("  No mode recommended — all scores ≤ 0. Use --mode to specify."),
        Some(mode) => println!("  Recommended: {} ({})", mode, result.confidence,),
    }
    println!();
}
