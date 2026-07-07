---
name: ecluse
description: >
  Complete reference for ecluse — ephemeral local environments for coding agents,
  any stack. Use this skill whenever ecluse is mentioned, a .ecluse.toml file is
  present in the repo, the user asks about worktree isolation, parallel dev
  environments, or port/database conflicts between branches. When a .ecluse.toml
  file is present in the repo, loading this skill before starting work is strongly
  recommended so you understand the isolation model and avoid port conflicts.
tags:
  - ecluse
  - worktree
  - isolation
  - environment
  - ephemeral
  - testing
  - parallelization
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
ecluse down feat-foo --delete-worktree   # clean teardown (--delete-worktree skips the interactive prompt)
```

**Important — use a long timeout for `ecluse up` and use `--delete-worktree` for `ecluse down`.**
`ecluse up` runs `post_up` hooks synchronously — a hook that polls until postgres is ready can take 30–120 seconds. Always invoke with a generous timeout:
```
Bash({"command": "ecluse up feat-foo --json", "timeout": 300000})
```

`ecluse down` prompts interactively before removing a worktree when neither `--keep-worktree` nor `--delete-worktree` is passed. In a non-interactive context (agents, CI) this blocks until killed. Always pass one of those flags:
```
Bash({"command": "ecluse down feat-foo --delete-worktree"})
Bash({"command": "ecluse down feat-foo --keep-worktree"})
```

Note: `ecluse shell` spawns an interactive subshell — agents cannot use it. Use `ecluse up --json` or `ecluse env <slug>` to get the worktree path and env, then operate directly.

### What `ecluse up` does

**New session:**
1. Allocates a slot (integer 1–N)
2. Creates a git worktree at `.ecluse/worktrees/<slug>` on the given branch (branch name preserved; slug is the sanitized form used for paths and Docker names)
3. Symlinks `.env` and `.env.local` from the repo root into the worktree **by default — no config needed**. To opt out entirely set `inherit_env = []` in `.ecluse.toml`, or pass `--no-inherit-env` to skip for a single run. To symlink a different set of files: `inherit_env = [".env", ".env.staging"]`. To make a file a per-worktree copy instead of a symlink (so edits in the worktree don't leak back to the root): `inherit_env = [".env", { file = ".env.local", mode = "copy" }]` — copy entries are initialized once and never re-copied on subsequent `ecluse up` runs.
4. Depending on mode: starts containers, writes `.env.ecluse`, runs `pre_up` then `post_up` hooks if configured
5. `post_up` runs in the worktree with all env vars set — use it for migrations, seeding, etc.

**Existing session (idempotent):**
- Reuses the existing worktree and slot — no worktree creation, no slot allocation
- Checks which services are already running; starts only the ones that are down
- Each service decision is logged: "already running — skipped" / "down — will start"
- Slug is auto-detected from cwd when inside a worktree

Pass your git branch name directly — slashes are sanitized to hyphens. When no argument is given, resolution is based on worktree location:

| Location | Behaviour |
|---|---|
| Inside an ecluse-registered worktree | Reuse stored slug/branch |
| Inside any other git worktree | Read branch from cwd, auto-register (`--reuse-worktree` implied) |
| Repo root / main worktree | Prompt for a branch name |

```bash
ecluse up feat/add-auth   # branch=feat/add-auth, slug=feat-add-auth
ecluse up feat-add-auth   # same result — already a valid slug
ecluse up feat-foo        # new or existing: always does the right thing
ecluse up                 # inside any worktree → branch auto-detected from cwd
ecluse up                 # in repo root → prompts for branch name
ecluse up --force         # kill all services on allocated ports, restart all
ecluse up --skip api      # skip the api service; start everything else
ecluse up --force --skip db  # kill + restart all except db
```

### Common first-time failures

- **"run `ecluse init` first"** — no `.ecluse.toml` found; run `ecluse init` from repo root
- **"all N slots in use"** — `ecluse ls` then `ecluse down <slug>` to free one
- **Docker not running** — `open -a OrbStack` or `open -a Docker`

---

## Agent Workflow

You're in a repo with `.ecluse.toml`. Use ecluse. Every task gets its own isolated slot — no port collisions, clean teardown when done.

### The canonical loop

```bash
# 1. Create session — pass your git branch name directly (slashes OK)
ecluse up feat/add-auth --json
# Returns:
# {
#   "slug": "feat-add-auth",   ← sanitized: / replaced with -, lowercased
#   "slot": 1,
#   "mode": "hybrid",
#   "branch": "feat/add-auth", ← original branch name preserved
#   "worktree_path": "/path/to/.ecluse/worktrees/feat-add-auth",
#   "env_file": "/path/to/.ecluse/worktrees/feat-add-auth/.env.ecluse",
#   "env": {
#     "PORT": "3001",                  ← alias for first native [[services]] entry
#     "ECLUSE_API_PORT": "3001",       ← if [[services]] name="api" base_port=3000
#     "ECLUSE_POSTGRES_PORT": "5433",  ← if [[services]] name="postgres" base_port=5432 run="docker"
#     "ECLUSE_SLOT": "1",
#     "ECLUSE_SLUG": "feat-add-auth",
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
ecluse down <slug> --delete-worktree
```

### Query an existing session anytime

```bash
ecluse ls                       # all sessions: ports as name=value pairs; TMUX column when tmux is in use
ecluse env <slug>               # same JSON as up --json: worktree_path + all env vars
ecluse env                      # auto-detects current session if run from inside a worktree
ecluse status <slug>            # per-service health: ✓/✗ with port; shows WINDOW (tmux) or PID (nohup); exit 1 if any down
ecluse status <slug> --json     # machine-readable health check (includes PIDs per service)
ecluse status                   # auto-detect slug from cwd
ecluse whose-pid <pid>          # which session owns this PID? exit 1 if not owned by any session
ecluse whose-pid <pid> --json   # machine-readable; required before any raw kill of an ecluse-allocated port
```

### Sync a manually-started environment

If services were started by hand (not via `ecluse up`), or state.json was lost, use `ecluse sync` to make ecluse aware of the running session:

```bash
ecluse sync <slug>          # discover processes + register session in state.json
ecluse sync                 # same, slug auto-detected from cwd
ecluse sync <slug> --json   # machine-readable output
```

`ecluse sync` works by:
1. Finding all processes whose cwd is inside the worktree (via `lsof +d`)
2. Matching each native service in `.ecluse.toml` by walking the process tree from its `command` and finding the descendant that is listening on a port
3. Detecting docker services (hybrid mode) via `docker ps`, matching by container name containing the slug
4. Writing PID files for discovered processes so `ecluse down` can kill them
5. Writing `.env.ecluse` with the actual ports found
6. Registering (or updating) the session in `state.json`

After sync, `ecluse ls`, `ecluse env`, and `ecluse down` all work normally. If a session already exists for the slug, sync refreshes its port_overrides and PID tracking without changing the slot or branch.

**Failure modes:**
- `"no running processes found in worktree"` — start your services first, then sync
- `"worktree not found for slug"` — either run from inside the worktree, or ensure the worktree exists at `.ecluse/worktrees/<slug>`
- Unmatched services are reported as warnings (not errors) — partial sync is still registered

**Requirement:** native services must have a `command` field in `.ecluse.toml` for sync to find them. Docker services are matched by container name.

### Hard reset with ecluse flush

Use `ecluse flush` when sessions are stuck, `state.json` is corrupted, or you want to wipe all ecluse state and start fresh:

```bash
ecluse flush        # prompts for confirmation
ecluse flush --yes  # skip prompt — use in CI or agent scripts
```

Flush does the following, in order:

1. Tears down all sessions known to `state.json` (same as `ecluse shutdown`)
2. Kills orphaned tmux sessions named `ecluse-*`
3. Stops orphaned Docker Compose projects matching `<prefix>_*` (detected via `docker ps`)
4. Removes all directories under `worktree_dir` with `git worktree remove --force`, then `git worktree prune`
5. Deletes `.ecluse/pids/`, `.ecluse/logs/`, `.ecluse/overlays/`
6. Resets `state.json` to `{"version": 1, "sessions": []}`

Steps 1–5 are best-effort: failures are logged and ignored. The only hard failure is step 6 (cannot reset state.json).

**Docker volumes are not removed.** Run `docker volume prune` separately if you also want data volumes gone.

After flush, `ecluse ls` returns "no active sessions" and all slots are free.

### Environment variables

All vars are in the JSON from `ecluse up --json` or `ecluse env <slug>`.

| Variable | Example | Description |
|---|---|---|
| `PORT` | `3001` | Alias for the first native `[[services]]` entry — never hardcode 3000 |
| `ECLUSE_<NAME>_PORT` | `ECLUSE_API_PORT=3001` | Per-service port: `base_port + slot` |
| `<port_env>` | `NODE_INSPECT_PORT=9230` | Custom var from `extra_ports[].port_env`: `base_port + slot`. Also published as a host→container binding in docker overlays. |
| `ECLUSE_<NAME>_DEBUG_PORT` | `ECLUSE_API_DEBUG_PORT=9230` | *Deprecated — use `extra_ports` instead.* Emitted when `debug_port` is set in `.ecluse.toml`. |
| `ECLUSE_SLOT` | `1` | Slot number |
| `ECLUSE_SLUG` | `feat-auth` | Session slug |
| `ECLUSE_MODE` | `hybrid` | `container`, `host`, or `hybrid` |

**No `DATABASE_URL` or `REDIS_URL` are set automatically.** Data service ports are exposed as `ECLUSE_<SERVICE>_PORT` (e.g. `ECLUSE_POSTGRES_PORT=5433`). Construct connection strings in your `post_up` hook or app config using that port. This keeps ecluse engine-agnostic.

### Parallel sessions

```bash
ecluse up feat-auth --json   # slot 1 → ECLUSE_API_PORT=3001, ECLUSE_POSTGRES_PORT=5433
ecluse up feat-cache --json  # slot 2 → ECLUSE_API_PORT=3002, ECLUSE_POSTGRES_PORT=5434
ecluse ls                    # see both
```

Each session is a separate git branch and worktree. They don't interfere.

### Common failures

- **"all slots in use"** — `ecluse ls` to find stale sessions, `ecluse down <slug> --delete-worktree` the oldest
- **`ecluse down` hangs waiting for input** — always pass `--delete-worktree` (or `--keep-worktree`) when running non-interactively; without either flag the command prompts the user before removing the worktree
- **Port in use** — first determine *who* owns it (see "Killing services safely" below). Never run a blind `lsof -ti TCP:<port> \| xargs kill` — that's how parallel agents kill each other.

### Killing services safely — never raw-kill ecluse-allocated ports

**Never `kill` a process on an ecluse-allocated port without checking ownership first.**

The canonical fix for misbehaving services is `ecluse down` + `ecluse up`, not `kill`:

```bash
ecluse down <your-slug> --keep-worktree
ecluse up <your-slug>                    # stopped session auto-detected; resumes at the same slot
```

This tears down only **your** services and respawns them with the right env. Idempotent, safe under parallel sessions, never touches another agent's work. Reach for this 95% of the time.

**If raw `kill` is truly unavoidable, always verify ownership first:**

```bash
ecluse whose-pid <pid>
# → "owned by session <slug>" → STOP. Do not kill. Coordinate with that session.
# → "not owned by any ecluse session" → safe to kill if you're sure it's not other work
```

Never run `lsof -ti TCP:<port> | xargs kill` blind. Every PID gets checked individually with `whose-pid` first.

**Why this matters:** in parallel sessions, the process on the port "next to yours" is almost always *another agent's working service*, not your own stale leftover. Kill it and you derail a sibling session. This has happened in production: three parallel agents killed each other's services seven times in seven minutes, each one thinking it was "cleaning up rogue processes" from the others' worktrees.

**Caveat — services spawned outside ecluse:** if you ran `task ...`, `make ...`, `npm run ...`, or similar from a tmux/Bash shell instead of letting ecluse spawn via `command = ...` in `.ecluse.toml`, ecluse may not know about those PIDs. Run `ecluse sync <your-slug>` first to register them, then `ecluse down --keep-worktree` will kill them properly.

### Port wiring — exhaust .ecluse.toml options before touching app code

When a service has a hardcoded port or reads it from a config file, resolve it through `.ecluse.toml` configuration. Only modify application source code as a last resort.

**Resolution order (stop at the first that works):**

1. **CLI flag in `command`** — most frameworks accept `--port` as a CLI argument. Pass the ecluse var directly:
   ```toml
   [[services]]
   name = "web"
   base_port = 3000
   command = "vite --port $ECLUSE_WEB_PORT"        # Vite
   # command = "next dev --port $ECLUSE_WEB_PORT"  # Next.js
   # command = "bin/rails s -p $ECLUSE_WEB_PORT"   # Rails
   # command = "pnpm dev --port $ECLUSE_WEB_PORT"  # any Vite-based monorepo package
   ```

2. **`port_env`** — app reads a custom env var name (not `PORT`). Map the allocated port to that name:
   ```toml
   [[services]]
   name = "api"
   base_port = 4000
   port_env = "DJANGO_PORT"   # ecluse sets DJANGO_PORT = allocated port
   command = "python manage.py runserver 0.0.0.0:$DJANGO_PORT"
   ```
   Multiple services that each need a distinct var (e.g. a monorepo with three APIs that all read `process.env.PORT`):
   ```toml
   [[services]]
   name = "api"
   base_port = 4444
   port_env = "ECLUSE_API_PORT"
   command = "pnpm --filter api dev --port $ECLUSE_API_PORT"

   [[services]]
   name = "admin-api"
   base_port = 4544
   port_env = "ECLUSE_ADMIN_API_PORT"
   command = "pnpm --filter admin-api dev --port $ECLUSE_ADMIN_API_PORT"
   ```

3. **Modify app source code** — only if the framework has no `--port` flag and reads no env var at all (rare). Document why the other options were not viable before making the change.

**Do not** modify `vite.config.ts`, `next.config.js`, `config/puma.rb`, or similar config files when a CLI flag or `port_env` can achieve the same result.

**Avoid external task runners as service entry points.** `task`, `make`, `npm run` and similar runners re-read `.env.local` and inherit the spawning shell's env — neither of those is `.env.ecluse`. Under parallel sessions this causes services to bind to the *wrong slot's* ports because the spawning shell may carry env from a different worktree. Always put the actual service command directly in `[[services]] command = "..."` so ecluse spawns it with the correct slot env in one step. If you must use an external runner, run `ecluse sync <your-slug>` immediately after starting it so the resulting PIDs are tracked.

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

Detection runs 20+ signals. Key ones (run `ecluse init --explain` for the full breakdown):

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
- **Monorepo, each service has its own compose file** — point each `[[services]]` block at its own file with `compose = "services/foo/docker-compose.yml"`; only use separate subdirectory `.ecluse.toml` files when you need fully independent slot pools and state

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

- **Hardcoded port in app code or config file** — use a CLI flag in `command` or `port_env` in `.ecluse.toml` before modifying app source; see Port wiring section in Agent Workflow
- **`--watch` requires Compose v2.22+** — pass `ecluse up --watch`
- **Invoking `docker compose` directly** — the overlay won't be included; use `ecluse up` or add `-f .ecluse/overlays/<slug>.yml`
- **Multiple services collide on a shared debugger/auxiliary port** — Node.js `--inspect` defaults to 9229, Delve to 2345, debugpy to 5678, etc. When multiple services share a default, the second one fails with `EADDRINUSE`. Fix: add `extra_ports = [{ base_port = 9229, port_env = "NODE_INSPECT_PORT" }]` to each conflicting `[[services]]` block and pass the allocated var in `command` (e.g. `NODE_OPTIONS='--inspect=0.0.0.0:$NODE_INSPECT_PORT'`, `dlv ... --listen=:$NODE_INSPECT_PORT`). For docker services the port is also published as a host→container binding automatically.

---

## Host Mode

No containers. ecluse reserves a port range, writes `.env.ecluse`, creates the worktree, runs `pre_up` then `post_up` hooks. If `command` is set on a `[[services]]` entry, ecluse spawns it automatically via your global `process_manager` (tmux or nohup). Otherwise you start your own dev server.

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
npm run dev    # reads $PORT; run migrations via [hooks] post_up
```

