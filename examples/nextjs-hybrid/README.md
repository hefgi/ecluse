# nextjs-hybrid

Next.js with Prisma and Postgres, running in hybrid mode.

Postgres runs in a Docker container managed by ecluse. Next.js runs natively. Each worktree gets its own isolated Postgres database on an offset port, so branches with diverging schemas never conflict.

## Mode

`hybrid` — Postgres containerized, Next.js runs natively.

## Services

| Service  | Role     | Label              |
|----------|----------|--------------------|
| postgres | data     | —                  |
| web      | app      | `ecluse.role: app` |

## Environment variables set by ecluse

| Variable               | Description                              |
|------------------------|------------------------------------------|
| `ECLUSE_SLUG`          | Session slug                             |
| `PORT`                 | Next.js port (`base_port + slot`, e.g. 3001 for slot 1) |
| `ECLUSE_POSTGRES_PORT` | Per-slot host port for Postgres          |

## Hooks

- `post_up`: runs `npx prisma migrate deploy` against the slot's database.

## Usage

```sh
ecluse init
ecluse up my-feature
ecluse shell my-feature

# Inside the session shell
npm run dev        # starts Next.js on $PORT

ecluse down my-feature
```
