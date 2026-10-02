# t3-monorepo

T3 monorepo (Turborepo + Next.js + tRPC + Prisma + BullMQ) in hybrid mode.

Data services (Postgres, Redis) run in Docker with per-slot ports. All four app services run natively on the host — each on its own dedicated `base_port` in `[[services]]`.

## Monorepo structure

```
apps/
  api/      tRPC server (Next.js API routes or standalone Express)
  web/      Next.js frontend
  worker/   BullMQ worker + bull-board admin UI
  email/    React Email preview server
packages/
  db/       Prisma schema + client (shared)
  ui/       Shared component library
```

## Ports (slot 1 example, `port = base_port + 1`)

| Variable              | Port | Service                  |
|-----------------------|------|--------------------------|
| `ECLUSE_API_PORT`     | 3001 | tRPC / REST API + PORT alias |
| `ECLUSE_WEB_PORT`     | 3101 | Next.js frontend         |
| `ECLUSE_WORKER_PORT`  | 3201 | BullMQ board UI          |
| `ECLUSE_EMAIL_PORT`   | 3301 | React Email preview      |
| `ECLUSE_POSTGRES_PORT`| 5433 | Postgres (from compose)  |
| `ECLUSE_REDIS_PORT`   | 6380 | Redis (from compose)     |

`PORT` is an alias for `ECLUSE_API_PORT` (the first native `[[services]]` entry).

## Usage

```sh
ecluse up my-feature
ecluse shell my-feature

# Inside the session shell — start all services via turbo
npm run dev
# or start individually (slot 1 example):
# npx next dev apps/api -p $ECLUSE_API_PORT
# npx next dev apps/web -p $ECLUSE_WEB_PORT
# node apps/worker/src/index.js  (reads ECLUSE_WORKER_PORT)
# npx email dev --port $ECLUSE_EMAIL_PORT

ecluse down my-feature
```

## App config

Each app reads its port from the matching env var. Example for `apps/web`:

```js
// apps/web/next.config.js
const port = process.env.ECLUSE_WEB_PORT ?? 3000;
```

Cross-service URLs (`NEXT_PUBLIC_API_URL` for the web, `INTERNAL_API_URL` for the worker) are written into `.env.development.local` by the `pre_spawn` hook, **before** any app boots. Every Next.js service reads these at startup — using `post_up` would fire too late (the apps have already read the file). The file is generated inside each worktree, so the symlinked `.env` / `.env.local` (and their secrets) are never overwritten and sibling worktrees can't clobber each other.

## Hooks

- `pre_spawn`: writes `.env.development.local` with slot-derived `DATABASE_URL`, `REDIS_URL`, `NEXT_PUBLIC_API_URL`, `NEXT_PUBLIC_WEB_URL`, and `INTERNAL_API_URL`; waits for postgres to accept queries; applies Prisma migrations. Everything runs before any app service boots, so every process reads the correct per-slot config and finds the schema in place. The Prisma CLI only reads `.env`, so the hook exports `DATABASE_URL` for it explicitly.
- `pre_down`: wipes the slot's database on teardown (again passing `DATABASE_URL` explicitly) (drop this if you want to keep data across down/up cycles).
