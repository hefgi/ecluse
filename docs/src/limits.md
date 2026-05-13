# Known limits

These are intentional v0 limitations, not bugs.

## Ports are checked, not reserved

ecluse finds a free port at `ecluse up` time and writes it to `.env.ecluse`. There is a small window between the check and when your process actually binds — if something else takes the port in between, the port in `.env.ecluse` will be wrong.

Fix: tear down and recreate the session, or pin a port manually.

```bash
ecluse down feat-foo --keep-worktree
ecluse up feat-foo --reuse-worktree
# or
ecluse up feat-foo --port api=4001
```

## No native process management

For `host` and `hybrid` modes, ecluse writes the environment and optionally runs `on_up` hooks — it does not start or stop your app. If a native service fails to start, ecluse has no way to retry it without a full `down`/`up` cycle.

## Mode is set at init time

Mode is stored in `.ecluse.toml` and applies to all sessions. Changing mode requires editing `.ecluse.toml` and tearing down all existing sessions first.

## One compose file per repo

ecluse expects a single `docker-compose.yml` at repo root. Multi-compose setups are not supported in v0.

## Localhost only

All ports are bound to `localhost`. No remote or network-accessible port management.

## No Ctrl+C rollback

If you interrupt `ecluse up` mid-flight, partial state may be left behind. Use `ecluse down <slug>` to clean up.

## Platform support

macOS and Linux. Windows is not supported.
