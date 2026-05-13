# Container Mode

## When to use

Your entire dev stack — app and data services — runs in Docker. Your `docker-compose.yml` has a service with `build: .` or all services should be containerized together.

## Prerequisites

- Docker installed and daemon running
- `docker-compose.yml` or `compose.yaml` at the repo root
- `ecluse init --mode container` has been run

## What ecluse does in container mode

1. Allocates a slot (integer 1–N) and computes `offset = slot × stride`.
2. Generates a compose overlay file at `.ecluse/overlays/<slug>.yml` that:
   - Adds `offset` to every host port mapping (so `3000:3000` becomes `3100:3000` for slot 1 with stride 100)
   - Namespaces all named volumes as `<volume>_ecluse_<slug>`
3. Runs `docker compose -p ecluse_<slug> -f docker-compose.yml -f overlay.yml up -d`
4. Creates a git worktree at `.ecluse/worktrees/<slug>`
5. Writes `.env.ecluse` to the worktree with slot, offset, mode, and service URLs

## Port offset example

With stride=100 and `docker-compose.yml`:
```yaml
services:
  web:
    ports: ["3000:3000"]
  postgres:
    ports: ["5432:5432"]
```

| Session | Slot | web port | postgres port |
|---|---|---|---|
| feat-foo | 1 | 3100 | 5532 |
| fix-bar | 2 | 3200 | 5632 |

## Volume namespacing example

Named volume `db_data` becomes:
- `db_data_ecluse_feat-foo` for slot 1
- `db_data_ecluse_fix-bar` for slot 2

Bind mounts (`./src:/app`) are NOT renamed — they point into the worktree.

## Common pitfalls

- **Hardcoded `localhost:3000` in app code**: the app itself runs inside Docker and should reference services by service name, not localhost. The offset applies only to host-side port bindings.
- **Compose file uses fixed external ports**: if any service uses a host port that doesn't contain a `$VAR`, ecluse offsets the numeric port directly. If your compose file uses `${PORT:-3000}:3000`, ecluse offsets the resolved base port.
- **`docker compose watch` requires compose v2.22+**: pass `--watch` to `ecluse up` to enable it.

## Teardown

```bash
ecluse down feat-foo             # stops containers, removes worktree, removes volumes
ecluse down feat-foo --keep-volumes  # stops containers, keeps volumes (for inspection)
```

## See also

- `ecluse skills show choosing-a-mode`
- `ecluse skills show troubleshooting`
