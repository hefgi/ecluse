# Changelog

All notable changes to ecluse are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

---

## [0.2.1] — 2026-05-14

### Added
- `ecluse shutdown` — tears down all active sessions in one shot; best-effort (continues through failures, reports all errors at the end)
- `--keep-worktrees` flag on `shutdown` — releases services and slots but leaves git worktrees on disk
- `--keep-volumes` flag on `shutdown` — skips `-v` on docker compose down, mirroring `down`'s behaviour

### Fixed
- `bring_down` no longer silently drops port-env mapping errors
- Containers are now torn down correctly when worktree creation fails during `up` (rollback path)
- Errors from state writes and worktree removal are surfaced instead of swallowed
- Port range arithmetic uses saturating ops to prevent overflow panics on edge-case configs
- `--keep-branch` on `down` now emits a deprecation warning instead of being silently ignored

---

## [0.2.0] — 2026-05-14

### Added
- `[[services]]` port model — each service declares its own `base_port`; slot port is `base_port + slot`; replaces the old flat `[ports]` table
- `port_env` field on `[[services]]` — override the env var name that receives the allocated port
- `command` field on `[[services]]` — shell command ecluse runs to start the service (host/hybrid modes)
- Native process management — tmux and nohup backends; auto-detected via `ecluse init`; stored in `~/.config/ecluse/config.toml`
- `ecluse validate` — checks port ranges, service gaps, search range safety, and process manager availability
- `strict_port` and `port_search_range` config fields — controls whether ecluse bumps ports on conflict or hard-fails
- `--keep-worktree` flag on `down` — tears down services but leaves the worktree on disk
- `--reuse-worktree` flag on `up` — skips worktree creation if the path already exists
- `--port NAME=VALUE` flag on `up` (repeatable) — override a service's allocated port for this session
- `--quiet` flag on `init`, `up`, `down`, `validate`
- Step logger (`StepLogger`) — TTY-aware `» step / → detail` output across all commands
- `pre_up` / `post_up` / `pre_down` / `post_down` lifecycle hooks
- Per-service `compose` field — point individual services at their own compose file (monorepo support)
- CI/CD pipeline with GitHub Actions and automated Homebrew tap updates
- Comprehensive unit test suite across all modules (slot, config, env, detect, compose, state, hooks, worktree, modes)
- Example configs for 9 common stacks (Rails, Next.js, T3, Django, FastAPI, k3d, …)
- Agent skill (`skills/ecluse/SKILL.md`) for use with Claude Code and other coding agents
- mdBook documentation site with `llms.txt` support

### Changed
- `[[services]]` replaces the `[ports]` table; old configs must migrate (one `[[services]]` block per service)
- `base_port` / `stride` removed from top-level config; port spacing is now per-service
- Environment variable generation rewritten around the `[[services]]` model

### Fixed
- App port defaults to 3000 (was incorrectly 0 in some modes)
- Compose overlay used `.keys()` iteration to avoid borrow conflicts

---

## [0.1.0] — 2026-05-13

Initial public release.

### Added
- Three isolation modes: `container`, `host`, `hybrid`
- Core commands: `init`, `up`, `down`, `ls`, `shell`, `env`
- Slot allocation (first-free integer in 1–max_slots) with file-locked `state.json`
- Mode auto-detection via 20 scored signals (Dockerfile, compose files, language toolchains, …)
- Git worktree creation and removal (`git worktree add/remove --force`)
- Docker Compose overlay generation — per-slot port bindings injected without touching the original file
- `.env.ecluse` written into each worktree with all allocated ports and paths
- `EcluseError` — actionable error messages following the `"what happened; what to do"` pattern
- `ecluse shell` — drops into a worktree with its env loaded
- Config schema: `mode`, `max_slots`, `prefix`, `worktree_dir`, `app_label`, `app_label_value`
- Initial integration test suite
