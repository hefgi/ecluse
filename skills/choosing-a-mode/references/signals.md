# Detection Signal Table

Full reference for the 20 signals used by `ecluse init` auto-detection.
Weights are applied to scores for each mode; highest score wins.

| Signal | Detection method | container | host | hybrid |
|---|---|---:|---:|---:|
| `docker-compose.yml` or `compose.yaml` at repo root | File stat | +2 | 0 | +2 |
| Compose has a service with `build: .` | Compose parse | +3 | 0 | 0 |
| All compose services match known data images | Compose parse + image-name match | −2 | 0 | +5 |
| Any compose service has label `ecluse.role: app` | Compose parse | 0 | 0 | +10 |
| Compose has `watch:` blocks | Compose parse | +2 | 0 | +1 |
| Compose has bind mounts of source into containers | Compose parse | +2 | 0 | 0 |
| `.devcontainer/devcontainer.json` exists | File stat | +4 | 0 | 0 |
| No compose file anywhere in repo | File absence | −5 | +4 | −5 |
| `bin/dev` exists and is executable | File stat | 0 | +3 | +2 |
| `Procfile.dev` exists | File stat | 0 | +3 | +2 |
| `package.json` with non-docker `dev` script | JSON parse + string match | 0 | +2 | +2 |
| `Gemfile` + `bin/rails` present | File stat | 0 | +2 | +2 |
| Host Postgres reachable on `localhost:5432` | TCP probe (500ms timeout) | 0 | +1 | 0 |
| Version manager file present (mise, asdf, nvm, etc.) | File stat | 0 | +1 | +1 |
| README mentions `docker compose up` then `bin/dev` within 10 lines | Regex | 0 | 0 | +3 |
| Docker not installed or daemon not running | `docker info` exit code | −10 | 0 | −10 |
| `flake.nix` exists | File stat | **Unsupported** | — | — |
| `WORKSPACE` / `BUILD.bazel` / `MODULE.bazel` exists | File stat | **Unsupported** | — | — |

**Known data images** (triggers the "all data" signal): `postgres`, `redis`, `mysql`, `mongo`,
`rabbitmq`, `mailhog`, `minio`, `elasticsearch`, `nats`, `kafka`, `clickhouse`, `memcached`.

**`Unsupported`** signals short-circuit scoring and print a reason. The user can override
with `ecluse init --mode <m>` if they disagree.
