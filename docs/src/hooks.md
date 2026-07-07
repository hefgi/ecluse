# Hooks

Hooks run shell commands at lifecycle points. Define them in `.ecluse.toml`:

```toml
[hooks]
pre_up    = "echo starting"
pre_spawn = "envsubst < .env.template > .env.local"
post_up   = "npx prisma migrate deploy"
pre_down  = "npx prisma migrate reset --force"
post_down = "echo done"
```

## Lifecycle order

```
ecluse up
  └─ pre_up    → repo root, NO env vars yet
  └─ [ports allocated, docker services started, worktree created,
      .env.ecluse written to worktree]
  └─ pre_spawn → worktree root, FULL env — native services not yet started
  └─ [native services spawn via tmux/nohup]
  └─ post_up   → worktree root, full env, all services running

ecluse down
  └─ pre_down  → worktree root, full env (services still running)
  └─ [services stopped, worktree removed]
  └─ post_down → repo root, env vars still available
```

## pre_up

Runs before any infrastructure is created. Working directory is the repo root. **No `ECLUSE_*` variables are available yet** — ports haven't been allocated, the worktree doesn't exist, no docker containers are up.

Use it for: pre-flight checks that don't need slot info (`command -v pnpm`, disk-space checks, image pulls that should happen before slot reservation).

## pre_spawn

Runs after ports are allocated and `.env.ecluse` is written, but **before native services are spawned**. Working directory is the worktree root. All `ECLUSE_*` variables are available, docker data services are up.

This is the hook for **slot-aware setup that must complete before your app boots**. Because it runs before native services start, anything you write to disk here will be present when the service reads it during startup — `post_up` is too late for that, since the service has already read its config.

Use it for:
- Writing per-worktree `.env.local` or `.dev.vars` with slot-specific URLs (e.g. `API_URL=http://localhost:$ECLUSE_API_PORT`) that a framework reads once at boot
- Rewriting an env file to substitute slot-derived values in place
- Waiting for postgres to accept queries (the docker container may be started but not yet ready)
- Applying database migrations that must exist before app code runs
- Generating client code (`prisma generate`) that services import at boot
- Installing dependencies (`pnpm install`) before services try to resolve them
- Setting up symlinks / overlay files that services read at startup

**Why not just use `post_up`?** Because a service that reads its config once at startup (Cloudflare vite plugin, most `dotenv` loaders, any framework using `sh -c 'export ... && ...'`) will see whatever the file contained before the hook ran — and then never re-read it. `post_up` fires after that point.

## post_up

Runs after all services are up and running. Working directory is the worktree root. All `ECLUSE_*` variables are available.

Use it for:
- Post-boot actions that need running services (curl a health endpoint, warm a cache)
- Sync-only setup that doesn't affect service startup env (some migration workflows against an already-running DB)
- Notifications, dashboard updates

**Prefer `pre_spawn`** when you're writing files the services will read at boot — `post_up` runs too late for that.

## pre_down

Runs before services are killed or containers are stopped. Working directory is the worktree root. All `ECLUSE_*` variables are available.

Use it for:
- Draining connections
- Resetting database state while the database is still running
- Recording final metrics before teardown

## post_down

Runs after all services are stopped and the worktree is removed. Working directory is the repo root. Env vars from the session are still available.

Use it for: cleanup that should happen after everything is gone (notifications, CI status updates, etc.).

## Environment

| Hook | Working dir | Env vars | Services state |
|---|---|---|---|
| `pre_up` | repo root | none | nothing exists yet |
| `pre_spawn` | worktree root | all `ECLUSE_*` + `PORT` | docker up, native not started |
| `post_up` | worktree root | all `ECLUSE_*` + `PORT` | everything running |
| `pre_down` | worktree root | all `ECLUSE_*` + `PORT` | everything still running |
| `post_down` | repo root | all `ECLUSE_*` + `PORT` | everything torn down |

## Examples

### Prisma migrations

