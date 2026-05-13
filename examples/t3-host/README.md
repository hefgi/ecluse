# t3-host

T3 stack (Next.js + tRPC + Prisma) running in host mode.

Everything — app and database — runs natively on the host. ecluse creates an isolated git worktree per session and assigns a unique port. There is no Docker in this setup. Postgres must be running locally (e.g. via Homebrew or Postgres.app).

Each slot gets a distinct `PORT` so multiple worktrees can serve simultaneously. The `DATABASE_URL` is not set by ecluse in host mode — you must set it in `.env` (or `.env.local`) pointing to your local Postgres. Use `$ECLUSE_SLUG` in a convention to keep databases separate (e.g. `myapp_$ECLUSE_SLUG`).

## Mode

`host` — no Docker, everything runs on the host.

## Environment variables set by ecluse

| Variable      | Description                              |
|---------------|------------------------------------------|
| `ECLUSE_SLUG` | Session slug (use in DB name convention) |
| `ECLUSE_SLOT` | Slot number                              |
| `PORT`        | Next.js port (`base_port + slot`, e.g. 3001 for slot 1) |

## Hooks

- `on_up`: runs `npx prisma migrate deploy` to apply migrations.
- `on_down`: runs `npx prisma migrate reset --force` to wipe the slot's database on teardown (optional — remove if you want to keep data).

## .env setup

Copy `.env.example` to `.env.local` and set `DATABASE_URL` to a database name that includes the slug:

```sh
DATABASE_URL="postgresql://postgres:postgres@localhost:5432/myapp_$(ecluse env | grep ECLUSE_SLUG | cut -d= -f2)"
```

Or set it manually per worktree after `ecluse shell <slug>`.

## Usage

```sh
ecluse init
ecluse up my-feature
ecluse shell my-feature

# Inside the session shell
npm run dev        # starts Next.js on $PORT

ecluse down my-feature
```
