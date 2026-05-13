# Contributing to ecluse — Agent Guide

This file is for coding agents (Claude Code, Cursor, Codex, etc.) contributing to the ecluse codebase itself.

## Project layout

```
ecluse/
├── src/
│   ├── main.rs          command dispatch
│   ├── cli.rs           clap structs
│   ├── config.rs        .ecluse.toml schema
│   ├── state.rs         state.json + file lock
│   ├── slot.rs          slot allocator
│   ├── worktree.rs      git worktree wrapper
│   ├── env.rs           .env.ecluse generation
│   ├── compose.rs       compose parse + overlay generation
│   ├── docker.rs        docker shell-outs
│   ├── postgres.rs      psql shell-outs
│   ├── detect.rs        mode detection signals
│   ├── skills.rs        embedded skill registry (points to skills/)
│   ├── error.rs         EcluseError enum
│   └── modes/           container / host / hybrid handlers
├── skills/ecluse/       canonical skill sources (SKILL.md per subfolder)
└── .claude-plugin/      agent harness plugin manifest
```

## Build

```bash
cargo build          # dev build
cargo build --release  # release (strip + lto + opt-z)
cargo clippy -- -D warnings
cargo fmt --check
```

The `rust-toolchain.toml` pins to stable. Do not add nightly features.

## Key invariants — do not break these

- **State is always consistent.** `state.json` is written atomically (write `.json.tmp`, then rename). The lock at `state.lock` is held for the entire duration of `up` and `down`.
- **Mode is per-repo, not per-session.** Stored in `.ecluse.toml`. `up` never reads `--mode` from CLI.
- **Rollback on failure.** Every `bring_up` in `modes/*.rs` must roll back any partially-created resources (worktree, containers, database) if it returns an error before `state_guard.commit()`.
- **Same CLI surface across all modes.** No mode-specific subcommands or flags. If you find yourself adding `--container-only-flag`, stop and reconsider.
- **LoC budget: 2500 lines of Rust** (`src/**/*.rs`). Check with `find src -name '*.rs' | xargs wc -l`.

## Skills are in `skills/`, not `src/`

The canonical skill content lives in `skills/ecluse/SKILL.md` — one consolidated skill covering everything. Edit that file directly; there is no build step for skills.

## Error messages

Every error message must tell the user what to do next. Pattern:

```
error: <what happened>; <what to do>
```

Example: `port 3100 is already in use by PID 12345; stop that process first`

Agents parse these messages to decide their next action. Vague messages waste tokens and cause retry loops.

## Commit style

```
feat(scope): short description
fix(scope): short description
docs: short description
chore: short description
```

One logical change per commit. See git log for examples.
