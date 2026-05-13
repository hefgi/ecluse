# ecluse config examples

Ready-to-use `.ecluse.toml` templates for common stacks. Read the files in this directory directly — they are the authoritative reference. Each directory contains the `.ecluse.toml` and, where relevant, a `docker-compose.yml` for the data services.

## Index

| Directory | Mode | Stack | `[ports]` entries |
|---|---|---|---|
| [rails-hybrid](examples/rails-hybrid/.ecluse.toml) | hybrid | Rails 7 + Angular + Postgres + Redis | `api`, `frontend` |
| [rails-monorepo](examples/rails-monorepo/.ecluse.toml) | hybrid | Rails 7 + Sidekiq + Blazer + Postgres + Redis | `web`, `sidekiq`, `admin` |
| [node-hybrid](examples/node-hybrid/.ecluse.toml) | hybrid | Express + React + Postgres | `api`, `frontend` |
| [node-container](examples/node-container/.ecluse.toml) | container | Node.js fully containerized | from compose |
| [nextjs-hybrid](examples/nextjs-hybrid/.ecluse.toml) | hybrid | Next.js + Prisma + Postgres | `app` |
| [t3-host](examples/t3-host/.ecluse.toml) | host | T3 (Next.js + tRPC + Prisma), no Docker | `app` |
| [t3-monorepo](examples/t3-monorepo/.ecluse.toml) | hybrid | Turborepo: API + Web + Worker + Email + Postgres + Redis | `api`, `web`, `worker`, `email` |
| [fastapi-hybrid](examples/fastapi-hybrid/.ecluse.toml) | hybrid | FastAPI + Vue + Postgres | `api`, `frontend` |
| [go-hybrid](examples/go-hybrid/.ecluse.toml) | hybrid | Go HTTP server + Postgres | `api` |
| [mongo-hybrid](examples/mongo-hybrid/.ecluse.toml) | hybrid | Node.js + MongoDB | `api` |
| [k3d](examples/k3d/.ecluse.toml) | host | Kubernetes via k3d (ingress only) | `http`, `https` |

## Patterns

**Single service** — one port, `PORT` alias set automatically:
```toml
[ports]
app = 0
```

**Backend + frontend** — two ports, each app reads its own var:
```toml
[ports]
api      = 0   # ECLUSE_API_PORT + PORT alias
frontend = 1   # ECLUSE_FRONTEND_PORT
```

**Full monorepo** — four ports, one per independently-started process:
```toml
[ports]
api     = 0   # ECLUSE_API_PORT + PORT alias
web     = 1   # ECLUSE_WEB_PORT
worker  = 2   # ECLUSE_WORKER_PORT
email   = 3   # ECLUSE_EMAIL_PORT
```

**Kubernetes / k3d** — two ingress host ports only; all services communicate inside the cluster via DNS:
```toml
[ports]
http  = 0   # ECLUSE_HTTP_PORT + PORT alias → maps to :80 inside cluster
https = 1   # ECLUSE_HTTPS_PORT → maps to :443 inside cluster
```

## How ports are resolved

`base_port + slot × stride + index`

With defaults (`base_port = 3000`, `stride = 100`):
- slot 1: index 0 → 3100, index 1 → 3101, index 2 → 3102
- slot 2: index 0 → 3200, index 1 → 3201, index 2 → 3202

The first entry always sets the `PORT` alias for framework compatibility.
Omit `[ports]` entirely for single-service stacks — `PORT` is set to `base_port + slot × stride` automatically.
