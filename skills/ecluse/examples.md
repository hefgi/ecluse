# ecluse config examples

Five examples covering every mode and port pattern. Read the `.ecluse.toml` and `docker-compose.yml` files directly — they are the authoritative reference.

## Index

| Directory | Mode | Language | Pattern |
|---|---|---|---|
| [t3-host](examples/t3-host/.ecluse.toml) | host | TypeScript | Single service, no Docker |
| [node-container](examples/node-container/.ecluse.toml) | container | TypeScript | Fully containerized, ports from compose |
| [fastapi-hybrid](examples/fastapi-hybrid/.ecluse.toml) | hybrid | Python | Single service + data in Docker |
| [t3-monorepo](examples/t3-monorepo/.ecluse.toml) | hybrid | TypeScript | Multi-port monorepo (4 services) |
| [k3d](examples/k3d/.ecluse.toml) | host | — | Kubernetes ingress, all services inside cluster |

## Patterns

**Host, single service** — no Docker, one port, `PORT` alias set automatically:
```toml
[ports]
app = 0   # → ECLUSE_APP_PORT + PORT alias
```

**Container** — all services in Docker, ports come from compose overlay — no `[ports]` needed.

**Hybrid, single service** — data in Docker, app on host:
```toml
[ports]
api = 0   # → ECLUSE_API_PORT + PORT alias
# data service ports available as ECLUSE_POSTGRES_PORT, ECLUSE_REDIS_PORT, etc.
```

**Hybrid, monorepo** — multiple native processes, each needs its own port:
```toml
[ports]
api     = 0   # → ECLUSE_API_PORT + PORT alias
web     = 1   # → ECLUSE_WEB_PORT
worker  = 2   # → ECLUSE_WORKER_PORT
email   = 3   # → ECLUSE_EMAIL_PORT
```

**Kubernetes / k3d** — two ingress host ports only; services talk to each other inside the cluster via DNS, not host ports:
```toml
[ports]
http  = 0   # → ECLUSE_HTTP_PORT + PORT alias → mapped to :80 inside cluster
https = 1   # → ECLUSE_HTTPS_PORT → mapped to :443 inside cluster
```

## How ports are resolved

`base_port + slot × stride + index`

With defaults (`base_port = 3000`, `stride = 100`):
- slot 1: index 0 → 3100, index 1 → 3101, index 2 → 3102
- slot 2: index 0 → 3200, index 1 → 3201, index 2 → 3202

The first `[ports]` entry always sets the `PORT` alias for framework compatibility.
Omit `[ports]` entirely for single-service stacks — `PORT` is set to `base_port + slot × stride` automatically.
