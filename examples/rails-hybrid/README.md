# rails-hybrid

Rails API + Angular frontend with Postgres and Redis, running in hybrid mode.

Data services (postgres, redis) run in Docker containers managed by ecluse. The Rails API and Angular dev server run natively on the host. Each worktree gets its own isolated Postgres database and Redis instance on offset ports.

## Mode

`hybrid` — data services containerized, app processes run natively.

## Services

| Service  | Role        | Label              |
|----------|-------------|--------------------|
| postgres | data        | —                  |
| redis    | data        | —                  |
| web      | app (Rails) | `ecluse.role: app` |
| angular  | app (ng)    | `ecluse.role: app` |

## Environment variables set by ecluse

| Variable               | Description                              |
|------------------------|------------------------------------------|
| `ECLUSE_SLUG`          | Session slug (e.g. `my-feature`)         |
| `PORT`                 | Rails port (`base_port + slot`, e.g. 3001 for slot 1) |
| `ECLUSE_POSTGRES_PORT` | Per-slot host port for Postgres          |
| `ECLUSE_REDIS_PORT`    | Per-slot host port for Redis             |

## Hooks

- `on_up`: runs `bin/rails db:prepare` — creates and migrates the database for this slot.
- `on_down`: runs `bin/rails db:drop` — drops the database before tearing down.

## Usage

```sh
# First-time setup
ecluse init

# Start a new isolated session
ecluse up my-feature

# Open a shell with the session env loaded
ecluse shell my-feature

# Start the Rails API (in the session shell or worktree)
bin/rails server -p $PORT

# Start the Angular frontend (separate terminal, same worktree)
npm start

# Tear down
ecluse down my-feature
```
