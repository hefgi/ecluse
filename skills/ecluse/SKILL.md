---
name: ecluse
description: >
  Complete reference for ecluse — the per-worktree dev environment isolation CLI.
  Use this skill whenever ecluse is mentioned, a .ecluse.toml file is present in
  the repo, the user asks about worktree isolation, parallel dev environments,
  or port/database conflicts between branches. If you are a coding agent about to
  do substantive work in a repo that has .ecluse.toml, use this skill automatically
  before starting — do not wait to be asked.
tags:
  - ecluse
  - worktree
  - isolation
  - docker
  - compose
  - postgres
---

# ecluse

Per-worktree isolation for development environments. Each `ecluse up` allocates a **slot** — an integer that drives every isolated resource: port offset, database name, Docker volume names, and git worktree. Nothing leaks between sessions.

## Quick navigation

- **New to ecluse?** → [Getting Started](#getting-started)
- **Coding agent in a repo with .ecluse.toml?** → [Agent Workflow](#agent-workflow) — use this now
- **Which mode to pick?** → [Choosing a Mode](#choosing-a-mode)
- **Container mode details** → [Container Mode](#container-mode)
- **Host mode details** → [Host Mode](#host-mode)
- **Hybrid mode details** → [Hybrid Mode](#hybrid-mode)
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

Or from source: `cargo install --git https://github.com/hefgi/ecluse`

### Five-minute on-ramp

```bash
cd my-project
ecluse init              # auto-detects mode, prompts to confirm
ecluse up feat-foo       # creates worktree + slot
ecluse shell feat-foo    # drops into worktree with env loaded
npm run dev              # PORT, DATABASE_URL, etc. already set
ecluse ls                # see active sessions (from another terminal)
ecluse down feat-foo     # clean teardown
```

### What `ecluse up` does

1. Allocates a slot (integer 1–N) and computes port offset (`slot × stride`)
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
#     "PORT": "3100",               ← alias for first [ports] entry
#     "ECLUSE_API_PORT": "3100",    ← if [ports] api = 0
#     "ECLUSE_FRONTEND_PORT": "3101", ← if [ports] frontend = 1
#     "ECLUSE_POSTGRES_PORT": "5532", ← from compose data service
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
| `PORT` | `3100` | Alias for the first `[ports]` entry — never hardcode 3000 |
| `ECLUSE_<NAME>_PORT` | `ECLUSE_API_PORT=3100` | Per-service port from `[ports]` config |
| `ECLUSE_<SERVICE>_PORT` | `ECLUSE_POSTGRES_PORT=5532` | Data service port from compose (hybrid/container) |
| `ECLUSE_SLOT` | `1` | Slot number |
| `ECLUSE_SLUG` | `feat-auth` | Session slug |
| `ECLUSE_OFFSET` | `100` | Port offset (`slot × stride`) |
| `ECLUSE_MODE` | `hybrid` | `container`, `host`, or `hybrid` |

**No `DATABASE_URL` or `REDIS_URL` are set automatically.** Data service ports are exposed as `ECLUSE_<SERVICE>_PORT` (e.g. `ECLUSE_POSTGRES_PORT=5532`). Construct connection strings in your `on_up` hook or app config using that port. This keeps ecluse engine-agnostic.

### Parallel sessions

```bash
ecluse up feat-auth --json   # slot 1 → ECLUSE_API_PORT=3100, ECLUSE_POSTGRES_PORT=5532
ecluse up feat-cache --json  # slot 2 → ECLUSE_API_PORT=3200, ECLUSE_POSTGRES_PORT=5632
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
- **Monorepo with multiple compose files** — run `ecluse init` separately per subdirectory

---

## Container Mode

Every service — including the app — runs in Docker under a unique compose project per session. Ports are offset; volumes are namespaced.

### Prerequisites

- Docker/OrbStack running
- `docker-compose.yml` with `build: .` on the app service

### Port offsets (stride = 100)

| Session | Slot | web 3000 → host | postgres 5432 → host |
|---|---|---|---|
| `feat-foo` | 1 | 3100 | 5532 |
| `fix-bar` | 2 | 3200 | 5632 |

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

No containers. ecluse reserves a port range, writes `.env.ecluse`, creates the worktree, runs `on_up`. You start your own dev server.

### Prerequisites

- No Docker required
- Host Postgres/MySQL/SQLite already available if your app needs a database

### Workflow

```bash
ecluse up feat-foo
# Session:   feat-foo (slot 1)
# App port:  3100  (ECLUSE_API_PORT + PORT alias, if [ports] api = 0)
# Next step: cd .ecluse/worktrees/feat-foo && source .env.ecluse

cd .ecluse/worktrees/feat-foo
source .env.ecluse
npm run dev    # reads $PORT; run migrations via [hooks] on_up
```

### Database in host mode

ecluse does not provision databases. Use `[hooks] on_up` with your app's own tooling:

```toml
[ports]
app = 0

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

Data services (postgres, redis, etc.) run in containers with offset ports and namespaced volumes. App runs on the host.

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
# postgres and redis start in Docker with offset ports
# ECLUSE_POSTGRES_PORT=5532, ECLUSE_REDIS_PORT=6479
# ECLUSE_API_PORT=3100, PORT=3100  (from [ports] api = 0)

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

**Error:** `port 3100 is already in use by PID 12345; stop that process first`

```bash
kill 12345
lsof -iTCP:3100 -sTCP:LISTEN   # verify port is free
ecluse up                       # retry
```

Persistent conflict: increase `stride` in `.ecluse.toml` to move into a less-contested range.

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

---

## Limits

What ecluse intentionally does not do in v0. These are design decisions, not bugs.

- **Mode is set at `init`, not re-detected on `up`** — to change: `ecluse init --mode <new>`
- **One compose file per repo root** — monorepos: run `ecluse init` per subdirectory
- **`localhost:<port>` only** — no public URLs; use cloudflared or ngrok alongside ecluse
- **No agent process sandboxing** — container mode isolates services, not the agent's filesystem
- **No process lifecycle management** — ecluse does not start dev servers or manage tmux sessions; it sets up the environment and writes `.env.ecluse`
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
base_port = 3000        # slot 1 = base_port + stride, slot 2 = base_port + 2*stride
stride = 100            # port offset per slot — must be > number of [ports] entries
prefix = "ecluse"       # prefix for compose project names and volume names
worktree_dir = ".ecluse/worktrees"

# Named ports within each slot's range.
# index 0 → base_port + slot*stride + 0  (also sets PORT alias)
# index 1 → base_port + slot*stride + 1
# Each entry generates ECLUSE_<NAME>_PORT.
# Omit [ports] for single-service stacks — PORT is set automatically.
[ports]
api = 0
frontend = 1

# Optional: lifecycle hooks — run in the worktree with all env vars set
[hooks]
on_up = "npx prisma migrate deploy"
on_down = "npx prisma migrate reset --force"
```

Hooks run as shell commands inside the worktree directory. All `.env.ecluse` variables (`PORT`, `ECLUSE_SLUG`, `ECLUSE_<NAME>_PORT`, etc.) are available. ecluse does not manage databases directly — use `on_up` for migrations, `on_down` for teardown.

## Examples

Ready-to-use config templates live in `examples/` at the repo root. When a user asks how to configure ecluse for a specific stack, read the relevant `.ecluse.toml` and `docker-compose.yml` directly — they are the authoritative reference.

| Directory | Mode | Stack | Ports |
|---|---|---|---|
| `examples/rails-hybrid` | hybrid | Rails 7 + Angular + Postgres + Redis | `api`, `frontend` |
| `examples/rails-monorepo` | hybrid | Rails 7 + Sidekiq + Blazer admin + Postgres + Redis | `web`, `sidekiq`, `admin` |
| `examples/node-hybrid` | hybrid | Express + React + Postgres | `api`, `frontend` |
| `examples/node-container` | container | Node.js fully containerized | from compose |
| `examples/nextjs-hybrid` | hybrid | Next.js + Prisma + Postgres | `app` |
| `examples/t3-host` | host | T3 (Next.js + tRPC + Prisma), no Docker | `app` |
| `examples/t3-monorepo` | hybrid | Turborepo (API + Web + Worker + Email) + Postgres + Redis | `api`, `web`, `worker`, `email` |
| `examples/fastapi-hybrid` | hybrid | FastAPI + Vue + Postgres | `api`, `frontend` |
| `examples/go-hybrid` | hybrid | Go HTTP server + Postgres | `api` |
| `examples/mongo-hybrid` | hybrid | Node.js + MongoDB | `api` |
| `examples/k3d` | host | Kubernetes via k3d (all services inside cluster) | `http`, `https` |

Key patterns to know:

- **Single service** (`app = 0` or `api = 0`) — one port, `PORT` alias set automatically
- **Frontend + backend** (`api = 0`, `frontend = 1`) — two ports, each app reads its own var
- **Full monorepo** (`api`, `web`, `worker`, `email`) — four ports, Turborepo starts all
- **Kubernetes** (`http`, `https`) — two ingress ports only; services communicate inside the cluster via DNS

## Commands

```
ecluse init [--mode container|host|hybrid] [--explain] [--yes]
ecluse up <slug> [--branch <name>] [--watch] [--json]
ecluse env <slug>
ecluse shell <slug>
ecluse down <slug> [--keep-volumes] [--keep-branch]
ecluse ls [--json]
```