### Database in host mode

ecluse does not provision databases. Use `[hooks] post_up` with your app's own tooling:

```toml
[[services]]
name = "app"
base_port = 3000

[hooks]
post_up = "npx prisma migrate deploy"
pre_down = "npx prisma migrate reset --force"
```

Your app's connection string is defined in your `.env` (not managed by ecluse). The hook runs inside the worktree with all ecluse env vars set.

### Teardown

```bash
ecluse down feat-foo   # runs pre_down hook, removes worktree, then runs post_down
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
ecluse whose-pid 12345   # MUST verify before killing — see "Killing services safely" above
# If unowned:
kill 12345
lsof -iTCP:3001 -sTCP:LISTEN   # verify port is free
ecluse up                       # retry
```

Persistent conflict: change `base_port` in the relevant `[[services]]` block, or increase `port_search_range` and run `ecluse validate` to confirm no overlaps.

### Cross-agent port collision (parallel sessions killing each other)

**Symptom:** in parallel sessions on the same repo, services keep dying with `exit 137` / SIGKILL even though no one ran `ecluse down`. Restarting via `task` or `make` lands them on the wrong port. Multiple agents are running `kill` or `lsof -ti | xargs kill` against each other's PIDs.

**Root cause:** an external task runner (`task`, `make`, `npm run`, `bin/dev`) was used as the service entry point instead of `command = "..."` in `.ecluse.toml`. External runners re-read `.env.local` and inherit the spawning shell's env — neither knows about `.env.ecluse`. Under parallel sessions the spawning shell can carry env from a *different* worktree's `source .env.ecluse`, so services bind to the wrong slot's ports. Agents then see "a process on a port adjacent to mine" and kill it, believing it's their own stale leftover.

