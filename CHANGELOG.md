# Changelog

All notable changes to ecluse are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

---

## [Unreleased]

### Added
- `ecluse status`: port-allocation-only services (no `command`) now show `—` in the STATUS column instead of `✗ down`. They are not counted in the "N services down" summary, do not trigger exit code 1, and have `"managed": false` in JSON output. This prevents agents from treating unmanaged services as failures.
- tmux windows now source `.env`, `.env.local`, and `.env.ecluse` before running the service command. Manual restarts inside a tmux window (`↑ Enter`) automatically have the correct environment without needing `source .env.ecluse &&` in every `command` field. Load order: `.env` → `.env.local` → `.env.ecluse` (ecluse slot vars win on overlap).
- nohup-spawned services now receive vars from `.env`, `.env.local`, and `.env.ecluse` merged into their environment at spawn time, consistent with tmux behaviour.

### Fixed
- `ecluse up` with no argument now fails fast with an actionable error when stdin is not a terminal (CI, agents, piped shells), instead of blocking on a branch-name prompt until killed.
- `ecluse down` and `ecluse shutdown` pre/post hook failures now emit a warning and continue teardown instead of aborting. Teardown must always complete regardless of hook exit codes.
- Error messages when no `.ecluse.toml` is found or no active session exists are now context-aware: the hint differs depending on whether the cwd is a linked git worktree, the repo root, or unrelated to any known session.

---

## [0.2.13] — 2026-06-02

### Added
- `extra_ports` field on `[[services]]` blocks: a list of additional per-slot port allocations, each with `base_port` and `port_env`. The env var is set to `base_port + slot` in the process environment. For docker services the port is also published as a host→container binding in the compose overlay and injected into `compose_env` so compose files can interpolate it (e.g. `${PGPORT}`). This is the generic replacement for `debug_port` — use it for debugger ports (Node.js `--inspect`, Delve, debugpy, pprof), auxiliary listeners, or any secondary port a service exposes.
- `debug_port` is now deprecated. It continues to work — existing configs are unchanged — but `ecluse validate` emits a warning and `extra_ports` should be used for new configs.

### Fixed
- `ecluse down` (hybrid mode) now always stops Docker containers even when no overlay file paths are recorded in session state. Previously, if `overlay_file` was absent from state, `docker compose down` was never called and containers kept running silently.
- `ecluse up` from inside a non-ecluse git worktree (e.g. a sibling path, not under `.ecluse/worktrees/`) now correctly uses the actual worktree directory instead of computing a path under `worktree_dir`. Previously the computed path didn't exist and `ecluse up` failed with a "worktree not found" error.
- `ecluse status`, `ecluse ls`, `ecluse env`, and `ecluse shell` now acquire a shared (read-only) lock instead of an exclusive lock. These commands no longer time out with "another ecluse process may be running" when a long-running `ecluse up` holds the exclusive lock.
- Port collision detection now checks `docker ps` host-port bindings in addition to `lsof`. Docker containers claim host ports before they start listening, so an `lsof`-only check could pick a port already reserved by a container. Both checks are best-effort: if Docker is unavailable, the check is skipped and never blocks.
- `ecluse down` and `ecluse shutdown` no longer block indefinitely when run non-interactively (CI, Claude Code Bash tool, piped shells). The worktree-removal prompt now detects non-interactive stdin and returns immediately with an actionable error instead of hanging until SIGKILL. Pass `--delete-worktree` / `-y` or `--keep-worktree` to skip the prompt.

---

## [0.2.12] — 2026-06-02

### Changed
- `command` is now optional for native services. Omitting it puts the service in **port-allocation-only mode**: ecluse allocates the port and injects all `ECLUSE_*` env vars, but does not spawn or manage the process — start it yourself via a task runner, `post_up` hook, or any other means. `ecluse validate` emits a warning (not an error) when `command` is absent.

### Fixed
- `ecluse down` (hybrid mode) now always stops Docker containers even when no overlay file paths are recorded in session state. Previously, if `overlay_file` was absent from state, `docker compose down` was never called and containers kept running silently.

---

## [0.2.11] — 2026-06-02

### Added
- `inherit_env` config field (default: `[".env", ".env.local"]`) — files listed here are symlinked from the main worktree root into each new worktree at `ecluse up` time. Symlinks keep worktrees in sync with root changes automatically. Set to `[]` to opt out. Pass `--no-inherit-env` to skip for a single `up` call (for CI/agents).
- `ecluse shutdown` now prints a `ecluse flush` hint when any session teardown fails, making recovery more discoverable.

