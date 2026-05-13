# ecluse config examples

Five examples covering every mode and port pattern. Read the `.ecluse.toml` and `docker-compose.yml` files directly — they are the authoritative reference.

## Index

| Directory | Mode | Language | Pattern |
|---|---|---|---|
| [t3-host](examples/t3-host/.ecluse.toml) | host | TypeScript | Single service, no Docker |
| [node-container](examples/node-container/.ecluse.toml) | container | TypeScript | Fully containerized, ports from compose |
| [fastapi-hybrid](examples/fastapi-hybrid/.ecluse.toml) | hybrid | Python | Two native services + data in Docker |
| [t3-monorepo](examples/t3-monorepo/.ecluse.toml) | hybrid | TypeScript | Multi-port monorepo (4 native services) |
| [k3d](examples/k3d/.ecluse.toml) | host | — | Kubernetes ingress, all services inside cluster |

## Patterns

**Host, single service** — no Docker, one port, `PORT` alias set automatically:
```toml
[[services]]
name = "app"
base_port = 3000   # slot 1 → PORT=3001, slot 2 → PORT=3002
```

**Container** — all services in Docker, ports come from compose overlay — no `[[services]]` needed.

**Hybrid, single native service + data in Docker:**
```toml
[[services]]
name = "api"
base_port = 8000   # slot 1 → ECLUSE_API_PORT=8001 + PORT alias

[[services]]
name = "postgres"
run = "docker"
base_port = 5432   # slot 1 → ECLUSE_POSTGRES_PORT=5433
```

**Hybrid, monorepo** — multiple native processes, each needs its own base_port:
```toml
[[services]]
name = "api"
base_port = 3000   # slot 1 → ECLUSE_API_PORT=3001 + PORT alias

[[services]]
name = "web"
base_port = 3100   # slot 1 → ECLUSE_WEB_PORT=3101

[[services]]
name = "worker"
base_port = 3200   # slot 1 → ECLUSE_WORKER_PORT=3201
```

**Kubernetes / k3d** — two ingress host ports; services talk inside the cluster via DNS:
```toml
[[services]]
name = "http"
base_port = 8080   # slot 1 → ECLUSE_HTTP_PORT=8081 + PORT alias

[[services]]
name = "https"
base_port = 8443   # slot 1 → ECLUSE_HTTPS_PORT=8444
```

## How ports are resolved

`port = base_port + slot`

- slot 1: base_port + 1
- slot 2: base_port + 2

Pick `base_port` values to match your app's conventional ports. Space them apart (e.g. 3000, 3100, 3200) so slots never collide even with many parallel sessions.

The first native `[[services]]` entry always sets the `PORT` alias for framework compatibility.
Omit `[[services]]` entirely for single-service stacks — `PORT` is set to `3000 + slot` automatically.