**Recovery (do this in each affected session):**

```bash
# 1. Stop all the wrong-slot processes safely
ecluse down <your-slug> --keep-worktree

# 2. Restart with ecluse spawning the services directly
ecluse up <your-slug>                    # stopped session auto-detected; resumes at the same slot

# 3. Verify ports are correct
ecluse status <your-slug>
```

**Prevention:**

1. **Move the actual service commands into `[[services]] command = "..."`** so ecluse spawns them with the correct slot env. This is the only real fix.
2. **If you must use an external runner**, run `ecluse sync <your-slug>` immediately after starting it so the resulting PIDs are tracked. Then `ecluse down --keep-worktree` will kill them properly.
3. **Never run `lsof -ti TCP:<port> | xargs kill` blind** — see "Killing services safely". Use `ecluse whose-pid` to verify ownership before any manual kill.
4. **Consider `slot_stride = 10` in `.ecluse.toml`** for visually distinct adjacent-slot ports (3010, 3020, 3030 instead of 3001, 3002, 3003). Doesn't prevent the root cause but makes mistakes harder.

### Wrong content served on the configured URL after multiple up/down cycles

**Symptom:** the user navigates to `http://localhost:7301` expecting slot 1, but sees slot 4's branch instead. `ecluse status` reports the slot 1 service as healthy. Restarting only the affected session doesn't fix it — the wrong content keeps appearing on the configured port.

