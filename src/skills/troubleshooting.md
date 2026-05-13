# Troubleshooting

## Port already in use

**Error**: `port 3100 is already in use by PID 12345`

**Fix**: Stop the occupying process:
```bash
kill 12345
# or: lsof -iTCP:3100 -sTCP:LISTEN  (to identify it)
```

If port 3100 is always occupied, increase `stride` in `.ecluse.toml` or reduce the number of always-on background services.

## Docker daemon not running

**Error**: `docker command failed` or `Docker not installed`

**Fix**:
```bash
# macOS — start Docker Desktop or OrbStack
open -a Docker
# or: open -a OrbStack

# Linux
sudo systemctl start docker
```

Verify: `docker info` should exit 0.

## Slot exhaustion

**Error**: `all 8 slots are in use`

**Fix**: List sessions and free one:
```bash
ecluse ls
ecluse down <oldest-slug>
```

Or increase `max_slots` in `.ecluse.toml` (requires no active sessions using the new slots — safe to edit and re-run `init` to recalibrate, or just edit the file directly).

## Stale state (worktree removed manually)

If a worktree was deleted outside of `ecluse down`, the state.json may still reference it.

**Fix**: Run `ecluse down <slug>` anyway — the mode handler ignores missing worktrees during teardown. It will remove the session from state.

If that fails, edit `.ecluse/state.json` directly to remove the stale session entry.

## Host Postgres unreachable

**Error**: `Host Postgres is unreachable; check your [database] config`

**Fix**:
```bash
# macOS (Homebrew)
brew services start postgresql@16

# Linux
sudo systemctl start postgresql

# Verify
psql -U postgres -c "SELECT 1"
```

Check that `.ecluse.toml` `[database]` section has the correct host, port, and user.

## Mode mismatch errors

After changing `.ecluse.toml` mode by hand, existing sessions still record their original mode in `state.json`. `ecluse down <slug>` uses the mode from state, not the config file. This is correct behavior.

If you're confused about which mode a session is using:
```bash
ecluse ls  # shows mode per session
```

## Lock timeout

**Error**: `timed out waiting for state lock after 10s`

**Cause**: Another `ecluse` process is running `up` or `down` simultaneously, or a previous run crashed while holding the lock.

**Fix**: Check for other ecluse processes:
```bash
ps aux | grep ecluse
```

If none are running, the lock file may be stale:
```bash
rm .ecluse/state.lock
```

## `ecluse init` fails: not inside a git repository

**Error**: `not inside a git repository`

**Fix**: `ecluse init` must be run from within a git repo. Initialize git first:
```bash
git init
git add .
git commit -m "initial"
ecluse init
```

## See also

- `ecluse skills show limits` — things ecluse intentionally does not do
