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

Or from source: `cargo install --git https://github.com/ecluse/ecluse`

### Five-minute on-ramp

```bash
cd my-project
ecluse init              # auto-detects mode, prompts to confirm
ecluse up feat-foo       # creates worktree + slot
cd .ecluse/worktrees/feat-foo
source .env.ecluse       # loads PORT, DATABASE_URL, etc.
npm run dev              # or bin/dev, bin/rails server, etc.
ecluse ls                # see active sessions
ecluse down feat-foo     # clean teardown
```

### What `ecluse up` does

1. Allocates a slot (integer 1–N) and computes port offset (`slot × stride`)
2. Creates a git worktree at `.ecluse/worktrees/<slug>` on branch `ecluse/<slug>`
3. Depending on mode: starts containers, provisions a database, or both
4. Writes `.env.ecluse` in the worktree

### Common first-time failures

- **"run `ecluse init` first"** — no `.ecluse.toml` found; run `ecluse init` from repo root
- **"all N slots in use"** — `ecluse ls` then `ecluse down <slug>` to free one
- **Docker not running** — `open -a OrbStack` or `open -a Docker`

---

## Agent Workflow

You're in a repo with `.ecluse.toml`. Use ecluse. Every task gets its own isolated slot — no port collisions, no shared database state, clean teardown when done.

### The canonical loop

```bash
# 1. Create a session — slug should be short, task-scoped, lowercase
ecluse up <slug>          # e.g. feat-auth, fix-login, refactor-api

# 2. Read the output — it tells you everything
#    Session:   feat-auth (slot 1)
#    Worktree:  .ecluse/worktrees/feat-auth
#    Mode:      hybrid
#    App port:  3100
#    Database:  myapp_feat_auth
#    Next step: cd .ecluse/worktrees/feat-auth && source .env.ecluse

# 3. Go to the worktree
cd <worktree_path>

# 4. Load environment
source .env.ecluse

# 5. Start dev server (host/hybrid only — container mode starts everything automatically)
npm run dev    # or: bin/dev / bin/rails server / python manage.py runserver

# 6. Do the work — normal git workflow, commits stay on ecluse/<slug> branch

# 7. Verify
npm test
curl http://localhost:$PORT/health

# 8. Tear down
ecluse down <slug>
```

### If you don't know the worktree path

```bash
ecluse ls --json | jq -r '.[] | select(.slug=="<slug>") | .worktree_path'
```

### Environment variables in `.env.ecluse`

| Variable | Example | Description |
|---|---|---|
| `PORT` | `3100` | App port — use this, not a hardcoded 3000 |
| `DATABASE_URL` | `postgres://localhost:5532/myapp_feat_auth` | Postgres connection string |
| `REDIS_URL` | `redis://localhost:6479` | Redis connection string (if redis present) |
| `ECLUSE_SLOT` | `1` | Slot number |
| `ECLUSE_OFFSET` | `100` | Port offset (`slot × stride`) |
| `ECLUSE_MODE` | `hybrid` | `container`, `host`, or `hybrid` |
| `ECLUSE_<SERVICE>_PORT` | `ECLUSE_POSTGRES_PORT=5532` | Offset host port per service |

### Parallel sessions

```bash
ecluse up feat-auth    # slot 1, port 3100, db myapp_feat_auth
ecluse up feat-cache   # slot 2, port 3200, db myapp_feat_cache
ecluse ls              # see both
```

Each session is a separate git branch. They don't interfere.

### Common failures

- **"session already exists"** — use a different slug or `ecluse down <slug>` first
- **"all slots in use"** — `ecluse ls` to find stale sessions, `ecluse down` the oldest
- **Dev server can't reach database** — you forgot `source .env.ecluse` before starting
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

No containers. ecluse reserves a port range, optionally provisions a Postgres database, writes `.env.ecluse`, creates the worktree. You start your own dev server.

### Prerequisites

- No Docker required
- Host Postgres if you want database isolation (`brew services start postgresql@16`)

### Workflow

```bash
ecluse up feat-foo
# Session:   feat-foo (slot 1)
# App port:  3100
# Database:  myapp_feat_foo
# Next step: cd .ecluse/worktrees/feat-foo && source .env.ecluse

cd .ecluse/worktrees/feat-foo
source .env.ecluse
npm run dev    # reads $PORT, $DATABASE_URL
```

### Database provisioning

Without a `[database]` section, only port isolation is provided. To enable per-session databases:

```toml
[database]
provider = "postgres-host"
host = "localhost"
port = 5432
user = "postgres"
base = "myapp"
# auth: use PGPASSWORD env var or ~/.pgpass — never write passwords to .ecluse.toml
```

With `base = "myapp"` and slug `feat-foo`, ecluse creates `myapp_feat_foo` on `up` and drops it on `down`.

### Teardown

```bash
ecluse down feat-foo                    # drops database, removes worktree
ecluse down feat-foo --keep-database    # keeps database
```

### Common failures

- **"Port 3100 is in use by PID 12345"** — `kill 12345`
- **"Host Postgres is unreachable"** — `brew services start postgresql@16`; check `[database]` config
- **App can't find database** — `source .env.ecluse` before starting the server

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
# App port:  3100
# Database:  postgres://localhost:5532/myapp_feat_foo

cd .ecluse/worktrees/feat-foo
source .env.ecluse
bin/dev    # or npm run dev — reads PORT, DATABASE_URL, REDIS_URL
```

### Without the label

If no service has `ecluse.role: app`, ecluse warns and treats all services as data. Not an error.

### Teardown

```bash
ecluse down feat-foo                  # stops data containers, removes worktree + volumes
ecluse down feat-foo --keep-volumes   # keeps volumes
```

### Common failures

- **App can't connect to postgres** — `DATABASE_URL` points to offset port (5532, not 5432). Run `source .env.ecluse` first.
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

Check `.ecluse.toml` `[database]` has correct `host`, `port`, `user`.

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
| `PostgresUnreachable` | Can't connect to configured postgres | Start postgres; check config |

---

## Limits

What ecluse intentionally does not do in v0. These are design decisions, not bugs.

- **Mode is set at `init`, not re-detected on `up`** — to change: `ecluse init --mode <new>`
- **One compose file per repo root** — monorepos: run `ecluse init` per subdirectory
- **`localhost:<port>` only** — no public URLs; use cloudflared or ngrok alongside ecluse
- **No agent process sandboxing** — container mode isolates services, not the agent's filesystem
- **No process lifecycle management** — ecluse does not start dev servers or manage tmux sessions; it sets up the environment and writes `.env.ecluse`
- **macOS and Linux only** — WSL2 acceptable but untested; native Windows not supported
- **Postgres only for database provisioning** — MySQL, MongoDB, SQLite not supported in v0
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
stride = 100            # port offset per slot
prefix = "ecluse"       # prefix for compose project names and volume names
worktree_dir = ".ecluse/worktrees"

# Optional: database provisioning (host and hybrid modes)
[database]
provider = "postgres-host"
host = "localhost"
port = 5432
user = "postgres"
base = "myapp"          # database name prefix; slug is appended
```

## Commands

```
ecluse init [--mode container|host|hybrid] [--explain] [--yes]
ecluse up <slug> [--branch <name>] [--watch]
ecluse down <slug> [--keep-volumes] [--keep-database] [--keep-branch]
ecluse ls [--json]
ecluse skills [list | show <name> | install]
```
