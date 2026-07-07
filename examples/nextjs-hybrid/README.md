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

- `pre_spawn`: writes `.env.local` with the slot's `DATABASE_URL`, waits for postgres to accept queries, then applies migrations. All of this must complete **before** Next.js boots — Prisma reads `DATABASE_URL` once at startup, and the app queries tables that must already exist. Using `post_up` here would mean the app boots against stale env / a missing schema and crashes.

## Why `.env.local` is copied, not symlinked

`inherit_env` defaults to symlinking `.env` and `.env.local` from the repo root into each worktree. That works for shared secrets (`.env`), but breaks for `.env.local` here: `pre_spawn` rewrites `.env.local` with the current slot's `DATABASE_URL`, and if the file were a symlink, every `ecluse up` in a different worktree would overwrite the single shared file — last writer wins, all other worktrees end up pointing at the wrong slot's postgres. Setting `mode = "copy"` gives each worktree its own real `.env.local`.

## Usage

```sh
ecluse init
ecluse up my-feature
ecluse shell my-feature

# Inside the session shell
npm run dev        # starts Next.js on $PORT

ecluse down my-feature
```
