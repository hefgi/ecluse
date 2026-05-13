---
name: ecluse-hybrid-mode
description: >
  Use this skill when the user is working with ecluse in hybrid mode,
  has a compose file with only data services (postgres, redis, etc.) and
  runs their app natively, asks how to set up the ecluse.role label,
  or wants data isolation with native app speed.
tags:
  - ecluse
  - hybrid
  - docker
  - compose
  - postgres
---

# Hybrid Mode

## What it does

Data services (postgres, redis, etc.) run in containers with offset ports and namespaced volumes. Your app runs on the host. ecluse bridges them via `.env.ecluse` — the app reads `DATABASE_URL` and `REDIS_URL` pointing at the containerized services.

This is the fastest dev loop: data isolated, app native (hot reload, native debugger, no container layer).

## Prerequisites

- Docker installed and running
- `docker-compose.yml` with data services
- App service labeled or no app service in compose at all
- `ecluse init --mode hybrid` (or auto-detected)

## Label your app service

Add `ecluse.role: app` to any service that should run on the host, not in a container:

```yaml
services:
  web:
    build: .
    labels:
      ecluse.role: app
    ports: ["3000:3000"]   # ecluse reads this to assign the host-side app port
  postgres:
    image: postgres:16     # no label = data service = gets containerized
  redis:
    image: redis:7
```

`ecluse up feat-foo` will start `postgres` and `redis` in containers. `web` is **not started**.

## Workflow

```bash
ecluse up feat-foo
# Data containers start automatically
# Output:
#   App port:   3100
#   Database:   postgres://localhost:5532/...
#   Next step:  cd worktree && source .env.ecluse

cd .ecluse/worktrees/feat-foo
source .env.ecluse
bin/dev       # or npm run dev — app reads PORT, DATABASE_URL, REDIS_URL from env
```

## Without the label

If no service is labeled `ecluse.role: app`, ecluse prints a warning and treats all services as data — behaving like container mode minus the app service. This is graceful degradation, not an error.

## Teardown

```bash
ecluse down feat-foo                  # stops data containers, removes worktree + volumes
ecluse down feat-foo --keep-volumes   # keeps volumes for inspection
```

## See also

- [choosing-a-mode](../choosing-a-mode/SKILL.md)
- [agent-workflow](../agent-workflow/SKILL.md)
- [container-mode](../container-mode/SKILL.md)