**Root cause (fixed in 0.3.2+):** an orphan from a previous session is holding the port. Common cause: pnpm/npm wrapper chains where the actual server is a grandchild (`sh → pnpm → node → vite`) — under 0.3.1 and earlier, `ecluse down` killed only the outer wrapper and the actual server reparented to `launchd`/`init`, surviving indefinitely and holding 4-8 ports each. After several `up`/`down` cycles these orphans accumulated and silently collided with new sessions.

**Detection:** `ecluse status` in 0.3.2+ flags this directly:

```
SERVICE       TYPE     PORT   STATUS                       WINDOW
backoffice    native   7301   ✗ wrong owner (PID 81906)    backoffice
```

The `wrong owner` row means: the stored PID (or its descendants) is NOT what's currently listening on 7301 — something else is. JSON output gains `listener_pid` and `wrong_owner` fields. Exit code is 1 (same as `✗ down`).

**Recovery on any version:**

```bash
ecluse whose-pid <listener-pid>   # confirm it's an orphan, not another session
# If unowned by any ecluse session:
kill -- -<listener-pid>            # kill the whole process group (the `-` prefix)
# OR, the recovery hammer (kills everything in worktrees + every configured port):
ecluse flush --yes
```

**Prevention:** upgrade to 0.3.2+. The tmux teardown path now kills the whole process group (TERM→KILL grace), matching what the nohup path already did. `ecluse flush` also sweeps both the worktree cwd and every configured port to clean up orphans that escaped a previous version's teardown.

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

