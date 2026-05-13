# t3-monorepo

T3 monorepo (Turborepo + Next.js + tRPC + Prisma + BullMQ) in hybrid mode.

Data services (Postgres, Redis) run in Docker with per-slot offset ports. All four app services run natively on the host — each on its own dedicated port from the `[ports]` table.

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

## Ports (slot 1 example)

| Variable              | Port | Service                  |
|-----------------------|------|--------------------------|
| `ECLUSE_API_PORT`     | 3100 | tRPC / REST API          |
| `ECLUSE_WEB_PORT`     | 3101 | Next.js frontend         |
| `ECLUSE_WORKER_PORT`  | 3102 | BullMQ board UI          |
| `ECLUSE_EMAIL_PORT`   | 3103 | React Email preview      |
| `ECLUSE_POSTGRES_PORT`| 5532 | Postgres (from compose)  |
| `ECLUSE_REDIS_PORT`   | 6479 | Redis (from compose)     |

`PORT` is an alias for `ECLUSE_API_PORT` (the first `[ports]` entry).

## Usage

```sh
ecluse up my-feature
ecluse shell my-feature

# Inside the session shell — start all services via turbo
npm run dev
# or start individually:
# ECLUSE_API_PORT=3100 npx next dev apps/api -p $ECLUSE_API_PORT
# ECLUSE_WEB_PORT=3101 npx next dev apps/web -p $ECLUSE_WEB_PORT
# ECLUSE_WORKER_PORT=3102 node apps/worker/src/index.js
# ECLUSE_EMAIL_PORT=3103 npx email dev --port $ECLUSE_EMAIL_PORT

ecluse down my-feature
```

## App config

Each app reads its port from the matching env var. Example for `apps/web`:

```js
// apps/web/next.config.js
const port = process.env.ECLUSE_WEB_PORT ?? 3000;
```

And for the API to know where the frontend lives (CORS, redirects):

```env
NEXT_PUBLIC_API_URL=http://localhost:${ECLUSE_API_PORT}
NEXT_PUBLIC_WEB_URL=http://localhost:${ECLUSE_WEB_PORT}
```
