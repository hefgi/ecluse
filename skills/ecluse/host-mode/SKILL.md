---
name: ecluse-host-mode
description: >
  Use this skill when the user is working with ecluse in host mode,
  has no Docker setup or prefers native dev tools, asks how host mode
  works, or needs help with port reservation and Postgres database
  provisioning per worktree.
tags:
  - ecluse
  - host
  - native
  - postgres
---

# Host Mode

## What it does

No containers. ecluse reserves a port range from the slot, optionally provisions a dedicated database on a host Postgres, writes `.env.ecluse`, and creates the worktree. You start your own dev server — ecluse doesn't run any process for you.

## Prerequisites

- No Docker required
- Optionally: a host Postgres instance (`brew services start postgresql@16` on macOS)
- `ecluse init --mode host` (or auto-detected)

## Workflow

```bash
ecluse up feat-foo
# Output:
#   Session:   feat-foo (slot 1)
#   Worktree:  .ecluse/worktrees/feat-foo
#   Mode:      host
#   App port:  3100
#   Database:  myapp_feat_foo
#   Next step: cd .ecluse/worktrees/feat-foo && source .env.ecluse

cd .ecluse/worktrees/feat-foo
source .env.ecluse
npm run dev         # your app listens on $PORT, connects to $DATABASE_URL
```

## Database provisioning config

Add to `.ecluse.toml`:

```toml
[database]
provider = "postgres-host"
host = "localhost"
port = 5432
user = "postgres"
base = "myapp"
# password: use PGPASSWORD env var or ~/.pgpass — never written to config
```

With `base = "myapp"` and slug `feat-foo` the database is `myapp_feat_foo`.

## Multiple sessions

```bash
ecluse up feat-foo   # port 3100, db myapp_feat_foo
ecluse up fix-bar    # port 3200, db myapp_fix_bar
```

Both share the same Postgres instance but have separate databases.

## Teardown

```bash
ecluse down feat-foo                    # drops myapp_feat_foo, removes worktree
ecluse down feat-foo --keep-database    # removes worktree, keeps database
```

## Common failures

- **"Port 3100 is in use by PID 12345"** — stop the process: `kill 12345`.
- **"Host Postgres is unreachable"** — `brew services start postgresql@16` or fix `[database]` config.
- **App can't find database** — ensure you ran `source .env.ecluse` before starting the server.

## See also

- [choosing-a-mode](../choosing-a-mode/SKILL.md)
- [agent-workflow](../agent-workflow/SKILL.md)
