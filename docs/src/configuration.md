# Configuration

`.ecluse.toml` lives at repo root and is written by `ecluse init`. All fields are optional except `mode`.

```toml
mode = "hybrid"
max_slots = 8
prefix = "ecluse"
worktree_dir = ".ecluse/worktrees"

# Port collision handling (both optional)
# strict_port = false        # default: search for a free port on collision
# port_search_range = 10     # how many alternatives to try (bump by max_slots each time)

# One [[services]] block per service. port = base_port + slot.
# Native services run on the host; docker services run in containers.
# The first native entry also sets the PORT alias for framework compatibility.
# command is optional for native services — omit to use port-allocation-only mode
# (ecluse allocates the port and injects env vars; you start the process yourself).
# When set, ecluse spawns it on up and uses it to identify the process during ecluse sync.

[[services]]
name = "api"
base_port = 3000             # slot 1 → ECLUSE_API_PORT=3001 + PORT, slot 2 → 3002
command = "npm run dev"      # optional — omit for port-allocation-only
# port_env = "DJANGO_PORT"  # optional — also set DJANGO_PORT to the allocated port
# port_env = ["DJANGO_PORT", "APP_PORT"]  # or multiple aliases

[[services]]
name = "postgres"
run = "docker"
base_port = 5432             # slot 1 → ECLUSE_POSTGRES_PORT=5433, slot 2 → 5434

# Env file inheritance — files symlinked from repo root into each worktree
# Default: [".env", ".env.local"]. Set to [] to opt out.
# inherit_env = [".env", ".env.local"]

# Optional: lifecycle hooks (see Hooks page for full details)
[hooks]
post_up  = "npx prisma migrate deploy"
pre_down = "npx prisma migrate reset --force"
```

## Fields

| Field | Type | Default | Description |
|---|---|---|---|
| `mode` | string | — | `container`, `host`, or `hybrid` |
| `max_slots` | integer | `8` | Maximum parallel sessions |
| `prefix` | string | `"ecluse"` | Prefix for compose project names and volume names |
| `worktree_dir` | string | `".ecluse/worktrees"` | Directory for git worktrees |
| `strict_port` | bool | `false` | Fail immediately on port collision instead of searching |
| `port_search_range` | integer | `10` | How many alternatives to try on collision |
| `app_label` | string | `"ecluse.role"` | Docker Compose label key used to identify app vs data services in hybrid mode |
| `app_label_value` | string | `"app"` | Value of `app_label` that marks a service as the app (not a data service) |
| `inherit_env` | array | `[".env", ".env.local"]` | Files to symlink from the repo root into each new worktree at `ecluse up` time. Symlinks keep worktrees in sync with root changes automatically. Set to `[]` to opt out. |

## `[[services]]`

Each `[[services]]` block defines one service. Each gets a stable, collision-free port per slot (`base_port + slot`).

| Field | Type | Description |
|---|---|---|
| `name` | string | Service name — becomes `ECLUSE_<NAME>_PORT` |
| `base_port` | integer | Port formula: `base_port + slot` |
| `run` | string | `"docker"` to run in a container; omit for native |
| `command` | string | Shell command ecluse spawns on `ecluse up` and uses to identify the process during `ecluse sync`. Managed by your global `process_manager` setting. Omit to use **port-allocation-only mode** — ecluse allocates the port and injects env vars; you start the process yourself (e.g. via a task runner). |
| `port_env` | string or array | Extra env var names to set to this service's allocated port — accepts a single string or an array |
| `debug_port` | integer | Secondary port for debuggers or auxiliary servers: `debug_port + slot` → `ECLUSE_<NAME>_DEBUG_PORT`. Use when a service exposes a second listener (Node.js `--inspect`, Delve, debugpy, pprof, etc.) and multiple parallel sessions would collide on a hardcoded default port. |

The first native (non-docker) service entry also sets `PORT` for framework compatibility.

Omit `[[services]]` entirely for single-service projects — ecluse falls back to `PORT = 3000 + slot`.

## Global config (`~/.config/ecluse/config.toml`)

Controls how ecluse spawns native service processes. Written by `ecluse init` based on what's installed.

```toml
process_manager = "tmux"   # "tmux" | "nohup" | "none"
```

| Value | Behaviour |
|---|---|
| `tmux` | Creates a detached tmux session `ecluse-<slug>` with one window per service. `ecluse shell <slug>` attaches to it. |
| `nohup` | Spawns each service as a background process. Logs at `.ecluse/logs/<slug>/`, PIDs at `.ecluse/pids/<slug>/`. |
| `none` | Spawns nothing — current behaviour before this was added. |

`ecluse init` sets this automatically: tmux if available, otherwise nohup. Change it at any time — the setting is per-machine, not per-repo.

Run `ecluse validate` to check the configured binary is installed.

## `[hooks]`

| Field | When it runs | Env vars |
|---|---|---|
| `pre_up` | Before any infrastructure is created | None (runs from repo root) |
| `post_up` | After all services are up and `.env.ecluse` is written | All `ECLUSE_*` + `PORT` |
| `pre_down` | Before services are stopped | All `ECLUSE_*` + `PORT` |
| `post_down` | After all services are stopped and worktree is removed | All `ECLUSE_*` + `PORT` |

Use `post_up` for migrations and seeding, `pre_down` for teardown that needs a live database. See the [Hooks](hooks.md) page for full details and examples.
