# node-container

Node.js + Postgres fully containerized. Everything runs in Docker — no native processes.

ecluse manages a separate compose project per worktree, with all ports offset by slot. The Docker image entrypoint runs `prisma migrate deploy` before starting the server, so no `on_up` hook is required.

## Mode

`container` — all services including the app run in Docker containers.

## Services

| Service  | Role              |
|----------|-------------------|
| postgres | database          |
| web      | Node.js app image |

Neither service has `ecluse.role: app` — in container mode all services are containerized.

## Environment variables set by ecluse

| Variable               | Description                              |
|------------------------|------------------------------------------|
| `ECLUSE_SLUG`          | Session slug                             |
| `ECLUSE_SLOT`          | Slot number                              |
| `ECLUSE_POSTGRES_PORT` | Offset host port for Postgres            |

In container mode ecluse does not set `DATABASE_URL` or `PORT` on the host, as the app resolves its peers via Docker networking (service name `postgres`).

## Hooks

None. The `CMD` in `Dockerfile` runs `prisma migrate deploy && node dist/index.js`.

## Usage

```sh
ecluse init
ecluse up my-feature           # starts postgres + web containers
ecluse ls                      # check status

ecluse down my-feature         # tears down containers and volumes
```
