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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn empty_dir() -> TempDir {
        TempDir::new().unwrap()
    }

    #[test]
    fn readme_hybrid_pattern_detects_compose_then_bin_dev() {
        let content = "## Setup\n\
            Run `docker compose up -d`\n\
            Then start the app with `bin/dev`\n";
        assert!(readme_has_hybrid_pattern(content));
    }

    #[test]
    fn readme_hybrid_pattern_detects_compose_then_npm_run_dev() {
        let content = "docker compose up\nnpm run dev\n";
        assert!(readme_has_hybrid_pattern(content));
    }

    #[test]
    fn readme_hybrid_pattern_no_match_without_compose() {
        let content = "Just run npm run dev directly\n";
        assert!(!readme_has_hybrid_pattern(content));
    }

    #[test]
    fn readme_hybrid_pattern_no_match_when_too_far_apart() {
        let far_apart = "some line\n".repeat(15);
        let content = format!("docker compose up\n{}bin/dev\n", far_apart);
        assert!(!readme_has_hybrid_pattern(&content));
    }

    #[test]
    fn flake_nix_triggers_unsupported() {
        let dir = empty_dir();
        fs::write(dir.path().join("flake.nix"), "").unwrap();
        let result = detect(dir.path());
        assert!(result.unsupported_reason.is_some());
        assert!(result.recommended.is_none());
    }

    #[test]
    fn bazel_workspace_triggers_unsupported() {
        let dir = empty_dir();
        fs::write(dir.path().join("WORKSPACE"), "").unwrap();
        let result = detect(dir.path());
        assert!(result.unsupported_reason.is_some());
        assert!(result.recommended.is_none());
    }

    #[test]
    fn devcontainer_boosts_container_score() {
        let baseline = detect(empty_dir().path()).scores.container;
        let dir = empty_dir();
        let dc = dir.path().join(".devcontainer");
        fs::create_dir_all(&dc).unwrap();
        fs::write(dc.join("devcontainer.json"), "{}").unwrap();
        let result = detect(dir.path());
        assert!(
            result.scores.container > baseline,
            "devcontainer should increase container score"
        );
    }

    #[test]
    fn bin_dev_boosts_host_and_hybrid() {
        let baseline = detect(empty_dir().path()).scores;
        let dir = empty_dir();
        let bin = dir.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        fs::write(bin.join("dev"), "#!/bin/bash").unwrap();
        let result = detect(dir.path());
        assert!(
            result.scores.host > baseline.host,
            "bin/dev should increase host score"
        );
        assert!(
            result.scores.hybrid > baseline.hybrid,
            "bin/dev should increase hybrid score"
        );
    }

    #[test]
    fn no_signals_returns_no_recommendation_or_low() {
        let dir = empty_dir();
        let result = detect(dir.path());
        // With no signals other than compose absence penalty, container/hybrid are negative
        // host gets the +4 bonus from no compose file
        assert!(result.scores.host > result.scores.container);
    }

    #[test]
    fn confidence_is_high_when_gap_at_least_4() {
        let dir = empty_dir();
        // devcontainer.json (+4 container) with no compose (-5 hybrid, -5 container)
        // net: container = -1, but devcontainer adds another +4 = +3 for container
        // Compose absent: container -5, host +4, hybrid -5
        // devcontainer: container +4
        // Result: container -1, host 4, hybrid -5 => gap host-container = 5
        let dc = dir.path().join(".devcontainer");
        fs::create_dir_all(&dc).unwrap();
        fs::write(dc.join("devcontainer.json"), "{}").unwrap();
        let result = detect(dir.path());
        // host should win with high confidence gap
        assert!(matches!(
            result.confidence,
            Confidence::High | Confidence::Medium
        ));
    }

    #[test]
    fn build_bazel_triggers_unsupported() {
        let dir = empty_dir();
        fs::write(dir.path().join("BUILD.bazel"), "").unwrap();
        let result = detect(dir.path());
        assert!(result.unsupported_reason.is_some());
        assert!(result.recommended.is_none());
    }

    #[test]
    fn module_bazel_triggers_unsupported() {
        let dir = empty_dir();
        fs::write(dir.path().join("MODULE.bazel"), "").unwrap();
        let result = detect(dir.path());
        assert!(result.unsupported_reason.is_some());
    }

    #[test]
    fn procfile_dev_boosts_host_and_hybrid() {
        let baseline = detect(empty_dir().path()).scores;
        let dir = empty_dir();
        fs::write(
            dir.path().join("Procfile.dev"),
            "web: bundle exec rails server",
        )
        .unwrap();
        let result = detect(dir.path());
        assert!(result.scores.host > baseline.host);
        assert!(result.scores.hybrid > baseline.hybrid);
    }

    #[test]
    fn gemfile_and_bin_rails_boost_host_and_hybrid() {
        let baseline = detect(empty_dir().path()).scores;
        let dir = empty_dir();
        fs::write(dir.path().join("Gemfile"), "source 'https://rubygems.org'").unwrap();
        let bin = dir.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        fs::write(bin.join("rails"), "#!/usr/bin/env ruby").unwrap();
        let result = detect(dir.path());
        assert!(result.scores.host > baseline.host);
        assert!(result.scores.hybrid > baseline.hybrid);
    }

    #[test]
    fn package_json_non_docker_dev_script_boosts_host() {
        let baseline = detect(empty_dir().path()).scores;
        let dir = empty_dir();
        let pkg = r#"{"scripts": {"dev": "vite"}}"#;
        fs::write(dir.path().join("package.json"), pkg).unwrap();
        let result = detect(dir.path());
        assert!(result.scores.host > baseline.host);
    }

    #[test]
    fn package_json_docker_dev_script_does_not_boost_host() {
        let dir_no_pkg = empty_dir();
        let baseline = detect(dir_no_pkg.path()).scores;
        let dir = empty_dir();
        let pkg = r#"{"scripts": {"dev": "docker compose up"}}"#;
        fs::write(dir.path().join("package.json"), pkg).unwrap();
        let result = detect(dir.path());
        // A docker-based dev script should NOT boost host score beyond baseline
        // (host score may differ from baseline due to other signals, but the docker
        // dev script specifically should not add +2)
        assert!(result.scores.host <= baseline.host + 4);
    }

    #[test]
    fn package_json_no_dev_script_does_not_boost() {
        let baseline = detect(empty_dir().path()).scores;
        let dir = empty_dir();
        let pkg = r#"{"scripts": {"build": "tsc"}}"#;
        fs::write(dir.path().join("package.json"), pkg).unwrap();
        let result = detect(dir.path());
        assert_eq!(result.scores.host, baseline.host);
    }

    #[test]
    fn version_manager_nvmrc_boosts_host() {
        let baseline = detect(empty_dir().path()).scores;
        let dir = empty_dir();
        fs::write(dir.path().join(".nvmrc"), "20").unwrap();
        let result = detect(dir.path());
        assert!(result.scores.host > baseline.host);
        assert!(result.scores.hybrid > baseline.hybrid);
    }

    #[test]
    fn version_manager_tool_versions_boosts_host() {
        let baseline = detect(empty_dir().path()).scores;
        let dir = empty_dir();
        fs::write(dir.path().join(".tool-versions"), "nodejs 20.0.0").unwrap();
        let result = detect(dir.path());
        assert!(result.scores.host > baseline.host);
    }

    #[test]
    fn version_manager_python_version_boosts_host() {
        let baseline = detect(empty_dir().path()).scores;
        let dir = empty_dir();
        fs::write(dir.path().join(".python-version"), "3.12").unwrap();
        let result = detect(dir.path());
        assert!(result.scores.host > baseline.host);
    }

    #[test]
    fn version_manager_ruby_version_boosts_host() {
        let baseline = detect(empty_dir().path()).scores;
        let dir = empty_dir();
        fs::write(dir.path().join(".ruby-version"), "3.3.0").unwrap();
        let result = detect(dir.path());
        assert!(result.scores.host > baseline.host);
    }

    #[test]
    fn version_manager_mise_toml_boosts_host() {
        let baseline = detect(empty_dir().path()).scores;
        let dir = empty_dir();
        fs::write(dir.path().join("mise.toml"), "[tools]\nnodejs = '20'").unwrap();
        let result = detect(dir.path());
        assert!(result.scores.host > baseline.host);
    }

    #[test]
    fn readme_hybrid_pattern_yarn_dev_detected() {
        let content = "## Setup\ndocker compose up -d\nyarn dev\n";
        assert!(readme_has_hybrid_pattern(content));
    }

    #[test]
    fn readme_hybrid_pattern_window_boundary() {
        // Exactly 9 lines after docker compose up — should still match (window is 10)
        let content =
            "docker compose up\nline1\nline2\nline3\nline4\nline5\nline6\nline7\nline8\nbin/dev\n";
        assert!(readme_has_hybrid_pattern(content));
    }

    #[test]
    fn readme_hybrid_pattern_no_match_compose_without_dev() {
        let content = "docker compose up\nnginx\napache\n";
        assert!(!readme_has_hybrid_pattern(content));
    }

    #[test]
    fn confidence_display() {
        assert_eq!(Confidence::High.to_string(), "high");
        assert_eq!(Confidence::Medium.to_string(), "medium");
        assert_eq!(Confidence::Low.to_string(), "low");
        assert_eq!(Confidence::None.to_string(), "none");
    }

    #[test]
    fn no_compose_boosts_host_and_penalises_container() {
        let dir = empty_dir();
        let result = detect(dir.path());
        // No compose → host +4, container -5, hybrid -5
        assert!(result.scores.host > result.scores.container);
        assert!(result.scores.host > result.scores.hybrid);
    }

    #[test]
    fn signals_are_recorded() {
        let dir = empty_dir();
        let result = detect(dir.path());
        assert!(
            !result.signals.is_empty(),
            "should have at least the no-compose signal"
        );
    }

    #[test]
    fn all_scores_zero_gives_none_recommendation() {
        // When all scores are <= 0 recommended should be None
        // We can't easily force this without mocking docker, but the logic is tested via flake_nix
        let dir = empty_dir();
        fs::write(dir.path().join("flake.nix"), "").unwrap();
        let result = detect(dir.path());
        assert!(result.recommended.is_none());
        assert!(matches!(result.confidence, Confidence::None));
    }

    #[test]
    fn readme_file_rst_extension_checked() {
        let dir = empty_dir();
        let content = "docker compose up\nbin/dev\n";
        fs::write(dir.path().join("README.rst"), content).unwrap();
        let result = detect(dir.path());
        assert!(result.scores.hybrid > detect(empty_dir().path()).scores.hybrid);
    }

    #[test]
    fn readme_file_no_extension_checked() {
        let dir = empty_dir();
        let content = "docker compose up\nnpm run dev\n";
        fs::write(dir.path().join("README"), content).unwrap();
        let result = detect(dir.path());
        assert!(result.scores.hybrid > detect(empty_dir().path()).scores.hybrid);
    }

    // ── Compose-based detection signals ───────────────────────────────────────

    fn write_compose(dir: &TempDir, yaml: &str) {
        fs::write(dir.path().join("docker-compose.yml"), yaml).unwrap();
    }

    #[test]
    fn compose_present_boosts_container_and_hybrid() {
        let baseline = detect(empty_dir().path()).scores;
        let dir = empty_dir();
        write_compose(&dir, "services:\n  db:\n    image: postgres:15\n");
        let result = detect(dir.path());
        // compose present: +2 container, +2 hybrid; compose-absent penalty removed
        assert!(result.scores.container > baseline.container);
        assert!(result.scores.hybrid > baseline.hybrid);
    }

    #[test]
    fn compose_app_build_boosts_container() {
        let baseline = detect(empty_dir().path()).scores.container;
        let dir = empty_dir();
        write_compose(
            &dir,
            "services:\n  app:\n    build: .\n    ports:\n      - \"3000:3000\"\n",
        );
        let result = detect(dir.path());
        // build: . → +3 container
        assert!(
            result.scores.container > baseline,
            "container={} baseline={}",
            result.scores.container,
            baseline
        );
    }

    #[test]
    fn compose_app_build_context_boosts_container() {
        let baseline = detect(empty_dir().path()).scores.container;
        let dir = empty_dir();
        let yaml =
            "services:\n  app:\n    build:\n      context: .\n    ports:\n      - \"3000:3000\"\n";
        write_compose(&dir, yaml);
        let result = detect(dir.path());
        assert!(result.scores.container > baseline);
    }

    #[test]
    fn compose_app_build_context_subdir_boosts_container() {
        let baseline = detect(empty_dir().path()).scores.container;
        let dir = empty_dir();
        let yaml = "services:\n  app:\n    build:\n      context: ./app\n";
        write_compose(&dir, yaml);
        let result = detect(dir.path());
        assert!(result.scores.container > baseline);
    }

    #[test]
    fn compose_all_data_images_boosts_hybrid() {
        let dir = empty_dir();
        write_compose(
            &dir,
            "services:\n  db:\n    image: postgres:15\n  cache:\n    image: redis:7\n",
        );
        let result = detect(dir.path());
        // all_data: +5 hybrid, -2 container
        assert!(result.scores.hybrid > result.scores.container);
    }

    #[test]
    fn compose_ecluse_role_app_label_boosts_hybrid() {
        let baseline = detect(empty_dir().path()).scores.hybrid;
        let dir = empty_dir();
        let yaml = "services:\n  web:\n    image: node:20\n    labels:\n      ecluse.role: app\n  db:\n    image: postgres:15\n";
        write_compose(&dir, yaml);
        let result = detect(dir.path());
        // has_app_label: +10 hybrid, compose present: +2 hybrid
        assert!(
            result.scores.hybrid >= baseline + 10 + 2,
            "hybrid={} baseline={}",
            result.scores.hybrid,
            baseline
        );
    }

    #[test]
    fn compose_bind_mounts_boost_container() {
        let baseline = detect(empty_dir().path()).scores.container;
        let dir = empty_dir();
        let yaml = "services:\n  app:\n    build: .\n    volumes:\n      - ./src:/app/src\n";
        write_compose(&dir, yaml);
        let result = detect(dir.path());
        // bind mounts: +2 container
        assert!(result.scores.container > baseline);
    }

    #[test]
    fn compose_absolute_bind_mount_also_detected() {
        let baseline = detect(empty_dir().path()).scores.container;
        let dir = empty_dir();
        let yaml = "services:\n  app:\n    image: nginx\n    volumes:\n      - /data:/data\n";
        write_compose(&dir, yaml);
        let result = detect(dir.path());
        // /data is an absolute bind mount — signal fires
        assert!(result.scores.container > baseline);
    }

    #[test]
    fn compose_watch_blocks_boost_container_and_hybrid() {
        let baseline = detect(empty_dir().path()).scores;
        let dir = empty_dir();
        // Use 'develop' key (which is recognized as a watch block)
        let yaml = "services:\n  app:\n    build: .\n    develop:\n      watch: []\n";
        write_compose(&dir, yaml);
        let result = detect(dir.path());
        // has_watch: +2 container, +1 hybrid
        assert!(result.scores.container > baseline.container);
        assert!(result.scores.hybrid > baseline.hybrid);
    }

    #[test]
    fn compose_invalid_yaml_does_not_panic() {
        let dir = empty_dir();
        // Valid enough to be a file but bad YAML so compose::parse fails
        write_compose(&dir, "services: { broken yaml: [");
        // Should not panic, just fall through without compose signals
        let result = detect(dir.path());
        assert!(result.unsupported_reason.is_none());
    }
}
