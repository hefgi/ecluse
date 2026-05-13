# Hybrid mode setup

In hybrid mode, data services (Postgres, Redis, etc.) run in containers with per-slot ports. Your app runs natively.

## Option 1: label in docker-compose.yml

Add `ecluse.role: app` to your app service:

```yaml
services:
  web:
    build: .
    labels:
      ecluse.role: app
    ports: ["3000:3000"]
  postgres:
    image: postgres:16   # no label = data service = containerized
  redis:
    image: redis:7
```

ecluse starts `postgres` and `redis` in containers with per-slot ports. Your app (`web`) is skipped — you run it yourself.

## Option 2: explicit run = "docker" in .ecluse.toml

Define which services stay in containers directly in the config:

```toml
mode = "hybrid"

[[services]]
name = "api"
base_port = 3000        # native — runs on host

[[services]]
name = "postgres"
run = "docker"
base_port = 5432        # containerized

[[services]]
name = "redis"
run = "docker"
base_port = 6379        # containerized
```

Either approach works — the label is simpler for single-app stacks. The config approach gives more control for monorepos.

## After ecluse up

`ecluse up feat-foo` starts postgres and redis in containers with per-slot ports, creates the worktree, and writes `.env.ecluse`:

```
PORT=3001
ECLUSE_API_PORT=3001
ECLUSE_POSTGRES_PORT=5433
ECLUSE_REDIS_PORT=6380
```

You start your app with these env vars already set:

```bash
ecluse shell feat-foo
npm run dev   # connects to ECLUSE_POSTGRES_PORT automatically
```
