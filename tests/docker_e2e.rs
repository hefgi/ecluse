// Docker-gated end-to-end tests: real `docker compose` against a local daemon.
// Every test no-ops (with a note) when docker is unavailable — they run on CI's
// ubuntu runners and on any dev machine with a daemon, and skip elsewhere
// (macOS runners have no docker).
//
// The image comes from the ECR public mirror: identical to docker.io alpine,
// but not subject to Docker Hub's unauthenticated pull rate limits, which
// shared CI runner IPs regularly exhaust.

use std::path::PathBuf;
use std::process::Command;

const TEST_IMAGE: &str = "public.ecr.aws/docker/library/alpine:3.20";

fn ecluse_bin() -> PathBuf {
    env!("CARGO_BIN_EXE_ecluse").into()
}

fn docker_available() -> bool {
    let daemon = Command::new("docker")
        .arg("info")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    let compose = Command::new("docker")
        .args(["compose", "version"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    daemon && compose
}

fn setup_repo(dir: &std::path::Path) {
    Command::new("git")
        .args(["init"])
        .current_dir(dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["-c", "commit.gpgsign=false"])
        .args(["commit", "--allow-empty", "-m", "init"])
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@test.com")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@test.com")
        .output()
        .unwrap();
}

/// HOME is pointed at the repo so the developer's real global config (and its
/// process_manager) never leaks into test behavior.
fn ecluse(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(ecluse_bin())
        .args(args)
        .current_dir(dir)
        .env("HOME", dir)
        .output()
        .expect("failed to run ecluse")
}

fn stdout(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn stderr(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

/// Names of running containers belonging to a compose project.
fn project_containers(project: &str) -> Vec<String> {
    let out = Command::new("docker")
        .args([
            "ps",
            "--filter",
            &format!("label=com.docker.compose.project={}", project),
            "--format",
            "{{.Names}}",
        ])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

/// Remove anything a previous (crashed) run of this project left behind.
fn docker_nuke(project: &str) {
    let out = Command::new("docker")
        .args([
            "ps",
            "-aq",
            "--filter",
            &format!("label=com.docker.compose.project={}", project),
        ])
        .output()
        .unwrap();
    for id in String::from_utf8_lossy(&out.stdout).lines() {
        let _ = Command::new("docker")
            .args(["rm", "-f", id.trim()])
            .output();
    }
}

fn write_compose(dir: &std::path::Path, rel: &str, services: &[&str]) {
    let mut body = String::from("services:\n");
    for svc in services {
        body.push_str(&format!(
            "  {}:\n    image: {}\n    command: sleep 300\n",
            svc, TEST_IMAGE
        ));
    }
    let path = dir.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, body).unwrap();
}

#[test]
fn hybrid_lifecycle_starts_and_stops_containers() {
    if !docker_available() {
        eprintln!("skipping: docker unavailable");
        return;
    }
    let repo = tempfile::tempdir().unwrap();
    setup_repo(repo.path());
    docker_nuke("ecluse_e2e-hy");

    write_compose(repo.path(), "docker-compose.yml", &["db"]);
    std::fs::write(
        repo.path().join(".ecluse.toml"),
        r#"mode = "hybrid"
inherit_env = []

[[services]]
name = "db"
run = "docker"
base_port = 5480
"#,
    )
    .unwrap();

    let up = ecluse(repo.path(), &["up", "e2e-hy"]);
    assert!(up.status.success(), "up failed: {}", stderr(&up));

    let containers = project_containers("ecluse_e2e-hy");
    assert_eq!(containers.len(), 1, "got containers: {:?}", containers);

    // The session records the (compose, overlay) pair and the allocated port.
    let state = std::fs::read_to_string(repo.path().join(".ecluse/state.json")).unwrap();
    assert!(state.contains("compose_overlays"), "got: {}", state);
    assert!(state.contains("\"db\": 5481"), "got: {}", state);

    // status sees the container as up, via the compose project label.
    let status = ecluse(repo.path(), &["status", "e2e-hy", "--json"]);
    let parsed: serde_json::Value = serde_json::from_str(&stdout(&status)).unwrap();
    let db = parsed["services"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "db")
        .unwrap()
        .clone();
    assert_eq!(db["healthy"], true, "got: {}", db);

    let down = ecluse(repo.path(), &["down", "--delete-worktree", "e2e-hy"]);
    assert!(down.status.success(), "down failed: {}", stderr(&down));
    assert!(
        project_containers("ecluse_e2e-hy").is_empty(),
        "containers must be gone after down"
    );
    assert!(
        !repo.path().join(".ecluse/overlays/e2e-hy.yml").exists(),
        "overlay must be removed"
    );
}

#[test]
fn failed_post_up_rolls_back_containers() {
    if !docker_available() {
        eprintln!("skipping: docker unavailable");
        return;
    }
    let repo = tempfile::tempdir().unwrap();
    setup_repo(repo.path());
    docker_nuke("ecluse_e2e-rb");

    write_compose(repo.path(), "docker-compose.yml", &["db"]);
    std::fs::write(
        repo.path().join(".ecluse.toml"),
        r#"mode = "hybrid"
inherit_env = []

[[services]]
name = "db"
run = "docker"
base_port = 5460

[hooks]
post_up = "false"
"#,
    )
    .unwrap();

    let up = ecluse(repo.path(), &["up", "e2e-rb"]);
    assert!(!up.status.success(), "up must fail on post_up");

    assert!(
        project_containers("ecluse_e2e-rb").is_empty(),
        "rollback must stop the containers it started"
    );
    assert!(
        !repo.path().join(".ecluse/worktrees/e2e-rb").exists(),
        "rollback must remove the fresh worktree"
    );
    let ls = ecluse(repo.path(), &["ls"]);
    assert!(
        stdout(&ls).contains("no active sessions"),
        "pending reservation must be cleared: {}",
        stdout(&ls)
    );
}

// The #9 regression scenario, end to end: a hyphenated slug whose suffix
// matches a real subdirectory with its own compose file. Teardown must use
// the recorded pairs, not filename parsing.
#[test]
fn hyphenated_slug_multi_compose_teardown() {
    if !docker_available() {
        eprintln!("skipping: docker unavailable");
        return;
    }
    let repo = tempfile::tempdir().unwrap();
    setup_repo(repo.path());
    docker_nuke("ecluse_feat-worker");

    write_compose(repo.path(), "docker-compose.yml", &["db"]);
    write_compose(repo.path(), "worker/docker-compose.yml", &["queue"]);
    std::fs::write(
        repo.path().join(".ecluse.toml"),
        r#"mode = "hybrid"
inherit_env = []

[[services]]
name = "db"
run = "docker"
base_port = 5470

[[services]]
name = "queue"
run = "docker"
base_port = 5700
compose = "worker/docker-compose.yml"
"#,
    )
    .unwrap();

    let up = ecluse(repo.path(), &["up", "feat-worker"]);
    assert!(up.status.success(), "up failed: {}", stderr(&up));
    assert_eq!(
        project_containers("ecluse_feat-worker").len(),
        2,
        "both compose groups must be up"
    );

    let down = ecluse(repo.path(), &["down", "--delete-worktree", "feat-worker"]);
    assert!(down.status.success(), "down failed: {}", stderr(&down));
    assert!(
        project_containers("ecluse_feat-worker").is_empty(),
        "both compose groups must be torn down despite the slug/subdir collision"
    );
}

#[test]
fn container_mode_runs_whole_compose_file() {
    if !docker_available() {
        eprintln!("skipping: docker unavailable");
        return;
    }
    let repo = tempfile::tempdir().unwrap();
    setup_repo(repo.path());
    docker_nuke("ecluse_e2e-ct");

    write_compose(repo.path(), "docker-compose.yml", &["web", "cache"]);
    std::fs::write(
        repo.path().join(".ecluse.toml"),
        "mode = \"container\"\ninherit_env = []\n",
    )
    .unwrap();

    let up = ecluse(repo.path(), &["up", "e2e-ct"]);
    assert!(up.status.success(), "up failed: {}", stderr(&up));
    assert_eq!(
        project_containers("ecluse_e2e-ct").len(),
        2,
        "container mode must bring up every service in the file"
    );

    let down = ecluse(repo.path(), &["down", "--delete-worktree", "e2e-ct"]);
    assert!(down.status.success(), "down failed: {}", stderr(&down));
    assert!(project_containers("ecluse_e2e-ct").is_empty());
}
