# node-hybrid

Express API + React frontend with Postgres, running in hybrid mode.

Postgres runs in a Docker container managed by ecluse. The Express API and React dev server run natively. Each worktree gets its own isolated Postgres database on an offset port.

## Mode

`hybrid` — Postgres containerized, app runs natively.

## Services

| Service  | Role         | Label              |
|----------|--------------|--------------------|
| postgres | data         | —                  |
| web      | app (Express)| `ecluse.role: app` |

## Environment variables set by ecluse

| Variable               | Description                              |
|------------------------|------------------------------------------|
| `ECLUSE_SLUG`          | Session slug                             |
| `PORT`                 | App port (`base_port + slot * stride`)   |
| `DATABASE_URL`         | `postgres://localhost:<offset_port>/postgres` |
| `ECLUSE_POSTGRES_PORT` | Offset host port for Postgres            |

## Hooks

- `on_up`: runs `npm run db:migrate` (wraps `prisma migrate deploy`) to apply pending migrations against the slot's database.

## Usage

```sh
ecluse init
ecluse up my-feature
ecluse shell my-feature

# Inside the session shell
npm run dev        # starts Express on $PORT
# In another terminal (same worktree)
cd frontend && npm run dev

ecluse down my-feature
```
