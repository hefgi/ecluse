---
name: ecluse-troubleshooting
description: >
  Use this skill when the user hits an error with ecluse, asks why something
  isn't working, or reports unexpected behavior. Covers port conflicts, Docker
  not running, slot exhaustion, stale state, host Postgres unreachable, lock
  timeout, and mode mismatch errors.
tags:
  - ecluse
  - troubleshooting
  - errors
---

# Troubleshooting

## Port already in use

**Error:** `port 3100 is already in use by PID 12345`

```bash
kill 12345
# or find it first:
lsof -iTCP:3100 -sTCP:LISTEN
```

Then retry `ecluse up`. If the port is persistently occupied by a background service, increase `stride` in `.ecluse.toml`.

## Docker daemon not running

**Error:** `docker command failed` or `Docker not installed`

```bash
open -a OrbStack      # macOS — OrbStack (recommended)
open -a Docker        # macOS — Docker Desktop
sudo systemctl start docker   # Linux
```

Verify: `docker info` should exit 0.

## Slot exhaustion

**Error:** `all 8 slots are in use`

```bash
ecluse ls                    # find active sessions
ecluse down <oldest-slug>    # free a slot
```

Or increase `max_slots` in `.ecluse.toml` (safe to edit directly).

## Stale state after manual worktree deletion

If a worktree was deleted outside of `ecluse down`, state.json still references it. Run `ecluse down <slug>` anyway — the mode handlers skip missing worktrees during teardown. The session is removed from state.

If `down` fails, edit `.ecluse/state.json` directly and remove the stale session entry.

## Host Postgres unreachable

**Error:** `Host Postgres is unreachable; check your [database] config`

```bash
brew services start postgresql@16    # macOS
sudo systemctl start postgresql      # Linux
psql -U postgres -c "SELECT 1"       # verify
```

Check that `.ecluse.toml` `[database]` section has the correct `host`, `port`, and `user`.

## Lock timeout

**Error:** `timed out waiting for state lock after 10s`

Another `ecluse` process holds the lock, or a previous run crashed mid-flight.

```bash
ps aux | grep ecluse     # check for running processes
rm .ecluse/state.lock    # remove stale lock if no processes found
```

## Mode mismatch after re-init

After changing `.ecluse.toml` mode, existing sessions still record their original mode in `state.json`. `ecluse down` uses each session's stored mode — this is correct. `ecluse ls` shows the mode per session. Run `ecluse down` on old sessions when convenient.

## Not inside a git repository

**Error:** `not inside a git repository`

`ecluse init` must be run from within a git repo:

```bash
git init && git add . && git commit -m "init"
ecluse init
```

## See also

- [limits](../limits/SKILL.md) — things ecluse intentionally does not do
- [references/error-codes.md](references/error-codes.md) — full error reference
