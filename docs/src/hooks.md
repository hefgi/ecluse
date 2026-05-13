# Hooks

Hooks run shell commands at lifecycle points. They execute inside the worktree directory with all `.env.ecluse` variables pre-loaded.

```toml
[hooks]
on_up = "npx prisma migrate deploy"
on_down = "npx prisma migrate reset --force"
```

## on_up

Runs after services start and env vars are written, before `ecluse up` returns. Use it for:

- Database migrations
- Seeding
- Any setup your app needs before it can run

## on_down

Runs before services stop. Use it for:

- Resetting database state
- Cleanup that must happen while services are still running

## Environment

All hooks run with:

- Working directory set to the worktree root
- All `ECLUSE_*` variables in the environment
- `PORT`, `ECLUSE_SLOT`, `ECLUSE_SLUG`, `ECLUSE_MODE` available

## Examples

### Prisma migrations

```toml
[hooks]
on_up = "npx prisma migrate deploy"
on_down = "npx prisma migrate reset --force"
```

### Rails

```toml
[hooks]
on_up = "bundle exec rails db:migrate"
on_down = "bundle exec rails db:drop"
```

### Multiple commands

```toml
[hooks]
on_up = "npx prisma migrate deploy && npx prisma db seed"
```

ecluse doesn't manage databases directly — your app's own tooling handles that via hooks.