### Fixed
- Hardcoded credentials removed from the `node-container` example compose file.

---

## [0.2.10] — 2026-06-01

### Added
- `inherit_env` config field (default: `[".env", ".env.local"]`): files listed here are symlinked from the main worktree root into each new worktree at `ecluse up` time. Symlinks keep worktrees in sync with root changes automatically — no stale copies. Set to `[]` to opt out. If a listed file already exists in the worktree, ecluse prompts to skip or overwrite. Add `--no-inherit-env` to `ecluse up` to skip entirely (for CI/agents).
- `debug_port` field on `[[services]]` blocks: secondary port for debuggers or auxiliary servers. ecluse computes `debug_port + slot` and exposes it as `ECLUSE_<NAME>_DEBUG_PORT`. Use when a service exposes a second listener (Node.js `--inspect`, Delve, debugpy, pprof, etc.) that defaults to a hardcoded port and would collide across parallel sessions.
- `ecluse up` now accepts branch names with slashes directly: `ecluse up feat/add-auth` → slug `feat-add-auth`, branch `feat/add-auth`. The `--branch` flag is removed — branch comes from the argument itself.
- `ecluse up` with no argument uses the git worktree location to determine intent: inside an ecluse-registered worktree → reuse stored slug; inside any other git worktree → auto-detect branch from cwd and register the worktree (no `--reuse-worktree` flag needed); in the main worktree / repo root → prompt for a branch name. Detached HEAD exits with a clear error.
- Worktree deletion guard: `ecluse down` and `ecluse shutdown` now always prompt before removing a worktree (stop / keep / delete). A ⚠ warning is shown if the worktree has uncommitted changes. Pass `--delete-worktree` / `--delete-worktrees` to skip the prompt and delete (for CI/agents), or `--keep-worktree` / `--keep-worktrees` to skip and keep.

### Changed
- `ecluse status` session header is now a borderless key-value block (`Slug`, `Slot`, `Worktree`, `Tmux`) with right-aligned labels, replacing the single run-on line. Labels use the user-facing term "Slug" (not "Session").
- `ecluse status` last column adapts to the session's process manager: `WINDOW` (tmux window name) for tmux sessions, `PID` for nohup, omitted entirely for container-only sessions.
- `ecluse status` native service health check for tmux sessions now verifies that a descendant of the tmux pane's shell process owns the expected port, rather than probing the port directly. This correctly handles port collisions with unrelated processes and services that fail to start despite their tmux window existing.

### Fixed
- `docker stop` calls in `--force` now use `DOCKER_HOST` from the active Docker context, so OrbStack containers are correctly targeted.
- tmux session spawning no longer fails with "duplicate session" when the previous session's shell is still alive after services crash. The stale session is killed before a new one is created.

---

## [0.2.9] — 2026-06-01

### Fixed
- `ecluse ls` table no longer wraps when a session has many services. The PORTS column is truncated to 40 characters with a `…` suffix — use `ecluse env <slug>` or `ecluse ls --json` to see all ports.

---

## [0.2.8] — 2026-05-29

### Added
- `ecluse up` is now idempotent. Running `ecluse up <slug>` when a session already exists resumes it: worktree and slot are reused, only services that are not running are started. Each service decision is logged explicitly ("already running — skipped" / "down — will start").
- All slug-accepting commands now auto-detect the slug from cwd. When run from inside a worktree, the slug is inferred automatically. Applies to `up`, `down`, `shell`, `env`, `status`, and `sync`.
- `--force` flag on `ecluse up`: kills all running services on the session's allocated ports before starting them fresh. Useful when processes are in a bad state. Combinable with `--skip`.
- `--skip <name>,<name>` flag on `ecluse up`: excludes named services from being started (or killed, with `--force`). Multiple values comma-separated. Combinable with `--force` and `--services`.

---

## [0.2.7] — 2026-05-28

### Added
- `ecluse status [<slug>] [--json] [--quiet]` — per-service health check. Shows a ✓/✗ indicator, port, type (native/docker), and PID for each service. For native services, matches running processes in the worktree by command line; for docker services, queries `docker ps` by container name. Exits with code 1 if any service is down, making it useful in agent scripts and CI pre-flight checks. Slug is auto-detected from cwd when omitted.

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
