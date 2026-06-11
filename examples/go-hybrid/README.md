# go-hybrid

Go API with Postgres, running in hybrid mode.

Postgres runs in a Docker container managed by ecluse. The Go binary runs natively. Each worktree gets its own isolated Postgres database on an offset port. The `migrate` CLI tool handles schema migrations.

## Mode

`hybrid` — Postgres containerized, Go API runs natively.

## Services

| Service  | Role     | Label              |
|----------|----------|--------------------|
| postgres | data     | —                  |
| api      | app (Go) | `ecluse.role: app` |

## Environment variables set by ecluse

| Variable               | Description                              |
|------------------------|------------------------------------------|
| `ECLUSE_SLUG`          | Session slug                             |
| `PORT`                 | API port (`base_port + slot`, e.g. 8081 for slot 1) |
| `ECLUSE_POSTGRES_PORT` | Per-slot host port for Postgres          |

## Hooks

- `post_up`: runs `migrate -path ./migrations -database "$DATABASE_URL" up` to apply pending migrations.

Requires the [`migrate` CLI](https://github.com/golang-migrate/migrate) to be installed (`brew install golang-migrate`).

## Usage

```sh
ecluse init
ecluse up my-feature
ecluse shell my-feature

# Inside the session shell
go run ./cmd/api

ecluse down my-feature
```
