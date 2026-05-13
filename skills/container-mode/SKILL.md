---
name: ecluse-container-mode
description: >
  Use this skill when the user is working with ecluse in container mode,
  asks how container mode works, has a compose file with build: . or a
  fully containerized stack, or encounters issues with port mapping or
  volume namespacing in container mode.
tags:
  - ecluse
  - container
  - docker
  - compose
---

# Container Mode

## What it does

Every service in your `docker-compose.yml` — including your app — runs in Docker under a unique compose project per session. Ports are offset by `slot × stride`; named volumes are namespaced per session. Nothing leaks between sessions.

## Prerequisites

- Docker installed and daemon running (`docker info` exits 0)
- `docker-compose.yml` or `compose.yaml` at the repo root
- `ecluse init --mode container` (or auto-detected)

## How ports are offset

With `stride = 100` and a compose file exposing `web` on `3000:3000` and `postgres` on `5432:5432`:

| Session | Slot | web (host) | postgres (host) |
|---|---|---|---|
| `feat-foo` | 1 | 3100 | 5532 |
| `fix-bar` | 2 | 3200 | 5632 |

The container-side port is never changed — only the host-side binding.

## How volumes are namespaced

Named volume `db_data` becomes:
- `db_data_ecluse_feat-foo` for slot 1
- `db_data_ecluse_fix-bar` for slot 2

Bind mounts (`./src:/app`) are not renamed — they already point into the worktree.

## What ecluse generates

For each session, ecluse writes an overlay file at `.ecluse/overlays/<slug>.yml` that docker compose merges on top of your original file. Your `docker-compose.yml` is never modified.

## Workflow

```bash
ecluse up feat-foo          # starts all containers, creates worktree
# Access the app at http://localhost:3100

ecluse down feat-foo        # stops containers, removes worktree + volumes
ecluse down feat-foo --keep-volumes   # stops containers, keeps volumes
```

## Common pitfalls

- **Hardcoded `localhost:3000` in app code** — if your app calls itself or another service by hardcoded host port, update it to read from env vars. ecluse writes `ECLUSE_<SERVICE>_PORT` for each service.
- **`--watch` requires compose v2.22+** — pass `ecluse up --watch` to enable `docker compose watch`.
- **Compose file uses hardcoded external ports** — ecluse offsets the numeric host port directly. Works for most cases; fails if the host port is referenced inside the container.

## See also

- [choosing-a-mode](../choosing-a-mode/SKILL.md)
- [troubleshooting](../troubleshooting/SKILL.md)
