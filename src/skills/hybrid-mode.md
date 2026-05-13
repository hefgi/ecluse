# Hybrid Mode

## When to use

Your data services (postgres, redis, etc.) run in Docker for isolation, but your application runs natively on the host for speed. This is the pragmatic middle: data isolated, app fast.

Common indicator: your README says "run `docker compose up`, then `bin/dev`."

## Prerequisites

- Docker installed and running
- `docker-compose.yml` with data services
- `ecluse init --mode hybrid` has been run

## What ecluse does in hybrid mode

1. Parses your compose file and partitions services into `app` and `data`.
2. Generates a port-offset overlay for data services only.
3. Runs `docker compose ... up -d <data-services>` — app services are **not started**.
4. Creates a git worktree.
5. Writes `.env.ecluse` with `PORT` (for your host app), `DATABASE_URL`, service ports, etc.
6. Prints next-step hint for starting your app.

## Labeling app services

Add the `ecluse.role: app` label to services that should run on the host:

```yaml
services:
  web:
    build: .
    labels:
      ecluse.role: app
    ports: ["3000:3000"]  # ecluse reads this for PORT assignment
  postgres:
    image: postgres:16
    # no label = data service, gets containerized
  redis:
    image: redis:7
    # no label = data service
```

With this config:
- `ecluse up feat-foo` starts `postgres` and `redis` in containers with offset ports
- `web` is NOT started
- `.env.ecluse` sets `PORT=3100`, `DATABASE_URL=postgres://localhost:5532/...`

## Without labels

If no service is labeled, ecluse treats all services as data and prints a warning:

```
WARNING: No service labeled ecluse.role=app found.
All services will run as data services.
Add the label to your app service for proper hybrid behavior.
```

The session still works — it behaves like container mode minus the app service.

## Typical workflow

```bash
ecluse up feat-foo
# Data containers start automatically

cd .ecluse/worktrees/feat-foo
source .env.ecluse
bin/dev  # or npm run dev, python manage.py runserver, etc.
# Your app uses $PORT and $DATABASE_URL from .env.ecluse
```

## Why hybrid is often fastest

- Your app code reloads via native file watchers (no container layer)
- Debuggers attach directly (no remote debugging setup)
- Database migrations run locally with full output
- Data is still isolated — each session has its own postgres/redis namespace

## Teardown

```bash
ecluse down feat-foo              # stops data containers, removes worktree + volumes
ecluse down feat-foo --keep-volumes  # keeps data volumes for inspection
```

## See also

- `ecluse skills show container-mode` — when you need full containerization
- `ecluse skills show host-mode` — when you don't want containers at all
- `ecluse skills show agent-workflow` — canonical agent loop
