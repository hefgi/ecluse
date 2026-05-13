# Agent Workflow

## When to use

You are a coding agent (Claude Code, Cursor, Codex, etc.) and need to work on a task in an isolated environment without disrupting other sessions.

## Prerequisites

- `ecluse init` has been run in this repo
- You have read/write access to the repo directory

## Canonical loop

```bash
# 1. Create an isolated session for this task
ecluse up <slug>
# slug should be short and task-descriptive: feat-auth, fix-login-bug, refactor-api

# 2. The output tells you the worktree path
# Example output:
#   Session:   feat-auth (slot 2)
#   Worktree:  /path/to/repo/.ecluse/worktrees/feat-auth
#   Mode:      hybrid
#   App port:  3200
#   Database:  myapp_feat_auth
#   Next:      cd worktree && source .env.ecluse

# 3. Change into the worktree
cd <worktree_path>

# 4. Load connection strings and port assignments
source .env.ecluse

# 5. Start the dev server (hybrid and host modes only — container mode starts automatically)
#    The .env.ecluse file sets PORT, DATABASE_URL, REDIS_URL, etc.
npm run dev      # or bin/dev, python manage.py runserver, bin/rails server, etc.

# 6. Do your work — edit files, run tests, make commits
# The worktree is a full git working directory on its own branch

# 7. Verify your changes
npm test
# or: curl http://localhost:$PORT/health

# 8. Tear down when done
ecluse down <slug>
```

## Mode-specific notes

**`container` mode**: `ecluse up` starts all containers. No manual dev server needed. Access services via `localhost:<offset_port>`.

**`host` mode**: `ecluse up` reserves ports and provisions a database (if configured). You must start your own dev server using the env vars from `.env.ecluse`.

**`hybrid` mode**: `ecluse up` starts data containers only. You must start your own app process. The `.env.ecluse` file wires the app to the containerized data services.

## Environment variables written to `.env.ecluse`

| Variable | Description |
|---|---|
| `ECLUSE_SLOT` | Slot number (integer) |
| `ECLUSE_OFFSET` | Port offset (slot × stride) |
| `ECLUSE_MODE` | Mode: container, host, or hybrid |
| `PORT` | App port (host and hybrid only) |
| `DATABASE_URL` | Postgres connection string (if database provisioned) |
| `REDIS_URL` | Redis connection string (if redis service found) |
| `ECLUSE_<SERVICE>_PORT` | Port for each data service |

## Common failures

- **"session already exists"**: that slug is already in use. Choose a different slug or run `ecluse down <slug>` first.
- **"all slots in use"**: run `ecluse ls` to find stale sessions and `ecluse down` the oldest ones.
- **dev server can't connect to database**: ensure you sourced `.env.ecluse` before starting the server.

## See also

- `ecluse skills show troubleshooting`
- `ecluse skills show limits`