Migrations don't affect service startup env, so `post_up` is fine:

```toml
[hooks]
post_up  = "npx prisma migrate deploy"
pre_down = "npx prisma migrate reset --force"
```

### Injecting slot-specific URLs before service boot

A frontend that reads `VITE_API_URL` at boot (Vite, Next.js, Cloudflare workers): the URL depends on the api service's allocated port, which only exists once ports are reserved. Write the file in `pre_spawn` so the frontend picks it up:

```toml
[[services]]
name = "api"
base_port = 4444
port_env = "ECLUSE_API_PORT"
command = "pnpm --filter api dev --port $ECLUSE_API_PORT"

[[services]]
name = "web"
base_port = 3000
port_env = "ECLUSE_WEB_PORT"
command = "pnpm --filter web dev --port $ECLUSE_WEB_PORT"

[hooks]
pre_spawn = """
cat > apps/web/.env.development.local <<EOF
VITE_API_URL=http://localhost:$ECLUSE_API_PORT
EOF
"""
```

The web service reads `.env.development.local` at boot and dials the correct per-slot api URL. Using `post_up` here would produce the wrong URL — the web service would have already booted with whatever was in the file before the hook ran.

### Waiting for postgres, then migrating, before services boot

`docker compose up` returns when the container has *started*, not when postgres is ready to accept queries. Services that fail-fast on the first connection attempt (Go binaries, most ORMs' initial pool ping) need postgres actually up before they boot:

```toml
[hooks]
pre_spawn = """
set -e
for i in 1 2 3 4 5 6 7 8 9 10; do
  if pg_isready -h localhost -p "$ECLUSE_POSTGRES_PORT" -U app >/dev/null 2>&1; then break; fi
  sleep 1
done
npx prisma migrate deploy
npx prisma generate
"""
```

### Rewriting `.env.local` per worktree

If `.env.local` holds slot-specific URLs (DB, Redis, API endpoints), the default `inherit_env` symlink would leak every `ecluse up`'s rewrite into every other worktree. Use `mode = "copy"` so each worktree has its own file, then rewrite it in `pre_spawn`:

```toml
inherit_env = [
  ".env",
  { file = ".env.local", mode = "copy" },
]

[hooks]
pre_spawn = """
awk -v pgport="$ECLUSE_POSTGRES_PORT" '
  /^DATABASE_URL=/ { print "DATABASE_URL=postgres://app@localhost:" pgport "/app"; next }
  { print }
' .env.local > .env.local.tmp && mv .env.local.tmp .env.local
"""
```

### Rails

```toml
[hooks]
post_up  = "bundle exec rails db:migrate"
pre_down = "bundle exec rails db:drop"
```

### Multiple commands

Chain with `&&` (fail fast) or `;` (continue on error). For long blocks, use a TOML multi-line string:

```toml
[hooks]
pre_spawn = """
set -e
pnpm install
pnpm run --filter=@app/prisma prisma generate
pnpm run --filter=@app/prisma prisma migrate deploy
"""
```

## Which hook when

| Situation | Hook |
|---|---|
| Pre-flight check that doesn't need slot info | `pre_up` |
| Write per-slot config file a service reads at boot | `pre_spawn` |
| Wait for a docker service to accept queries | `pre_spawn` |
| Generate client code services import at boot | `pre_spawn` |
| Install dependencies before services try to use them | `pre_spawn` |
| Run migration against an already-running DB (service doesn't care) | `post_up` |
| Curl a health endpoint after boot | `post_up` |
| Drain connections before teardown | `pre_down` |
| Wipe DB state before docker stops it | `pre_down` |
| Send notification after tear-down | `post_down` |

The rule of thumb: **if a service reads the thing you're setting up at boot, use `pre_spawn`. Otherwise `post_up`.**

## Deprecated field names

`on_up` and `on_down` still work as aliases for `pre_up` and `pre_down` respectively, but are deprecated. Migrate to the five-field form above.
