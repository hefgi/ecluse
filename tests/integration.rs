use std::path::PathBuf;
use std::process::Command;

fn ecluse_bin() -> PathBuf {
    env!("CARGO_BIN_EXE_ecluse").into()
}

fn setup_repo(dir: &std::path::Path) {
    Command::new("git")
        .args(["init"])
        .current_dir(dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "--allow-empty", "-m", "init"])
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@test.com")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@test.com")
        .output()
        .unwrap();
}

fn ecluse(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(ecluse_bin())
        .args(args)
        .current_dir(dir)
        .output()
        .expect("failed to run ecluse")
}

fn stdout(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn stderr(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn tmp_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    setup_repo(dir.path());
    dir
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[test]
fn version() {
    let out = Command::new(ecluse_bin())
        .arg("--version")
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(stdout(&out).contains("ecluse"));
}

#[test]
fn init_host_creates_config() {
    let repo = tmp_repo();
    let out = ecluse(repo.path(), &["init", "--mode", "host", "--yes"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(repo.path().join(".ecluse.toml").exists());
    let config = std::fs::read_to_string(repo.path().join(".ecluse.toml")).unwrap();
    assert!(config.contains("mode = \"host\""));
    // New model: no base_port/stride at top level; services define their own ports
    assert!(!config.contains("stride"));
    assert!(config.contains("prefix = \"ecluse\""));
}

#[test]
fn init_adds_gitignore_entry() {
    let repo = tmp_repo();
    ecluse(repo.path(), &["init", "--mode", "host", "--yes"]);
    let gitignore = std::fs::read_to_string(repo.path().join(".gitignore")).unwrap();
    assert!(gitignore.contains(".ecluse/"));
}

#[test]
fn up_creates_worktree_and_env() {
    let repo = tmp_repo();
    ecluse(repo.path(), &["init", "--mode", "host", "--yes"]);
    let out = ecluse(repo.path(), &["up", "feat-foo"]);
    assert!(out.status.success(), "{}", stderr(&out));

    let worktree = repo.path().join(".ecluse/worktrees/feat-foo");
    assert!(worktree.exists(), "worktree directory missing");

    let env = std::fs::read_to_string(worktree.join(".env.ecluse")).unwrap();
    // Fallback: no [[services]] defined → "app" service at 3000 + slot
    // slot 1 → PORT=3001
    assert!(env.contains("PORT=3001"), "got env: {}", env);
    assert!(env.contains("ECLUSE_SLOT=1"));
    assert!(env.contains("ECLUSE_MODE=host"));
    // ECLUSE_OFFSET is removed in the new model
    assert!(!env.contains("ECLUSE_OFFSET="));
}

#[test]
fn up_output_shows_correct_port() {
    let repo = tmp_repo();
    ecluse(repo.path(), &["init", "--mode", "host", "--yes"]);
    let out = ecluse(repo.path(), &["up", "feat-foo"]);
    // slot 1 + fallback base 3000 → 3001
    assert!(
        stdout(&out).contains("App port:  3001"),
        "got: {}",
        stdout(&out)
    );
}

#[test]
fn parallel_sessions_get_different_slots_and_ports() {
    let repo = tmp_repo();
    ecluse(repo.path(), &["init", "--mode", "host", "--yes"]);

    let out1 = ecluse(repo.path(), &["up", "feat-foo"]);
    assert!(out1.status.success(), "{}", stderr(&out1));
    assert!(stdout(&out1).contains("slot 1"));
    // slot 1 → 3001
    assert!(
        stdout(&out1).contains("App port:  3001"),
        "got: {}",
        stdout(&out1)
    );

    let out2 = ecluse(repo.path(), &["up", "fix-bar"]);
    assert!(out2.status.success(), "{}", stderr(&out2));
    assert!(stdout(&out2).contains("slot 2"));
    // slot 2 → 3002
    assert!(
        stdout(&out2).contains("App port:  3002"),
        "got: {}",
        stdout(&out2)
    );
}

#[test]
fn duplicate_slug_resumes_idempotently() {
    let repo = tmp_repo();
    ecluse(repo.path(), &["init", "--mode", "host", "--yes"]);
    ecluse(repo.path(), &["up", "feat-foo"]);
    // Second up on same slug should succeed (resume path, not an error).
    let out = ecluse(repo.path(), &["up", "feat-foo"]);
    assert!(out.status.success(), "got: {}", stderr(&out));
    // State should still have exactly one session for feat-foo.
    let ls = ecluse(repo.path(), &["ls"]);
    assert!(stdout(&ls).contains("feat-foo"), "got: {}", stdout(&ls));
}

#[test]
fn down_removes_worktree_and_clears_state() {
    let repo = tmp_repo();
    ecluse(repo.path(), &["init", "--mode", "host", "--yes"]);
    ecluse(repo.path(), &["up", "feat-foo"]);

    let out = ecluse(repo.path(), &["down", "--delete-worktree", "feat-foo"]);
    assert!(out.status.success(), "{}", stderr(&out));

    let worktree = repo.path().join(".ecluse/worktrees/feat-foo");
    assert!(!worktree.exists(), "worktree should be removed");

    let ls = ecluse(repo.path(), &["ls"]);
    assert!(stdout(&ls).contains("no active sessions"));
}

#[test]
fn down_nonexistent_slug_errors() {
    let repo = tmp_repo();
    ecluse(repo.path(), &["init", "--mode", "host", "--yes"]);
    let out = ecluse(
        repo.path(),
        &["down", "--delete-worktree", "does-not-exist"],
    );
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("does-not-exist"),
        "got: {}",
        stderr(&out)
    );
}

#[test]
fn ls_shows_active_sessions() {
    let repo = tmp_repo();
    ecluse(repo.path(), &["init", "--mode", "host", "--yes"]);
    ecluse(repo.path(), &["up", "feat-foo"]);
    ecluse(repo.path(), &["up", "fix-bar"]);

    let out = ecluse(repo.path(), &["ls"]);
    assert!(stdout(&out).contains("feat-foo"));
    assert!(stdout(&out).contains("fix-bar"));
}

#[test]
fn ls_json_is_valid_json() {
    let repo = tmp_repo();
    ecluse(repo.path(), &["init", "--mode", "host", "--yes"]);
    ecluse(repo.path(), &["up", "feat-foo"]);

    let out = ecluse(repo.path(), &["ls", "--json"]);
    assert!(out.status.success());
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout(&out)).expect("ls --json output is not valid JSON");
    assert!(parsed.is_array());
    assert_eq!(parsed.as_array().unwrap().len(), 1);
}

#[test]
fn slot_reuse_after_down() {
    let repo = tmp_repo();
    ecluse(repo.path(), &["init", "--mode", "host", "--yes"]);
    ecluse(repo.path(), &["up", "feat-foo"]);
    ecluse(repo.path(), &["down", "--delete-worktree", "feat-foo"]);

    // New session should reuse slot 1
    let out = ecluse(repo.path(), &["up", "feat-bar"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).contains("slot 1"));
    // slot 1 → 3001
    assert!(
        stdout(&out).contains("App port:  3001"),
        "got: {}",
        stdout(&out)
    );
}

#[test]
fn services_config_sets_per_service_ports() {
    let repo = tmp_repo();
    // Write a config with explicit [[services]]
    std::fs::write(
        repo.path().join(".ecluse.toml"),
        r#"mode = "host"

[[services]]
name = "api"
run = "native"
base_port = 8000
command = "echo api"

[[services]]
name = "frontend"
run = "native"
base_port = 3000
command = "echo frontend"
"#,
    )
    .unwrap();
    let out = ecluse(repo.path(), &["up", "svc-ports"]);
    assert!(out.status.success(), "{}", stderr(&out));

    let worktree = repo.path().join(".ecluse/worktrees/svc-ports");
    let env = std::fs::read_to_string(worktree.join(".env.ecluse")).unwrap();

    // api: 8000 + 1 = 8001; frontend: 3000 + 1 = 3001
    assert!(env.contains("ECLUSE_API_PORT=8001"), "got env: {}", env);
    assert!(
        env.contains("ECLUSE_FRONTEND_PORT=3001"),
        "got env: {}",
        env
    );
    // PORT alias = first native service (api)
    assert!(env.contains("PORT=8001"), "got env: {}", env);

    ecluse(repo.path(), &["down", "--delete-worktree", "svc-ports"]);
}

#[test]
fn services_config_slot2_increments_correctly() {
    let repo = tmp_repo();
    std::fs::write(
        repo.path().join(".ecluse.toml"),
        r#"mode = "host"

[[services]]
name = "api"
run = "native"
base_port = 8000
command = "echo api"
"#,
    )
    .unwrap();
    // First session: slot 1 → 8001
    let out1 = ecluse(repo.path(), &["up", "svc-slot1"]);
    assert!(out1.status.success(), "{}", stderr(&out1));

    // Second session: slot 2 → 8002
    let out2 = ecluse(repo.path(), &["up", "svc-slot2"]);
    assert!(out2.status.success(), "{}", stderr(&out2));

    let env2 = std::fs::read_to_string(repo.path().join(".ecluse/worktrees/svc-slot2/.env.ecluse"))
        .unwrap();
    assert!(env2.contains("ECLUSE_API_PORT=8002"), "got env: {}", env2);
    assert!(env2.contains("PORT=8002"), "got env: {}", env2);

    ecluse(repo.path(), &["down", "--delete-worktree", "svc-slot1"]);
    ecluse(repo.path(), &["down", "--delete-worktree", "svc-slot2"]);
}

#[test]
fn invalid_slug_is_rejected() {
    let repo = tmp_repo();
    ecluse(repo.path(), &["init", "--mode", "host", "--yes"]);
    let out = ecluse(repo.path(), &["up", "Invalid_Slug!"]);
    assert!(!out.status.success());
}

#[test]
fn up_without_init_errors() {
    let repo = tmp_repo();
    let out = ecluse(repo.path(), &["up", "feat-foo"]);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("ecluse init"),
        "got: {}",
        stderr(&out)
    );
}
