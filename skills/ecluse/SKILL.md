---
name: ecluse
description: >
  Complete reference for ecluse — ephemeral local environments for coding agents,
  any stack. Use this skill whenever ecluse is mentioned, a .ecluse.toml file is
  present in the repo, the user asks about worktree isolation, parallel dev
  environments, or port/database conflicts between branches. If you are a coding
  agent about to do substantive work in a repo that has .ecluse.toml, use this
  skill automatically before starting — do not wait to be asked.
tags:
  - ecluse
  - worktree
  - isolation
  - docker
  - compose
  - postgres
---

# ecluse

Ephemeral local environments for coding agents — any stack. Each git worktree gets its own slot — isolated ports, isolated services, isolated data. Works whether your stack runs in Docker, on the host, or a mix. No collisions, clean teardown.

Each `ecluse up` allocates a **slot** — an integer that drives every isolated resource: port offset, database name, Docker volume names, and git worktree. Nothing leaks between sessions.

## Quick navigation

- **New to ecluse?** → [Getting Started](#getting-started)
- **Coding agent in a repo with .ecluse.toml?** → [Agent Workflow](#agent-workflow) — use this now
- **Which mode to pick?** → [Choosing a Mode](#choosing-a-mode)
- **Container mode details** → [Container Mode](#container-mode)
- **Host mode details** → [Host Mode](#host-mode)
- **Hybrid mode details** → [Hybrid Mode](#hybrid-mode)
- **Config templates for your stack?** → [examples.md](examples.md) — 5 canonical examples
- **Something broken?** → [Troubleshooting](#troubleshooting)
- **Feature not supported?** → [Limits](#limits)

---

## Getting Started

### Prerequisites

- macOS 14+ or Linux (WSL2 works but untested)
- Git repo with at least one commit
- Docker/OrbStack for `container` or `hybrid` mode; nothing extra for `host`

### Install

```bash
brew install ecluse/tap/ecluse
ecluse --version
```

Or from source: `cargo install ecluse`

### Agent quick-start

```bash
ecluse init --mode hybrid --yes          # write .ecluse.toml non-interactively
ecluse up feat-foo --json                # worktree + slot + all env vars in one JSON call
# → use worktree_path and env from JSON to run commands and edit files
ecluse ls                                # see active sessions
ecluse down feat-foo                     # clean teardown
```

Note: `ecluse shell` spawns an interactive subshell — agents cannot use it. Use `ecluse up --json` or `ecluse env <slug>` to get the worktree path and env, then operate directly.

### What `ecluse up` does

1. Allocates a slot (integer 1–N)
2. Creates a git worktree at `.ecluse/worktrees/<slug>` on branch `ecluse/<slug>`
3. Depending on mode: starts containers, writes `.env.ecluse`, runs `on_up` hook if configured
4. `on_up` runs in the worktree with all env vars set — use it for migrations, seeding, etc.

### Common first-time failures

- **"run `ecluse init` first"** — no `.ecluse.toml` found; run `ecluse init` from repo root
- **"all N slots in use"** — `ecluse ls` then `ecluse down <slug>` to free one
- **Docker not running** — `open -a OrbStack` or `open -a Docker`

---

## Agent Workflow

You're in a repo with `.ecluse.toml`. Use ecluse. Every task gets its own isolated slot — no port collisions, clean teardown when done.

### The canonical loop

```bash
# 1. Create session — get everything you need in one JSON call
ecluse up <slug> --json
# Returns:
# {
#   "slug": "feat-auth",
#   "slot": 1,
#   "mode": "hybrid",
#   "branch": "ecluse/feat-auth",
#   "worktree_path": "/path/to/.ecluse/worktrees/feat-auth",
#   "env_file": "/path/to/.ecluse/worktrees/feat-auth/.env.ecluse",
#   "env": {
#     "PORT": "3001",                  ← alias for first native [[services]] entry
#     "ECLUSE_API_PORT": "3001",       ← if [[services]] name="api" base_port=3000
#     "ECLUSE_POSTGRES_PORT": "5433",  ← if [[services]] name="postgres" base_port=5432 run="docker"
#     "ECLUSE_SLOT": "1",
#     "ECLUSE_SLUG": "feat-auth",
#     "ECLUSE_MODE": "hybrid",
#     ...
#   }
# }

# 2. Work in the worktree — edit files at worktree_path
# Use env vars from the JSON for config, curl endpoints, running commands

# 3. Run commands in the worktree with env loaded
cd <worktree_path> && PORT=<port> npm test
# or source the env file before running commands:
cd <worktree_path> && source .env.ecluse && npm test

# 4. Tear down
ecluse down <slug>
```

### Query an existing session anytime

```bash
ecluse env <slug>   # same JSON as up --json: worktree_path + all env vars
ecluse env          # auto-detects current session if run from inside a worktree
```

### Environment variables

All vars are in the JSON from `ecluse up --json` or `ecluse env <slug>`.

| Variable | Example | Description |
|---|---|---|
| `PORT` | `3001` | Alias for the first native `[[services]]` entry — never hardcode 3000 |
| `ECLUSE_<NAME>_PORT` | `ECLUSE_API_PORT=3001` | Per-service port: `base_port + slot` |
| `ECLUSE_SLOT` | `1` | Slot number |
| `ECLUSE_SLUG` | `feat-auth` | Session slug |
| `ECLUSE_MODE` | `hybrid` | `container`, `host`, or `hybrid` |

**No `DATABASE_URL` or `REDIS_URL` are set automatically.** Data service ports are exposed as `ECLUSE_<SERVICE>_PORT` (e.g. `ECLUSE_POSTGRES_PORT=5433`). Construct connection strings in your `on_up` hook or app config using that port. This keeps ecluse engine-agnostic.

### Parallel sessions

```bash
ecluse up feat-auth --json   # slot 1 → ECLUSE_API_PORT=3001, ECLUSE_POSTGRES_PORT=5433
ecluse up feat-cache --json  # slot 2 → ECLUSE_API_PORT=3002, ECLUSE_POSTGRES_PORT=5434
ecluse ls                    # see both
```

Each session is a separate git branch and worktree. They don't interfere.

### Common failures

- **"session already exists"** — use a different slug or `ecluse down <slug>` first
- **"all slots in use"** — `ecluse ls` to find stale sessions, `ecluse down` the oldest
- **Port in use** — `lsof -iTCP:<port> -sTCP:LISTEN` to find the blocker

---

## Choosing a Mode

| Mode | What runs in containers | What runs on host | Best for |
|---|---|---|---|
| `container` | Everything — app + all services | Nothing | `docker compose up` is the team's primary dev command |
| `host` | Nothing | Everything | Native-only stacks; no Docker |
| `hybrid` | Data services only (postgres, redis, etc.) | App | Compose data plane, app runs natively for speed |

### Decision guide

**`container`** — your `docker-compose.yml` has `build: .` on the app service and the team's day-one command is `docker compose up`.

**`host`** — no compose file at all; dev command is `npm run dev`, `bin/rails server`, etc. Uses `mise`, `asdf`, `rbenv`, or similar. Docker absent or too heavy.

**`hybrid`** — compose has only data services (postgres, redis); the README says "run `docker compose up -d`, then `bin/dev`". You want per-session database isolation with native app speed. **This is the most common choice for Rails, Django, and Node.js apps.**

### Auto-detection

```bash
ecluse init             # detect, prompt to confirm
ecluse init --explain   # show full signal score breakdown
ecluse init --mode hybrid   # skip detection, force mode
```

Detection runs 20 signals. Key ones:

| Signal | container | host | hybrid |
|---|---|---|---|
| Compose has `build: .` | +3 | 0 | 0 |
| All compose services are data images | −2 | 0 | +5 |
| Service labeled `ecluse.role: app` | 0 | 0 | +10 |
| No compose file | −5 | +4 | −5 |
| `bin/dev` exists | 0 | +3 | +2 |
| README: `docker compose up` then `bin/dev` | 0 | 0 | +3 |
| Docker not installed | −10 | 0 | −10 |

Confidence: gap ≥ 4 = High (auto-accept), 2–3 = Medium, 0–1 = Low (full breakdown shown), all ≤ 0 = `--mode` required.

### Changing modes later

`ecluse init --mode <new>` overwrites `.ecluse.toml`. Existing sessions keep their stored mode; `ecluse down` still works for them.

### Edge cases

- **Nix flake** — use `nix develop`; ecluse doesn't understand `flake.nix`
- **Bazel** — use Bazel's native sandbox
- **Monorepo, single compose at root** — the common case; use `[[services]]` to allocate one port per native service (see `t3-monorepo` example)
- **Monorepo, each service has its own compose file** — ecluse only reads one compose file per repo root; run `ecluse init` inside each service subdirectory, giving each its own `.ecluse.toml`, slot pool, and state; the agent must `cd` into the right subdirectory before running ecluse commands

---

## Container Mode

Every service — including the app — runs in Docker under a unique compose project per session. Ports are offset; volumes are namespaced.

### Prerequisites

- Docker/OrbStack running
- `docker-compose.yml` with `build: .` on the app service

### Port allocation

Ports are computed as `base_port + slot`. With `[[services]] name="web" base_port=3000` and `[[services]] name="postgres" run="docker" base_port=5432`:

| Session | Slot | web → host | postgres → host |
|---|---|---|---|
| `feat-foo` | 1 | 3001 | 5433 |
| `fix-bar` | 2 | 3002 | 5434 |

Only the host-side port changes. Container-internal ports stay the same.

### Volume namespacing

Named volume `db_data` becomes `db_data_ecluse_feat-foo` for slot 1. Bind mounts are unchanged.

### How it works

ecluse writes `.ecluse/overlays/<slug>.yml` — a compose override that rewrites ports and volume names. Your `docker-compose.yml` is never modified. Merged at runtime via `docker compose -f docker-compose.yml -f .ecluse/overlays/<slug>.yml`.

### Workflow

```bash
ecluse up feat-foo               # starts all containers
# app at http://localhost:3100
ecluse down feat-foo             # stops containers, removes worktree + volumes
ecluse down feat-foo --keep-volumes   # keeps volumes
```

### Common pitfalls

- **Hardcoded `localhost:3000` in app code** — read from `$PORT` and `$ECLUSE_<SERVICE>_PORT` instead
- **`--watch` requires Compose v2.22+** — pass `ecluse up --watch`
- **Invoking `docker compose` directly** — the overlay won't be included; use `ecluse up` or add `-f .ecluse/overlays/<slug>.yml`

---

## Host Mode

No containers. ecluse reserves a port range, writes `.env.ecluse`, creates the worktree, runs `on_up`. If `command` is set on a `[[services]]` entry, ecluse spawns it automatically via your global `process_manager` (tmux or nohup). Otherwise you start your own dev server.

### Prerequisites

- No Docker required
- Host Postgres/MySQL/SQLite already available if your app needs a database

### Workflow

```bash
ecluse up feat-foo
# Session:   feat-foo (slot 1)
# App port:  3001  (ECLUSE_APP_PORT + PORT alias, from [[services]] name="app" base_port=3000)
# If command = "npm run dev" is set, the dev server is already running.
# Otherwise: cd .ecluse/worktrees/feat-foo && source .env.ecluse

cd .ecluse/worktrees/feat-foo
source .env.ecluse
npm run dev    # reads $PORT; run migrations via [hooks] on_up
```

### Database in host mode

ecluse does not provision databases. Use `[hooks] on_up` with your app's own tooling:

```toml
[[services]]
name = "app"
base_port = 3000

[hooks]
on_up = "npx prisma migrate deploy"
on_down = "npx prisma migrate reset --force"
```

Your app's connection string is defined in your `.env` (not managed by ecluse). The hook runs inside the worktree with all ecluse env vars set.

### Teardown

```bash
ecluse down feat-foo   # runs on_down hook, removes worktree
```

### Common failures

- **"Port 3100 is in use by PID 12345"** — `kill 12345`
- **App can't find database** — `source .env.ecluse` before starting; check hook output for migration errors

---

## Hybrid Mode

Data services (postgres, redis, etc.) run in containers with offset ports and namespaced volumes. App runs on the host. If `command` is set on the native `[[services]]` entry, ecluse spawns it automatically — no manual `npm run dev` required.

This is the fastest dev loop: isolated data, native app speed, hot reload, native debugger.

### Prerequisites

- Docker/OrbStack running
- `docker-compose.yml` with data services

### Label your app service

Add `ecluse.role: app` to any service that should **not** start in a container:

```yaml
services:
  web:
    build: .
    labels:
      ecluse.role: app       # skip this in containers; run it yourself
    ports: ["3000:3000"]
  postgres:
    image: postgres:16       # no label = data service = containerized with offset port
  redis:
    image: redis:7
```

### Workflow

```bash
ecluse up feat-foo
# postgres and redis start in Docker with per-slot ports
# ECLUSE_POSTGRES_PORT=5433, ECLUSE_REDIS_PORT=6380  (base_port + slot 1)
# ECLUSE_API_PORT=3001, PORT=3001  (from [[services]] name="api" base_port=3000)

cd .ecluse/worktrees/feat-foo
source .env.ecluse
bin/dev    # or npm run dev — reads PORT, ECLUSE_POSTGRES_PORT, etc.
```

### Without the label

If no service has `ecluse.role: app`, ecluse warns and treats all services as data. Not an error.

### Teardown

```bash
ecluse down feat-foo                  # stops data containers, removes worktree + volumes
ecluse down feat-foo --keep-volumes   # keeps volumes
```

### Common failures

- **App can't connect to postgres** — `ECLUSE_POSTGRES_PORT` is the offset port (e.g. 5532, not 5432). Run `source .env.ecluse` first and use that port in your connection string.
- **Data containers didn't start** — `docker info` to verify Docker is running
- **Wrong service excluded** — check `ecluse.role: app` is only on the app service

---

## Troubleshooting

### Port already in use

By default ecluse auto-bumps to a free port if the nominal one is taken — you'll see a log line like `port 3001 in use; using 3009 for service 'api'`. No action needed unless you want a specific port.

If `strict_port = true` is set (or all alternatives are exhausted):

**Error:** `port 3001 is already in use by PID 12345; stop that process first`

```bash
kill 12345
lsof -iTCP:3001 -sTCP:LISTEN   # verify port is free
ecluse up                       # retry
```

Persistent conflict: change `base_port` in the relevant `[[services]]` block, or increase `port_search_range` and run `ecluse validate` to confirm no overlaps.

### Docker not running

```bash
open -a OrbStack      # macOS recommended
open -a Docker        # macOS fallback
sudo systemctl start docker   # Linux
docker info           # verify
```

### Slot exhaustion

**Error:** `all 8 slots are in use; run ecluse ls to see active sessions`

```bash
ecluse ls
ecluse down <stale-slug>
```

Or increase `max_slots` in `.ecluse.toml` directly.

### Stale state after manual worktree deletion

Run `ecluse down <slug>` anyway — handlers skip missing resources and remove the state entry. If that fails, edit `.ecluse/state.json` directly and remove the stale session object.

### Host Postgres unreachable

```bash
brew services start postgresql@16    # macOS
sudo systemctl start postgresql      # Linux
psql -U postgres -c "SELECT 1"       # verify
```

### Lock timeout

**Error:** `timed out waiting for state lock after 10s`

```bash
ps aux | grep ecluse      # check for live process
rm .ecluse/state.lock     # remove stale lock if nothing found
```

### Not inside a git repository

```bash
git init && git add . && git commit -m "init"
ecluse init
```

### Debug output

```bash
RUST_LOG=debug ecluse up feat-foo
```

### Error code reference

| Error | Cause | Fix |
|---|---|---|
| `SlugInvalid` | Slug doesn't match `^[a-z0-9][a-z0-9-]{0,30}[a-z0-9]$` | Lowercase letters, numbers, hyphens; 2–32 chars |
| `SlotsExhausted` | All slots in use | `ecluse ls` then `ecluse down <slug>` |
| `SessionExists` | Slug already active | Different slug or `ecluse down` first |
| `SessionNotFound` | Slug not in state | Check `ecluse ls` |
| `LockTimeout` | Another process holds lock | Check processes; remove stale lock |
| `ConfigMissing` | No `.ecluse.toml` found | `ecluse init` |
| `NotAGitRepo` | Not in a git repo | `git init` first |
| `ComposeFileNotFound` | No compose file at repo root | Add compose file or switch mode |
| `PortInUse` | Port bound by another process | `kill <pid>` then retry |
| `HookFailed` | `on_up` or `on_down` exited non-zero | Check the hook command and its output |
| `ProcessManagerUnavailable` | Configured `process_manager` binary not installed | Install it or set `process_manager = "none"` in `~/.config/ecluse/config.toml` |
| `SpawnFailed` | Failed to spawn a native service process | Check the `command` field in `.ecluse.toml` and the binary's availability |

---

## Limits

What ecluse intentionally does not do in v0. These are design decisions, not bugs.

- **Ports are checked, not reserved** — ecluse finds a free port at `up` time and writes it to `.env.ecluse`. There is a small window between that check and when your process actually binds. If another process takes the port in between, the value in `.env.ecluse` will be wrong. Fix: `ecluse down feat-foo --keep-worktree` then `ecluse up feat-foo --reuse-worktree`, or pin a specific port with `--port name=value`.
- **No process lifecycle management beyond spawn/kill** — ecluse can spawn native services on `up` (via `command` + `process_manager`) and kill them on `down`, but cannot restart a crashed process. If a service dies, check logs and do a `down`/`up` cycle. `ecluse ls` and `ecluse env` warn about dead nohup processes.
- **`command` requires the app to read its port from the environment** — ecluse injects the full `.env.ecluse` contents (`PORT`, `ECLUSE_SLOT`, `ECLUSE_SLUG`, `ECLUSE_MODE`, all `ECLUSE_<NAME>_PORT` vars, and any `port_env` aliases) directly into the spawned process environment — no separate sourcing step needed. This fails only if the app ignores the environment entirely: port hardcoded in source, or set in a config file (e.g. `config/puma.rb`, `vite.config.ts`). Use `port_env = "DJANGO_PORT"` to inject a custom var name, or pass via CLI flag: `command = "next dev --port $PORT"`.
- **Mode is set at `init`, not re-detected on `up`** — to change: `ecluse init --mode <new>`
- **Multiple compose files supported via `compose` field** — each `[[services]]` block with `run = "docker"` can point at its own compose file; services without it fall back to the root compose file. Run `ecluse init` per subdirectory only when you need fully independent slot pools.
- **`localhost:<port>` only** — no public URLs; use cloudflared or ngrok alongside ecluse
- **No agent process sandboxing** — container mode isolates services, not the agent's filesystem
- **`ecluse shell` is for humans, not agents** — agents use `ecluse up --json` or `ecluse env` to get the worktree path and env vars, then operate directly; `ecluse shell` spawns an interactive subshell which blocks non-interactive execution
- **No built-in database management** — ecluse allocates ports and writes env vars; use `[hooks] on_up`/`on_down` with your app's own tooling (prisma, rails db:create, psql, etc.)
- **macOS and Linux only** — WSL2 acceptable but untested; native Windows not supported
- **No background daemon** — every ecluse command is a short-lived process
- **No Ctrl+C rollback guarantee** — if killed mid-`up`, run `ecluse down <slug>` to clean partial state
- **No plugin/hook system** — wrap ecluse in a shell script for custom lifecycle behaviour
- **No telemetry** — no network calls except the optional Postgres TCP probe during `init`

---

## Configuration reference

`.ecluse.toml` (written by `ecluse init`, lives at repo root):

```toml
mode = "hybrid"         # container | host | hybrid
max_slots = 8           # max parallel sessions
prefix = "ecluse"       # prefix for compose project names and volume names
worktree_dir = ".ecluse/worktrees"

# Port collision handling (both optional)
# strict_port = false        # default: search for a free port on collision
# port_search_range = 10     # how many alternatives to try (bump by max_slots each time)
#                            # Guard: port_search_range × max_slots must not exceed
#                            # the gap between adjacent service base_ports.
#                            # Run `ecluse validate` to check.

# One [[services]] block per service.
# port = base_port + slot  (slot 1 → +1, slot 2 → +2, …)
# run = "native" (default) runs on host; run = "docker" runs in a container.
# The first native entry also sets the PORT alias for framework compatibility.
# Omit [[services]] entirely for single-service stacks — PORT = 3000 + slot.
# Add command = "..." to have ecluse spawn the process on ecluse up (native only).

[[services]]
name = "api"
base_port = 3000        # slot 1 → ECLUSE_API_PORT=3001 + PORT, slot 2 → 3002
command = "npm run dev" # optional — ecluse spawns this on ecluse up
# port_env = "DJANGO_PORT"              # also set DJANGO_PORT = allocated port
# port_env = ["DJANGO_PORT", "APP_PORT"] # or multiple aliases

[[services]]
name = "postgres"
run = "docker"
base_port = 5432        # slot 1 → ECLUSE_POSTGRES_PORT=5433, slot 2 → 5434
# compose = "services/postgres/docker-compose.yml"  # optional: per-service compose file

# Optional: lifecycle hooks — run in the worktree with all env vars set
[hooks]
on_up = "npx prisma migrate deploy"
on_down = "npx prisma migrate reset --force"
```

### Global config (`~/.config/ecluse/config.toml`)

Controls how native service processes are spawned. Written by `ecluse init`:

```toml
process_manager = "tmux"   # "tmux" | "nohup" | "none"
```

- `tmux` — creates a detached tmux session `ecluse-<slug>`; `ecluse shell <slug>` attaches to it
- `nohup` — background processes, logs at `.ecluse/logs/<slug>/`, PIDs at `.ecluse/pids/<slug>/`
- `none` — spawns nothing (default pre-v0.3 behaviour)

`ecluse init` auto-detects: tmux if present, otherwise nohup. `ecluse validate` checks the binary is installed. This is per-machine, not per-repo.

Hooks run as shell commands inside the worktree directory. All `.env.ecluse` variables (`PORT`, `ECLUSE_SLUG`, `ECLUSE_<NAME>_PORT`, etc.) are available. ecluse does not manage databases directly — use `on_up` for migrations, `on_down` for teardown.

## Examples

See [examples.md](examples.md) for 5 canonical config templates covering host, container, hybrid, multi-service monorepo, and Kubernetes. Each entry links directly to the `.ecluse.toml` and `docker-compose.yml` you can read and adapt.

## Commands

```
ecluse init [--mode container|host|hybrid] [--explain] [--yes]
ecluse up <slug> [--branch <name>] [--watch] [--json] [--reuse-worktree] [--port <name>=<value>]
ecluse env [<slug>]
ecluse down <slug> [--keep-volumes] [--keep-branch] [--keep-worktree]
ecluse ls [--json]
ecluse validate [--ports]
```

`ecluse shell` exists but is human-only — it spawns an interactive subshell that blocks non-interactive execution. Agents must not use it.

`ecluse validate` checks your `.ecluse.toml` for port range safety (ensures `port_search_range` doesn't create overlaps between services) and prints the current config. Pass `--ports` to see the full port allocation table across all slots.

**Soft restart** — tear down services without losing the git worktree, then spin up fresh:

```bash
ecluse down feat-foo --keep-worktree   # stops services, removes session from state, keeps worktree on disk
ecluse up feat-foo --reuse-worktree    # allocates a new slot, skips worktree creation
```

Use this when a service failed to bind after `up` and you want a fresh start without losing changes in the worktree.

**Port override** — pin a service to a specific port for this session:

```bash
ecluse up feat-foo --port api=4001 --port postgres=5444
```

Overrides bypass the auto-bump logic and use the given value directly. The overridden ports are stored in session state and reflected in `ecluse env` output.
