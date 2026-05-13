---
name: ecluse-agent-workflow
description: >
  Use this skill when you are a coding agent (Claude Code, Cursor, Codex, etc.)
  and need to work on a task in an isolated environment. Covers the canonical
  ecluse loop: up → cd → source → work → verify → down. Use whenever the user
  asks you to implement a feature, fix a bug, or run experiments in parallel
  without disturbing other sessions.
tags:
  - ecluse
  - agents
  - workflow
  - isolation
---

# Agent Workflow

## Prerequisites

- `ecluse init` has been run in this repo (`.ecluse.toml` exists)
- You have read/write access to the repo directory

## Canonical loop

```bash
# 1. Create an isolated session for this task
#    slug: short, task-descriptive, lowercase — e.g. feat-auth, fix-login, refactor-api
ecluse up <slug>

# 2. Read the output — it gives you the worktree path and connection details
#    Session:   feat-auth (slot 2)
#    Worktree:  /path/to/repo/.ecluse/worktrees/feat-auth
#    Mode:      hybrid
#    App port:  3200
#    Database:  myapp_feat_auth
#    Next step: cd worktree && source .env.ecluse

# 3. Change into the worktree
cd <worktree_path from output>

# 4. Load environment (port assignments, connection strings)
source .env.ecluse

# 5. Start the dev server — only needed for host and hybrid modes
#    container mode starts all services automatically via docker compose
npm run dev      # or: bin/dev  /  python manage.py runserver  /  bin/rails server

# 6. Do the work — edit files, run tests, make commits in this worktree
#    This is a full git working directory on branch ecluse/<slug>

# 7. Verify
npm test
curl http://localhost:$PORT/health

# 8. Tear down
ecluse down <slug>
```

## Mode-specific notes

**`container` mode** — `ecluse up` starts all services in Docker. No manual dev server needed. Access services at `localhost:<offset_port>`.

**`host` mode** — `ecluse up` reserves ports and provisions a database if configured. You must start the dev server yourself after sourcing `.env.ecluse`.

**`hybrid` mode** — `ecluse up` starts data containers only (postgres, redis, etc.). You must start the app process yourself. `.env.ecluse` wires the app to the containerized data services.

## Environment variables in `.env.ecluse`

See [references/env-vars.md](references/env-vars.md) for the full variable reference.

Key variables:

| Variable | Description |
|---|---|
| `PORT` | App port — use this instead of hardcoding 3000 |
| `DATABASE_URL` | Postgres connection string |
| `REDIS_URL` | Redis connection string (if redis service present) |
| `ECLUSE_SLOT` | Slot number |
| `ECLUSE_MODE` | `container`, `host`, or `hybrid` |

## Running multiple sessions in parallel

Each session gets a unique slot and port range. You can run several in parallel:

```bash
ecluse up feat-auth   # slot 1, port 3100
ecluse up feat-cache  # slot 2, port 3200
ecluse ls             # see both
```

Each worktree is an independent git branch. Changes in one do not affect the other.

## Common failures

- **"session already exists"** — slug already in use. Pick a different slug or `ecluse down <slug>` first.
- **"all slots in use"** — `ecluse ls` to find stale sessions, `ecluse down` the oldest.
- **Dev server can't connect to database** — you forgot to `source .env.ecluse` before starting.
- **Port already in use** — another process holds the port. Check with `lsof -iTCP:<port>`.

## See also

- [troubleshooting](../troubleshooting/SKILL.md)
- [limits](../limits/SKILL.md)
