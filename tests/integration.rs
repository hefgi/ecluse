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
fn status_reports_expected_port_from_state() {
    // Regression: cmd_status used to report whatever port the matched
    // process happened to be listening on (m.port from sync::match_services),
    // which could be wrong if a child process bound a different port.
    // It must always report the allocated/expected port from state.json
    // — the same port written into .env.ecluse — so `ecluse status` never
    // contradicts the env the agent is actually using.
    let repo = tmp_repo();
    std::fs::write(
        repo.path().join(".ecluse.toml"),
        r#"mode = "host"

[[services]]
name = "api"
base_port = 4000
command = "echo api"
"#,
    )
    .unwrap();

    let up = ecluse(repo.path(), &["up", "feat-foo"]);
    assert!(up.status.success(), "up failed: {}", stderr(&up));

    let status = ecluse(repo.path(), &["status", "feat-foo", "--json"]);
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout(&status)).expect("status --json output is not valid JSON");

    // Verify the api service's reported port matches the allocated port
    // (base_port=4000 + slot=1 = 4001), and matches what's in state.json.
    let services = parsed["services"].as_array().unwrap();
    let api = services.iter().find(|s| s["name"] == "api").unwrap();
    assert_eq!(
        api["port"].as_u64(),
        Some(4001),
        "status should report the allocated port from state, got: {}",
        api
    );

    // And state.json should have the same value — confirms the chain is
    // consistent.
    let state: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(repo.path().join(".ecluse/state.json")).unwrap(),
    )
    .unwrap();
    let recorded = state["sessions"][0]["port_overrides"]["api"]
        .as_u64()
        .unwrap();
    assert_eq!(recorded, 4001);
}

#[test]
fn resume_honors_external_worktree_path() {
    // Regression test: when a session's worktree lives outside .ecluse/worktrees/
    // (e.g. a sibling git worktree the user created manually), `ecluse up` from
    // inside that worktree must reuse the recorded worktree_path on resume —
    // not recompute the default <root>/<worktree_dir>/<slug> location.
    let repo = tmp_repo();
    ecluse(repo.path(), &["init", "--mode", "host", "--yes"]);

    // Create a sibling worktree in its own tmpdir on a new branch.
    let sibling_parent = tempfile::tempdir().unwrap();
    let sibling = sibling_parent.path().join("sibling-wt");
    let out = Command::new("git")
        .args(["worktree", "add", "-b", "feature/sibling"])
        .arg(&sibling)
        .current_dir(repo.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git worktree add failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // First `ecluse up` from inside the sibling auto-registers it with the
    // external path. This should succeed without --reuse-worktree.
    let out = ecluse(&sibling, &["up"]);
    assert!(out.status.success(), "first up failed: {}", stderr(&out));

    // state.json should record the sibling path, not the default location.
    // Canonicalize both to handle macOS `/private/var` vs `/var` symlink.
    let sibling_canon = std::fs::canonicalize(&sibling).unwrap();
    let state_path = repo.path().join(".ecluse/state.json");
    let state: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&state_path).unwrap()).unwrap();
    let sessions = state["sessions"].as_array().unwrap();
    assert_eq!(sessions.len(), 1);
    let recorded_path = sessions[0]["worktree_path"].as_str().unwrap();
    assert_eq!(
        std::fs::canonicalize(recorded_path).unwrap(),
        sibling_canon,
        "first up should record the actual worktree location"
    );

    // Second `ecluse up` (the resume path) MUST honor the recorded path, not
    // recompute <root>/.ecluse/worktrees/<slug>. Before the fix, bring_up was
    // called with worktree_override=None and recomputed the default location,
    // failing the reuse_worktree existence check with
    // "worktree not found at <wrong path>". --force ensures the resume path
    // actually invokes bring_up (it would otherwise short-circuit when no
    // services need restarting).
    let out = ecluse(&sibling, &["up", "--force"]);
    assert!(
        out.status.success(),
        "resume failed (the bug): {}",
        stderr(&out)
    );

    // After resume, state must still point at the sibling path — the resume
    // must not silently rewrite the path either.
    let state: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&state_path).unwrap()).unwrap();
    let recorded_path = state["sessions"][0]["worktree_path"].as_str().unwrap();
    assert_eq!(
        std::fs::canonicalize(recorded_path).unwrap(),
        sibling_canon,
        "resume must preserve external worktree path"
    );

    // Clean up the sibling worktree.
    let _ = Command::new("git")
        .args(["worktree", "remove", "--force"])
        .arg(&sibling)
        .current_dir(repo.path())
        .output();
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

#[test]
fn sync_rejects_repo_root_as_worktree() {
    let repo = tmp_repo();
    ecluse(repo.path(), &["init", "--mode", "host", "--yes"]);
    // No worktree exists for this slug; running from the repo root must not
    // register the main checkout as the session's worktree.
    let out = ecluse(repo.path(), &["sync", "ghost-slug"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("ghost-slug"), "got: {}", stderr(&out));
}
