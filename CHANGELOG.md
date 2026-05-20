# Changelog

All notable changes to ecluse are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

---

## [Unreleased]

### Changed
- `ecluse ls` now shows all allocated ports in a `PORTS` column (`name=value` pairs, alphabetically sorted) instead of a single `PORT` value. Sessions with many services no longer require `ecluse env` to see the full port map.
- `ecluse ls` now shows a `TMUX` column with the tmux session name when at least one active session uses tmux as the process manager. The column is hidden entirely for repos where no session uses tmux (container mode, nohup, or no native services), keeping the table uncluttered.

---

## [0.2.6] — 2026-05-19

### Added
- `ecluse flush` — hard reset to clean state, regardless of what `state.json` says. Tears down all known sessions (via the same path as `ecluse shutdown`), kills orphaned tmux sessions named `ecluse-*`, stops orphaned Docker Compose projects matching `<prefix>_*`, removes all worktrees under `worktree_dir` with `git worktree remove --force`, wipes `.ecluse/pids/`, `.ecluse/logs/`, and `.ecluse/overlays/`, then resets `state.json` to an empty sessions list. Docker volumes are not removed. Steps 1–5 are best-effort; only the state reset is required. `--yes` skips the confirmation prompt for agent/CI use.

### Fixed
- Hardcoded `container_name` fields in `docker-compose.yml` are now overridden in the compose overlay with `<prefix>-<service>-<slot>` (e.g. `ecluse-postgres-2`). The previous fix set the field to `null`, which some Docker Compose versions do not honour — the original value survived the merge and caused container name conflicts between sessions. Using an explicit slot-scoped name guarantees uniqueness across all concurrent sessions and the main devenv.
- `detect` tests no longer fail in environments where Docker is not running. Seven tests asserted absolute scores (`> 0`) but the Docker-unavailable penalty (`-10`) in CI pushed scores negative. Tests now compare against an `empty_dir()` baseline so the delta assertions hold regardless of Docker availability.
- Compose overlay now unconditionally emits the port mapping for every `run = "docker"` service that has a `base_port`, even when the base compose file declares no `ports:` field. Previously, the overlay skipped the port entirely in this case — the container started with no host port published, `ECLUSE_*_PORT` was advertised but nothing was listening, and connections silently fell through to other services (e.g. the main-repo postgres).
- Compose overlay now uses the `ports: !override` YAML merge tag when the base compose file already declares `ports:` for a service. Docker Compose's default additive merge would otherwise publish both the base port (e.g. `5432:5432`) and the slot port (e.g. `5433:5432`), causing a bind failure when the base port is already in use by the main devenv.
- `ECLUSE_*_PORT` environment variables are now injected into the `docker compose up` child process. Compose files can reference `${ECLUSE_POSTGRES_PORT}` etc. directly for interpolation without needing the overlay to rewrite anything.

---

## [0.2.4] — 2026-05-18

### Added
- `ecluse sync <slug>` — register a manually-started environment with ecluse. Discovers running processes whose cwd is inside the worktree, matches them to services declared in `.ecluse.toml` by walking the process tree from the service's `command`, and registers the session in `state.json` (including PID files so `ecluse down` can kill those processes). Docker services in hybrid mode are detected via `docker ps`. If a session already exists for the slug, sync updates its port_overrides and PID tracking in place.
- `command` is now required for native services (`run = "native"`) in `.ecluse.toml`. Previously optional, it was always needed in practice — ecluse now enforces this at config validation time with a clear error message.

### Fixed
- Running any ecluse command from inside an ecluse-managed worktree now correctly resolves the main worktree root. Previously, `find_and_load` would walk the filesystem and find `.ecluse.toml` inside the worktree (which contains it as a committed file), treating the worktree as the project root and writing a stray `state.json` inside it. It now asks git for the main worktree root first via `git worktree list --porcelain`.

---

## [0.2.3] — 2026-05-18

### Added
- `--services <name>,<name>` flag on `ecluse up` — bring up only a subset of the services defined in `.ecluse.toml`. Unknown service names are rejected before any worktree is created. The subset is stored in session state and surfaced in `ecluse env` output.

### Fixed
- Compose overlay now clears hardcoded `container_name` fields by setting them to `null`, preventing Docker name conflicts when an ecluse session runs alongside the main dev environment.

---

## [0.2.2] — 2026-05-15

### Fixed
- `--branch` flag now validates the value to block git option injection and malformed refspecs
- Compose file paths that escape the repo root are now rejected, preventing path traversal
- Removed `set_var(HOME)` call in process config tests that introduced a race condition under parallel test execution

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
