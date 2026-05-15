# Hooks

Hooks run shell commands at lifecycle points. Define them in `.ecluse.toml`:

```toml
[hooks]
pre_up   = "echo starting"
post_up  = "npx prisma migrate deploy"
pre_down = "npx prisma migrate reset --force"
post_down = "echo done"
```

## Lifecycle order

```
ecluse up
  └─ pre_up    → runs from repo root, no env vars yet
  └─ [services start, worktree created, .env.ecluse written]
  └─ post_up   → runs from worktree root, full env available

ecluse down
  └─ pre_down  → runs from worktree root, full env available (services still running)
  └─ [services stopped, worktree removed]
  └─ post_down → runs from repo root, env vars still available
```

## pre_up

Runs before any infrastructure is created. Working directory is the repo root. No `ECLUSE_*` variables are available yet.

Use it for: pre-flight checks, pulling images, anything that must happen before services start.

## post_up

Runs after all services are up and `.env.ecluse` is written. Working directory is the worktree root. All `ECLUSE_*` variables are available.

Use it for:
- Database migrations
- Seeding
- Any setup your app needs before it can run

## pre_down

Runs before services are killed or containers are stopped. Working directory is the worktree root. All `ECLUSE_*` variables are available.

Use it for:
- Draining connections
- Resetting database state while the database is still running

## post_down

Runs after all services are stopped and the worktree is removed. Working directory is the repo root. Env vars from the session are still available.

Use it for: cleanup that should happen after everything is gone (notifications, CI status updates, etc.).

## Environment

All hooks receive the session's env vars except `pre_up` (which runs before anything exists):

| Hook | Working dir | Env vars |
|---|---|---|
| `pre_up` | repo root | none |
| `post_up` | worktree root | all `ECLUSE_*` + `PORT` |
| `pre_down` | worktree root | all `ECLUSE_*` + `PORT` |
| `post_down` | repo root | all `ECLUSE_*` + `PORT` |

## Examples

### Prisma migrations

```toml
[hooks]
post_up  = "npx prisma migrate deploy"
pre_down = "npx prisma migrate reset --force"
```

### Rails

```toml
[hooks]
post_up  = "bundle exec rails db:migrate"
pre_down = "bundle exec rails db:drop"
```

### Multiple commands

```toml
[hooks]
post_up = "npx prisma migrate deploy && npx prisma db seed"
```

## Deprecated field names

`on_up` and `on_down` still work as aliases for `pre_up` and `pre_down` respectively, but are deprecated. Migrate to the four-field form above.
