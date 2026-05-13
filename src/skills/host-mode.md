# Host Mode

## When to use

Your dev stack runs entirely on the host — no Docker required. You use `npm run dev`, `bin/rails server`, `python manage.py runserver`, or similar native commands.

## Prerequisites

- No Docker dependency (Docker can be absent)
- Optionally: a host Postgres for database isolation
- `ecluse init --mode host` has been run

## What ecluse does in host mode

1. Allocates a slot and computes `offset = slot × stride`.
2. Reserves port `offset` (checks via `lsof` that it's free; errors with PID if occupied).
3. If `database.provider = "postgres-host"` in `.ecluse.toml`:
   - Checks that host Postgres is reachable
   - Runs `CREATE DATABASE "<base>_<slug>"` via `psql`
4. Creates a git worktree at `.ecluse/worktrees/<slug>`.
5. Writes `.env.ecluse` with `PORT`, `DATABASE_URL`, `ECLUSE_SLOT`, etc.
6. Prints next-step hint.

**Host mode does not start any process.** You run your dev command.

## Typical workflow

```bash
ecluse up feat-foo
# Output:
#   Worktree:  .ecluse/worktrees/feat-foo
#   Port:      3100
#   Database:  myapp_feat_foo
#   Next:      cd .ecluse/worktrees/feat-foo && source .env.ecluse && npm run dev

cd .ecluse/worktrees/feat-foo
source .env.ecluse
npm run dev
# Your app now listens on $PORT (3100) and connects to $DATABASE_URL
```

## Database config in `.ecluse.toml`

```toml
[database]
provider = "postgres-host"
host = "localhost"
port = 5432
user = "postgres"
base = "myapp"
# password: use PGPASSWORD env var or ~/.pgpass
```

With `base = "myapp"` and slug `feat-foo`, the database is `myapp_feat_foo`.

## Multiple sessions

```bash
ecluse up feat-foo   # port 3100, db myapp_feat_foo
ecluse up fix-bar    # port 3200, db myapp_fix_bar
```

Both share the same host Postgres instance but have separate databases.

## Teardown

```bash
ecluse down feat-foo                  # drops myapp_feat_foo, removes worktree
ecluse down feat-foo --keep-database  # removes worktree, keeps database
```

## Common failures

- **"Port 3100 is in use by PID 12345"**: stop the process holding the port before running `up`.
- **"Host Postgres is unreachable"**: start Postgres (`brew services start postgresql`) or fix the `[database]` config.
- **App can't find database**: ensure you `source .env.ecluse` before starting your dev server.

## See also

- `ecluse skills show choosing-a-mode`
- `ecluse skills show agent-workflow`
