# ecluse — CLAUDE.md

## What this is

ecluse is a Rust CLI that gives each git worktree its own isolated slot: unique ports, isolated services, zero collisions between parallel sessions. Built for coding agents running tasks in parallel on the same repo.

The core abstraction is a **slot** (integer 1–max_slots). Every resource derives from it: `port = base_port + slot`, volume name, compose project name, branch name.

Three **modes** determine what gets isolated:
- `host` — app runs natively, ports only
- `hybrid` — app runs natively, data services (Postgres, Redis) run in containers
- `container` — everything runs in containers

## Commands

```
ecluse init       # detect mode, write .ecluse.toml
ecluse up         # create worktree + allocate slot + bring up services
ecluse down       # teardown session, free slot, remove worktree
ecluse shutdown   # tear down ALL active sessions (--keep-worktrees to leave worktrees on disk)
ecluse ls         # list active sessions
ecluse env        # print session env vars as JSON
ecluse shell      # drop into worktree with env loaded (interactive use only)
ecluse validate   # validate .ecluse.toml port ranges and service gaps
```

## Dev workflow

```bash
cargo fmt --check && cargo clippy -- -D warnings && cargo test   # run before pushing; CI enforces all three
cargo build
cargo test
cargo clippy -- -D warnings
cargo fmt
```

Run a specific test module:
```bash
cargo test slot::tests
cargo test config::tests
cargo test env::tests
cargo test detect::tests
```

Check coverage (must stay ≥ 95%):
```bash
cargo llvm-cov --summary-only   # requires cargo-llvm-cov: cargo install cargo-llvm-cov
```

## After every change — check what needs updating

Changes to behavior, config schema, CLI flags, env vars, or examples require updating the affected docs. Ask yourself which of these apply:

| File / directory | Update when… |
|---|---|
| `README.md` | User-facing behavior changes: new command, new flag, new install step |
| `CONTRIBUTING.md` | Dev workflow changes: new toolchain requirement, new test command, new step to add a mode |
| `examples/` | A `.ecluse.toml` field or default changes — update every affected example repo |
| `skills/ecluse/SKILL.md` | Any agent-visible behavior: new env var, new flag, new failure mode, config schema change |
| `skills/ecluse/examples/` | Example configs shown to agents diverge from the new behavior |

Not every change touches all five — a slot-allocation bug fix probably touches none of them. A new `[[services]]` field touches all of them. Use judgment; the table is a checklist, not a mandate.

## Project structure

```
src/
├── main.rs       command handlers (init/up/down/ls/shell/env)
├── cli.rs        clap CLI definitions
├── config.rs     .ecluse.toml parsing, Config struct, Mode enum
├── slot.rs       slot allocation (first free in 1..max_slots)
├── env.rs        .env.ecluse generation from slot + config
├── state.rs      state.json persistence with file locking (StateGuard)
├── error.rs      EcluseError variants with actionable messages
├── detect.rs     mode auto-detection via signal scoring
├── worktree.rs   git worktree create/remove wrappers
├── hooks.rs      on_up/on_down lifecycle hook execution
├── compose.rs    docker-compose.yml parsing + overlay generation
├── docker.rs     Docker CLI wrappers
└── modes/        ModeHandler trait + container/host/hybrid impls
```

All tests are inline in source modules using `tempfile::TempDir` for isolation.

## Key invariants — never break these

- **State is always consistent**: `state.json` is written atomically (tmp → rename) under an exclusive file lock (`state.lock`). Never write state without `StateGuard`.
- **Rollback on failure**: if `ecluse up` fails partway through, it must clean up whatever was created (worktree, containers, slot allocation).
- **Mode is per-repo, not per-session**: `mode` lives in `.ecluse.toml`, not in state. All sessions in a repo share the same mode.
- **Same CLI surface across modes**: no mode-specific flags on `up`/`down`.
- **Error messages must be actionable**: pattern is `"<what happened>; <what to do next>"`.

## Adding a new isolation mode

1. Open an issue first.
2. Add a variant to `config::Mode` and `error::EcluseError` as needed.
3. Implement `ModeHandler` in a new file under `src/modes/`.
4. Register it in `modes::get_handler`.
5. Add detection signals in `detect.rs`.
6. Add unit tests alongside the new code.

## Releasing a new version

1. Bump `version` in `Cargo.toml`.
2. Add a `## [x.y.z] — YYYY-MM-DD` section to `CHANGELOG.md` with `Added`, `Changed`, and `Fixed` entries covering everything since the last release.
3. Commit, tag (`git tag vx.y.z`), and push — **including tags**: `git push && git push --tags`. Pushing the tag triggers CI to build cross-platform binaries and update the Homebrew tap formula automatically.

The changelog entry is required — do not skip it when publishing a new version.

## Pull requests

- One logical change per commit.
- For bug fixes: a failing test that passes after the fix is ideal.
- For new features: unit tests are required, integration tests are a bonus.