### Service can't find secrets from .env / .env.local

ecluse symlinks `.env` and `.env.local` from the repo root into each worktree at `ecluse up` time — so the files are there on disk. Whether the service actually reads them depends on the framework:

- **Auto-loaded** (no action needed): Next.js, Vite, Create React App, `docker compose` (reads `.env` from the compose file's directory)
- **Must load explicitly**: Node.js without a dotenv call, Rails, Django, Go, Rust — the process only sees what ecluse injects via `command` / `.env.ecluse`

For frameworks that need explicit loading, two options:

1. Use a `post_up` hook to source the file or run a seed/setup script
2. Make the app call `dotenv` (or equivalent) at startup to load `.env`

Note: `ECLUSE_*` vars and `PORT` from `.env.ecluse` are always injected into the spawned process environment by ecluse — those never need explicit loading.

### Per-worktree .env override is being clobbered (or shared across worktrees)

**Symptom:** the user edits `.env.local` inside a worktree to set, say, `AUTH_ENABLED=false`, but every other worktree (and the repo root) now sees the same flip — or running `ecluse up` again restores the original value.

**Cause:** by default `inherit_env` uses `mode = "symlink"`, so each worktree's `.env.local` is a symlink back to the root file. Editing it edits the shared root file, and every other worktree sees the change through its own symlink. This is correct for shared secrets (DB passwords, API keys) but wrong for per-worktree feature flags.

**Fix:** mark the file as `mode = "copy"` in `.ecluse.toml`. Copy entries are initialized from the root file once on first `ecluse up`, then left alone forever after — worktree edits stay local, root edits don't propagate, and parallel worktrees each get their own independent file.

```toml
# .ecluse.toml
inherit_env = [
  ".env",                                  # shared secrets — keep symlinked
  { file = ".env.local", mode = "copy" },  # per-worktree overrides — independent
]
```

After the change: existing symlinks for `.env.local` in already-created worktrees are replaced with a fresh copy on the next `ecluse up`. If a worktree already has a real `.env.local` (not a symlink), it is preserved as-is — `ecluse up` never re-copies on top of a non-symlinked file.

To skip inheritance entirely for a single run (CI, ephemeral agent sessions): `ecluse up <slug> --no-inherit-env`.

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
| `SlugInvalid` | Slug doesn't match `^[a-z0-9][a-z0-9-]{0,60}[a-z0-9]$` | Lowercase letters, numbers, hyphens; 2–62 chars |
| `SlotsExhausted` | All slots in use | `ecluse ls` then `ecluse down <slug>` |
| `SessionNotFound` | Slug not in state | Check `ecluse ls` |
| `LockTimeout` | Another process holds lock | Check processes; remove stale lock |
| `ConfigMissing` | No `.ecluse.toml` found | `ecluse init` |
| `NotAGitRepo` | Not in a git repo | `git init` first |
| `ComposeFileNotFound` | No compose file at repo root | Add compose file or switch mode |
| `PortInUse` | Port bound by another process | `kill <pid>` then retry |
| `HookFailed` | A hook (`pre_up`, `pre_spawn`, `post_up`, `pre_down`, `post_down`) exited non-zero | Check the hook command and its output |
| `ProcessManagerUnavailable` | Configured `process_manager` binary not installed | Install it or set `process_manager = "none"` in `~/.config/ecluse/config.toml` |
| `SpawnFailed` | Failed to spawn a native service process | Check the `command` field in `.ecluse.toml` and the binary's availability |

---

## Concurrency and recovery

`ecluse up`/`down` no longer hold the state lock while provisioning or tearing
down — sessions are reserved with a **pending** marker instead, so parallel
agents never block on each other's slow image pulls or hooks.

What this means for you:

- `ecluse ls` shows `<slug> (pending)` while an up/down is in flight
  (`"status": "pending"` in `ls --json`). Other read commands keep working.
- Running `up`, `env`, `status`, `shell`, or `sync` against a pending session
  fails with `operation in progress`. Wait for the owning command, or — if it
  crashed — run `ecluse down <slug>` to take the session over and clean it up.
- `ls` warns when a session has been pending for more than 15 minutes; that
  means the owning command died and the slot is leaked until you `down` it.
- If a session is removed (`down`, `flush`) while another command was still
  provisioning it, the loser detects the takeover, tears down whatever it
  created, and exits non-zero. State never resurrects deleted sessions.

### Service identity

Pid files record the process **start token** alongside the PID. A recycled PID
(same number, different process) is never killed, never attributed by
`whose-pid`, and reports as down in `status`. Containers are matched by their
compose project label, never by name substrings.

### tmux sessions

Services run as their tmux window's own process. A command that exits within
~1.5 s fails `ecluse up` with the exit status and last output — a "ready"
session means the services actually started. Dead panes are kept on screen
(`remain-on-exit`) so you can attach and read the error; window `shell` is a
plain shell with the session env loaded. Service commands must be
long-running: a command like `echo done` is treated as an instant failure
under tmux.

### --force and unowned ports

`ecluse up --force` only kills processes that **belong to the session**
(verified via pid files / tmux panes). A process squatting the session's port
that ecluse does not own produces a warning naming the PID instead of a kill —
inspect it with `ecluse whose-pid <pid>` and kill it manually if intended.

## Limits

What ecluse intentionally does not do in v0. These are design decisions, not bugs.

- **Ports are checked, not reserved** — ecluse finds a free port at `up` time and writes it to `.env.ecluse`. There is a small window between that check and when your process actually binds. If another process takes the port in between, the value in `.env.ecluse` will be wrong. Fix: `ecluse down feat-foo --keep-worktree` then `ecluse up feat-foo` (stopped session auto-detected, re-probes for a free port), or pin a specific port with `--port name=value`.
- **No process lifecycle management beyond spawn/kill** — ecluse can spawn native services on `up` (via `command` + `process_manager`) and kill them on `down`, but cannot auto-restart a crashed process. If a service dies, `ecluse up` (idempotent — slug auto-detected from cwd) starts only the downed services. `ecluse up --force` kills everything on allocated ports and restarts fresh. `ecluse ls` and `ecluse env` warn about dead nohup processes.
- **`command` requires the app to expose a port entry point** — ecluse injects the full `.env.ecluse` contents (`PORT`, `ECLUSE_SLOT`, `ECLUSE_SLUG`, `ECLUSE_MODE`, all `ECLUSE_<NAME>_PORT` vars, and any `port_env` aliases) directly into the spawned process environment — no separate sourcing step needed. If the port is hardcoded or set in a config file, resolve it via `.ecluse.toml` first: pass it as a CLI flag (`command = "vite --port $ECLUSE_WEB_PORT"`), or use `port_env` to inject it under the var name the app already reads. Modifying app source code is the last resort — see [Port wiring](#port-wiring--exhaust-eclusetoml-options-before-touching-app-code) above.
- **Mode is set at `init`, not re-detected on `up`** — to change: `ecluse init --mode <new>`
- **Multiple compose files supported via `compose` field** — each `[[services]]` block with `run = "docker"` can point at its own compose file; services without it fall back to the root compose file. Run `ecluse init` per subdirectory only when you need fully independent slot pools.
- **`localhost:<port>` only** — no public URLs; use cloudflared or ngrok alongside ecluse
- **No agent process sandboxing** — container mode isolates services, not the agent's filesystem
- **`ecluse shell` is for humans, not agents** — agents use `ecluse up --json` or `ecluse env` to get the worktree path and env vars, then operate directly; `ecluse shell` spawns an interactive subshell which blocks non-interactive execution
- **No built-in database management** — ecluse allocates ports and writes env vars; use `[hooks] post_up`/`pre_down` with your app's own tooling (prisma, rails db:create, psql, etc.)
- **macOS and Linux only** — WSL2 acceptable but untested; native Windows not supported
- **No background daemon** — every ecluse command is a short-lived process
- **No Ctrl+C rollback guarantee** — if killed mid-`up`, run `ecluse down <slug>` to clean partial state
- **Hooks run shell commands, not arbitrary plugins** — `[hooks]` in `.ecluse.toml` supports five lifecycle points (`pre_up`, `pre_spawn`, `post_up`, `pre_down`, `post_down`); each runs a shell command in the worktree with all env vars set; there is no plugin API or event bus beyond these. Hooks execute with the same privileges as the agent — only run `ecluse up`/`ecluse down` in repositories whose `.ecluse.toml` you trust
- **No telemetry** — no network calls except the optional Postgres TCP probe during `init`

---

## Configuration reference

`.ecluse.toml` (written by `ecluse init`, lives at repo root):

```toml
mode = "hybrid"         # container | host | hybrid
max_slots = 8           # max parallel sessions
prefix = "ecluse"       # prefix for compose project names and volume names
worktree_dir = ".ecluse/worktrees"
# app_label = "ecluse.role"  # compose label key that marks the app service in hybrid mode
# app_label_value = "app"    # value to match on that label

# Env file inheritance — materialized from repo root into each new worktree at ecluse up time.
# Default: [".env", ".env.local"] — active with no config needed, opt out with [].
# Each entry is either a bare string (mode = "symlink" by default) or an object
# { file = "...", mode = "symlink" | "copy" }.
#   - symlink (default): worktree file points to the root; edits propagate both ways.
#                        Good for shared secrets that should stay in sync.
#   - copy:              file is copied once on first ecluse up, then independent.
#                        Per-worktree edits stay local — never re-copied on subsequent ups.
#                        Use for feature flags / per-worktree overrides.
# inherit_env = [".env", ".env.local"]   # default — no need to set this explicitly
# inherit_env = []                       # opt out entirely
# inherit_env = [".env", ".env.staging"] # custom list for other env files
# inherit_env = [".env", { file = ".env.local", mode = "copy" }]  # mixed modes

# Port collision handling (all optional)
# strict_port = false        # default: search for a free port on collision
# port_search_range = 10     # how many alternatives to try (bump by max_slots × slot_stride each time)
#                            # Guard: port_search_range × max_slots × slot_stride must not exceed
#                            # the gap between adjacent service base_ports.
#                            # Run `ecluse validate` to check.
# slot_stride = 1            # spacing between adjacent slots' ports.
#                            # slot_stride = 1 (default): slots 1/2/3 → base+1, base+2, base+3
#                            # slot_stride = 10:          slots 1/2/3 → base+10, base+20, base+30
#                            # Wider stride makes adjacent-slot ports visually distinct in lsof
#                            # output and reduces the chance of agents misidentifying them.

# One [[services]] block per service.
# port = host_port_base + slot × slot_stride  (default stride 1: slot 1 → +1, slot 2 → +2)
# run = "native" (default) runs on host; run = "docker" runs in a container.
# The first native entry also sets the PORT alias for framework compatibility.
# Omit [[services]] entirely for single-service stacks — PORT = 3000 + slot.
# Add command = "..." to have ecluse spawn the process on ecluse up (native only).

[[services]]
name = "api"
base_port = 3000        # slot 1 → ECLUSE_API_PORT=3001 + PORT, slot 2 → 3002
command = "npm run dev" # optional — ecluse spawns this on ecluse up
#                       # omit for port-allocation-only: ecluse assigns the port and injects
#                       # env vars; you (or a task runner) start the process yourself
# port_env = "DJANGO_PORT"              # also set DJANGO_PORT = allocated port
# port_env = ["DJANGO_PORT", "APP_PORT"] # or multiple aliases
# extra_ports = [{ base_port = 9229, port_env = "NODE_INSPECT_PORT" }]
#   slot 1 → NODE_INSPECT_PORT=9230; for docker services also published as 9230:9229 in overlay
#   use for debugger ports, auxiliary listeners, or any secondary port the service exposes:
#   Node.js: command = "NODE_OPTIONS='--inspect=0.0.0.0:$NODE_INSPECT_PORT' npm run dev"
#   Delve:   command = "dlv debug --headless --listen=:$NODE_INSPECT_PORT ./cmd/api"

[[services]]
name = "postgres"
run = "docker"
base_port = 5432        # container-internal port (unchanged)
# host_port = 11532     # host range base — slot 1 → ECLUSE_POSTGRES_PORT=11533, slot 2 → 11534
#                       # overlay maps 11533→5432 (host_port+slot → base_port)
#                       # defaults to base_port when omitted (5433, 5434, …)
# port_env = "PGPORT"   # also set PGPORT = allocated host port (e.g. 11533)
# extra_ports = [{ base_port = 11533, port_env = "PGPORT" }]
#   slot 1 → PGPORT=11534 in compose env; also published as 11534:11533 in the overlay
# compose = "services/postgres/docker-compose.yml"  # optional: per-service compose file

# Optional: lifecycle hooks — run in the worktree with all env vars set
[hooks]
pre_up = "..."           # before any infrastructure is created (no env vars yet)
pre_spawn = "..."        # after .env.ecluse is written, before native services spawn; full ECLUSE_* env available
#                        # use to compute derived values (URLs, connection strings) that depend on
#                        # allocated ports but must exist before the process starts
post_up = "npx prisma migrate deploy"   # after all services are up
pre_down = "npx prisma migrate reset --force"  # before services are killed (all env vars set)
post_down = "..."        # after worktree is removed
```

### Global config (`~/.config/ecluse/config.toml`)

Controls how native service processes are spawned. Written by `ecluse init`:

```toml
process_manager = "tmux"   # "tmux" | "nohup" | "none"
```

- `tmux` — creates a detached tmux session `ecluse-<slug>`; `ecluse shell [<slug>]` attaches to it
- `nohup` — background processes, logs at `.ecluse/logs/<slug>/`, PIDs at `.ecluse/pids/<slug>/`
- `none` — spawns nothing (default pre-v0.3 behaviour)

`ecluse init` auto-detects: tmux if present, otherwise nohup. `ecluse validate` checks the binary is installed. This is per-machine, not per-repo.

Hooks run as shell commands inside the worktree directory. `pre_up` runs before any infrastructure exists (env vars not yet available). `pre_spawn` runs after `.env.ecluse` is written but before native services are started — use it to derive env values from allocated ports (e.g. `CORE_API_URL=http://localhost:$ECLUSE_API_PORT`). `post_up`, `pre_down`, and `post_down` all have the full `.env.ecluse` set (`PORT`, `ECLUSE_SLUG`, `ECLUSE_<NAME>_PORT`, etc.). ecluse does not manage databases directly — use `post_up` for migrations and `pre_down` for teardown.

## Examples

See [examples.md](examples.md) for 5 canonical config templates covering host, container, hybrid, multi-service monorepo, and Kubernetes. Each entry links directly to the `.ecluse.toml` and `docker-compose.yml` you can read and adapt.

## Commands

```
ecluse init [--mode container|host|hybrid] [--explain] [--yes]
ecluse up [<slug>] [--branch <name>] [--watch] [--json] [--reuse-worktree] [--port <name>=<value>] [--services <name>,...] [--force] [--skip <name>,...]
ecluse env [<slug>]
ecluse down [<slug>] [--keep-volumes] [--keep-branch] [--keep-worktree] [--delete-worktree]
ecluse ls [--json]
ecluse validate [--ports]
ecluse status [<slug>] [--json] [--quiet]
ecluse whose-pid <pid> [--json]
```

`ecluse shell` exists but is human-only — it spawns an interactive subshell that blocks non-interactive execution. Agents must not use it.

`ecluse validate` checks your `.ecluse.toml` for port range safety (ensures `port_search_range` doesn't create overlaps between services) and prints the current config. Pass `--ports` to see the full port allocation table across all slots.

`ecluse status` checks whether each service is actually running. For native services it matches running processes by command line; for docker services it queries `docker ps`. Exits with code 1 if any service is down — useful in CI pre-flight or as a readiness gate after `ecluse up`. Use `--json` for machine-readable output:

```bash
ecluse status feat-foo           # human table: ✓/✗ per service with port and PID
ecluse status feat-foo --json    # { "all_healthy": true/false, "services": [...] }
ecluse status --quiet            # exit-code only (0 = all up, 1 = any down)
```

**Soft restart** — tear down services without losing the git worktree, then spin up fresh:

```bash
ecluse down feat-foo --keep-worktree   # stops services, marks the session Stopped (slot reserved), keeps worktree on disk
ecluse up feat-foo                      # resumes at the same slot (stopped session auto-detected; --reuse-worktree not needed)
```

Use this when a service failed to bind after `up` and you want a fresh start without losing changes in the worktree. While stopped, `ecluse env`, `ecluse shell`, and `ecluse status` on that session error with a hint to run `ecluse up` — they will not surface stale, no-longer-running ports.

**Port override** — pin a service to a specific port for this session:

```bash
ecluse up feat-foo --port api=4001 --port postgres=5444
```

Overrides bypass the auto-bump logic and use the given value directly. The overridden ports are stored in session state and reflected in `ecluse env` output.
