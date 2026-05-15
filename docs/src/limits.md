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

## Process management is spawn-and-kill only

For `host` and `hybrid` modes, ecluse spawns native services on `up` (via `command` + `process_manager`) and kills them on `down`. It does not monitor or restart crashed processes. `ecluse ls` warns about dead nohup-managed processes. For a fresh start use:

```bash
ecluse down feat-foo --keep-worktree
ecluse up feat-foo --reuse-worktree
```

## `command` requires the app to read its port from the environment

ecluse injects the full `.env.ecluse` contents — `PORT`, `ECLUSE_SLOT`, `ECLUSE_SLUG`, `ECLUSE_MODE`, all `ECLUSE_<NAME>_PORT` vars, and any `port_env` aliases — directly into the environment of the spawned process. There is no separate sourcing step; the same map written to `.env.ecluse` is passed to the child process before exec. This only fails if the app ignores the environment entirely:

- **The port is hardcoded in source code** — change the app to read `$PORT`.
- **The port is set in a config file** (e.g. `config/puma.rb`, `vite.config.ts`, `.env`) — ecluse does not modify app config files; update the config to read from the environment instead.

If the app reads a custom env var name, use `port_env` to inject it:

```toml
[[services]]
name = "api"
base_port = 3000
port_env = "DJANGO_PORT"   # ecluse sets DJANGO_PORT = allocated port
```

If the framework accepts a CLI flag, pass the env var through the command directly:

```toml
command = "next dev --port $PORT"
command = "bundle exec rails s -p $PORT"
```

## Mode is set at init time

Mode is stored in `.ecluse.toml` and applies to all sessions. Changing mode requires editing `.ecluse.toml` and tearing down all existing sessions first.

## Multiple compose files require explicit `compose` fields

ecluse supports multiple compose files via the `compose` field on each `[[services]]` block. Services without a `compose` field fall back to the root compose file. There is no automatic discovery of compose files in subdirectories.

## Localhost only

All ports are bound to `localhost`. No remote or network-accessible port management.

## No Ctrl+C rollback

If you interrupt `ecluse up` mid-flight, partial state may be left behind. Use `ecluse down <slug>` to clean up.

## Platform support

macOS and Linux. Windows is not supported.
