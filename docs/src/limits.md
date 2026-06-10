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

## `.env` / `.env.local` are symlinked but not auto-loaded by every framework

ecluse materializes `.env` and `.env.local` from the repo root into each worktree at `ecluse up` time (configurable via `inherit_env`). By default they are symlinks, but each entry may opt into `mode = "copy"` for per-worktree independence (see next section). Either way, the files are present at the expected path inside the worktree, but **whether they are actually read depends on your framework**:

- **Auto-load** (no action needed): Next.js, Vite, Create React App, docker-compose — these discover and load `.env` / `.env.local` automatically.
- **Explicit-load required**: Node.js without dotenv, Rails, Django, Go, Rust binaries — the app must call `dotenv.config()` / `Dotenv::dotenv()` etc. at startup.

`ECLUSE_*` variables and `PORT` are always injected directly into the spawned process environment — they do not rely on dotenv at all. Only secrets and base config that your app reads from `.env` at runtime require the framework to auto-load the file.

If your framework does not auto-load, add a `post_up` hook to source the file or call your loader:

```toml
[hooks]
post_up = "set -a && source .env && set +a && your-start-command"
```

Or configure your app's dotenv library to load the file explicitly.

## Per-worktree env overrides require `mode = "copy"`

By default, `inherit_env` symlinks `.env` and `.env.local` from the repo root into each worktree. This is correct for **shared secrets** that should stay in sync everywhere (DB passwords, API keys), but it breaks isolation when the user wants to flip a value in one worktree only.

**Concrete example.** The user has `AUTH_ENABLED=true` in the root `.env.local`. Inside worktree `feat-foo` they want to test with `AUTH_ENABLED=false`. They edit `.ecluse/worktrees/feat-foo/.env.local` and flip the value — but since the file is a symlink, they actually edited the shared root file. Worktrees `feat-bar`, `feat-baz`, and the root project now all see `AUTH_ENABLED=false` too.

**Fix.** Set `mode = "copy"` on the file in `.ecluse.toml`:

```toml
inherit_env = [
  ".env",                                  # shared secrets — symlinked
  { file = ".env.local", mode = "copy" },  # per-worktree overrides — copied once
]
```

With copy mode:

- On the first `ecluse up` for a worktree, ecluse copies the root file into the worktree as a real file (not a symlink).
- Edits in the worktree's copy stay local — never propagate to the root.
- Edits in the root's file do not propagate to existing worktree copies on subsequent `ecluse up` runs. The copy is initialized once and then frozen.
- Each new worktree gets a fresh copy from the current state of the root file at the time of its creation.
- If a worktree already has a real (non-symlink) file at that path, ecluse leaves it untouched — never clobbers user files.
- Stale symlinks left over from a prior `symlink` configuration are replaced with a fresh copy on the next `ecluse up`.

To skip inheritance entirely for a single run: `ecluse up <slug> --no-inherit-env`.

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
