# fastapi-hybrid

FastAPI backend + Vue frontend with Postgres, running in hybrid mode.

Postgres runs in a Docker container managed by ecluse. The FastAPI process and Vite dev server run natively. Each worktree gets its own isolated Postgres database on an offset port.

## Mode

`hybrid` — Postgres containerized, app processes run natively.

## Services

| Service   | Role       | Label              |
|-----------|------------|--------------------|
| postgres  | data       | —                  |
| api       | app (FastAPI) | `ecluse.role: app` |
| frontend  | app (Vite) | `ecluse.role: app` |

## Environment variables set by ecluse

| Variable               | Description                              |
|------------------------|------------------------------------------|
| `ECLUSE_SLUG`          | Session slug                             |
| `PORT`                 | API port (`base_port + slot`, e.g. 8001 for slot 1) |
| `ECLUSE_POSTGRES_PORT` | Per-slot host port for Postgres          |
| `ECLUSE_REDIS_PORT`    | Per-slot host port for Redis             |

## Hooks

- `post_up`: runs `alembic upgrade head` to apply pending migrations against the slot's database.

## Usage

```sh
ecluse init
ecluse up my-feature
ecluse shell my-feature

# Inside the session shell
uvicorn app.main:app --host 0.0.0.0 --port $PORT --reload
# In another terminal (same worktree)
cd frontend && npm run dev

ecluse down my-feature
```
