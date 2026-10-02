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

- `pre_spawn`: writes `.env.development.local` with the slot's `DATABASE_URL`, waits for postgres to accept queries, then applies migrations. All of this must complete **before** Next.js boots — Next.js reads its env files once at startup, and the app queries tables that must already exist. Using `post_up` here would mean the app boots against stale env / a missing schema and crashes.

## Why `.env.development.local`

`.env` and `.env.local` stay symlinked from the repo root (the `inherit_env` default), so shared secrets are the same in every worktree and never overwritten. Slot-specific values go in `.env.development.local`, which Next.js loads ahead of `.env.local` in dev. `pre_spawn` generates it inside each worktree, so sibling worktrees can't clobber each other.

The Prisma CLI only reads `.env`, so the hook exports `DATABASE_URL` before running `prisma migrate deploy` — otherwise the migration would run against whatever database `.env` points at.

## Usage

```sh
ecluse init
ecluse up my-feature
ecluse shell my-feature

# Inside the session shell
npm run dev        # starts Next.js on $PORT

ecluse down my-feature
```
