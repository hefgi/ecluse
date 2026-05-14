# Commands

```
ecluse init [--mode container|host|hybrid] [--explain] [--yes]
ecluse up <slug> [--branch <name>] [--watch] [--json] [--reuse-worktree] [--port <name>=<value>]
ecluse shell <slug>
ecluse env [<slug>]
ecluse down <slug> [--keep-volumes] [--keep-branch] [--keep-worktree]
ecluse ls [--json]
ecluse validate [--ports]
```

## ecluse init

Detects the right mode for your repo and writes `.ecluse.toml`. Runs interactively — shows the detected mode and asks for confirmation before writing.

| Flag | Description |
|---|---|
| `--mode container\|host\|hybrid` | Override detected mode |
| `--explain` | Show detection signals |
| `--yes` | Skip confirmation prompt |

## ecluse up

Creates a git worktree, allocates a slot, starts services, and writes `.env.ecluse`. Returns the worktree path and all env vars.

| Flag | Description |
|---|---|
| `--branch <name>` | Use a specific branch name instead of the slug |
| `--watch` | Stream service logs after startup |
| `--json` | Output worktree path + env vars as JSON |
| `--reuse-worktree` | Reuse an existing worktree instead of creating one |
| `--port <name>=<value>` | Pin a service to a specific port for this session |

## ecluse down

Tears down services, frees the slot, and removes the worktree.

| Flag | Description |
|---|---|
| `--keep-volumes` | Preserve named Docker volumes |
| `--keep-branch` | Keep the git branch |
| `--keep-worktree` | Keep the worktree directory |

## ecluse shell

Drops into the worktree with all `.env.ecluse` variables loaded in the shell environment. Interactive use only.

If the session has a tmux session (i.e. `process_manager = "tmux"` was set at `ecluse up` time), this attaches to that session instead of spawning a new shell — you'll see the running service windows directly.

## ecluse env

Prints the session's environment variables as JSON. Includes `worktree_path` and all `ECLUSE_*` vars.

## ecluse ls

Lists active sessions. Use `--json` for machine-readable output.

## ecluse validate

Validates port ranges in `.ecluse.toml` and checks for gaps or collisions. Use `--ports` to preview the full port allocation table across all slots. Also checks that the configured `process_manager` binary is installed (e.g. tmux or nohup).
