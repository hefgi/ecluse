# Commands

```
ecluse init     [--mode container|host|hybrid] [--explain] [--yes] [--quiet]
ecluse up       <slug> [--branch <name>] [--watch] [--json] [--reuse-worktree] [--port <name>=<value>] [--services <name>,...] [--quiet]
ecluse sync     <slug> [--json] [--quiet]
ecluse down     <slug> [--keep-volumes] [--keep-branch] [--keep-worktree] [--quiet]
ecluse shutdown [--keep-volumes] [--keep-worktrees] [--quiet]
ecluse flush    [--yes] [--quiet]
ecluse ls       [--json]
ecluse shell    <slug>
ecluse env      [<slug>]
ecluse validate [--ports] [--quiet]
```

## ecluse init

Detects the right mode for your repo and writes `.ecluse.toml`. Runs interactively — shows the detected mode and asks for confirmation before writing.

| Flag | Description |
|---|---|
| `--mode container\|host\|hybrid` | Override detected mode |
| `--explain` | Show detection signals |
| `--yes` | Skip confirmation prompt |
| `--quiet` | Suppress step output |

## ecluse up

Creates a git worktree, allocates a slot, starts services, and writes `.env.ecluse`. Returns the worktree path and all env vars.

| Flag | Description |
|---|---|
| `--branch <name>` | Use a specific branch name instead of the slug |
| `--watch` | Stream service logs after startup |
| `--json` | Output worktree path + env vars as JSON |
| `--reuse-worktree` | Reuse an existing worktree instead of creating one |
| `--port <name>=<value>` | Pin a service to a specific port for this session |
| `--services <name>,...` | Bring up only this subset of services; unknown names are rejected before any worktree is created |
| `--quiet` | Suppress step output (implied by `--json`) |

## ecluse sync

Registers a manually-started environment with ecluse. Use this when services were started by hand (not via `ecluse up`) or when `state.json` was lost.

```bash
ecluse sync <slug>          # discover + register
ecluse sync <slug> --json   # machine-readable output
```

`ecluse sync` finds all processes whose cwd is inside the worktree, matches them to services in `.ecluse.toml` by walking the process tree from each service's `command`, and records the actual listening ports. Docker services (hybrid mode) are detected via `docker ps` by container name. PID files are written so `ecluse down` can kill discovered processes normally.

If a session for the slug already exists in `state.json`, sync refreshes its port_overrides and PID tracking without changing the slot or branch.

| Flag | Description |
|---|---|
| `--json` | Output session info as JSON |
| `--quiet` | Suppress step output (implied by `--json`) |

**Requirements:** native services must have a `command` field in `.ecluse.toml`. Services with no matching running process are reported as warnings — partial sync is still registered.

## ecluse down

Tears down services, frees the slot, and removes the worktree.

| Flag | Description |
|---|---|
| `--keep-volumes` | Preserve named Docker volumes |
| `--keep-branch` | Keep the git branch (no-op — branches are never deleted by ecluse) |
| `--keep-worktree` | Keep the worktree directory on disk |
| `--quiet` | Suppress step output |

## ecluse shutdown

Tears down all active sessions at once. Equivalent to running `ecluse down` on every session.

| Flag | Description |
|---|---|
| `--keep-volumes` | Preserve named Docker volumes |
| `--keep-worktrees` | Keep worktree directories on disk |
| `--quiet` | Suppress step output |

## ecluse flush

Hard reset — returns the repo to a clean state as if `ecluse init` was just run. Use this when sessions are stuck, state is corrupted, or you want to wipe everything and start fresh.

```bash
ecluse flush        # prompts for confirmation
ecluse flush --yes  # skip the prompt (CI / agent use)
```

What flush does:

1. Gracefully tears down all sessions known to `state.json` (same as `ecluse shutdown`).
2. Kills orphaned tmux sessions named `ecluse-*`.
3. Stops orphaned Docker Compose projects matching `<prefix>_*` (detected via `docker ps`).
4. Runs `git worktree remove --force` on every directory under `worktree_dir`, then `git worktree prune`.
5. Removes `.ecluse/pids/`, `.ecluse/logs/`, and `.ecluse/overlays/`.
6. Resets `state.json` to `{"version": 1, "sessions": []}`.

**Docker volumes are not removed** — flush clears infra but preserves data volumes. Run `docker volume prune` separately if you also want those gone.

Steps 1–5 are best-effort: failures are logged and ignored. Step 6 (resetting state) is required — flush fails only if it cannot write `state.json`.

| Flag | Description |
|---|---|
| `--yes` | Skip confirmation prompt |
| `--quiet` | Suppress step output |

## ecluse shell

Drops into the worktree with all `.env.ecluse` variables loaded in the shell environment. Interactive use only.

If the session has a tmux session (i.e. `process_manager = "tmux"` was set at `ecluse up` time), this attaches to that session instead of spawning a new shell — you'll see the running service windows directly.

## ecluse env

Prints the session's environment variables as JSON. Includes `worktree_path` and all `ECLUSE_*` vars.

## ecluse ls

Lists active sessions. Use `--json` for machine-readable output.

## ecluse validate

Validates port ranges in `.ecluse.toml` and checks for gaps or collisions. Use `--ports` to preview the full port allocation table across all slots. Also checks that the configured `process_manager` binary is installed (e.g. tmux or nohup).

| Flag | Description |
|---|---|
| `--ports` | Print the full port allocation table for all slots |
| `--quiet` | Suppress step output |
