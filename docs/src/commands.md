# Commands

```
ecluse init     [--mode container|host|hybrid] [--explain] [--yes] [--quiet]
ecluse up       [<slug>] [--watch] [--json] [--reuse-worktree] [--port <name>=<value>] [--services <name>,...] [--force] [--skip <name>,...] [--quiet]
ecluse sync     [<slug>] [--json] [--quiet]
ecluse down     [<slug>] [--keep-volumes] [--keep-branch] [--keep-worktree] [--delete-worktree] [--quiet]
ecluse shutdown [--keep-volumes] [--keep-worktrees] [--delete-worktrees] [--quiet]
ecluse flush    [--yes] [--quiet]
ecluse ls       [--json]
ecluse shell    [<slug>]
ecluse env      [<slug>]
ecluse validate [--ports] [--quiet]
ecluse status   [<slug>] [--json] [--quiet]
ecluse whose-pid <pid> [--json]
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

When run against a session that already exists, `ecluse up` is idempotent: it reuses the existing worktree and slot, checks which services are running, and starts only the ones that are down. Each service decision is logged explicitly.

The positional argument accepts branch names with slashes — `feat/add-auth` becomes slug `feat-add-auth` while the original branch name is preserved for `git worktree add`.

When called with no argument, slug resolution uses worktree location:
1. **Inside an ecluse-registered worktree** → reuse stored slug/branch
2. **Inside any other git worktree** → read branch from cwd, auto-register (`--reuse-worktree` implied)
3. **In repo root / main worktree** → prompt for branch name

```bash
ecluse up feat/add-auth     # branch=feat/add-auth, slug=feat-add-auth
ecluse up feat-add-auth     # already a valid slug — same result
ecluse up                   # inside any git worktree → auto-detects branch from cwd
ecluse up                   # in repo root → prompts for branch name
ecluse up feat-foo --force      # kill all running services on allocated ports, then restart all
ecluse up --force               # same but slug auto-detected
ecluse up feat-foo --skip api   # skip the api service; start everything else
ecluse up --force --skip postgres  # kill + restart all except postgres
```

| Flag | Description |
|---|---|
| `--watch` | Stream service logs after startup |
| `--json` | Output worktree path + env vars as JSON |
| `--reuse-worktree` | Reuse an existing worktree instead of creating one |
| `--port <name>=<value>` | Pin a service to a specific port for this session |
| `--services <name>,...` | Bring up only this subset of services; unknown names are rejected before any worktree is created |
| `--force` | Kill all running services on allocated ports before starting them; full restart |
| `--skip <name>,...` | Exclude these services entirely (comma-separated); combinable with `--force` |
| `--quiet` | Suppress step output (implied by `--json`) |

## ecluse sync

Registers a manually-started environment with ecluse. Use this when services were started by hand (not via `ecluse up`) or when `state.json` was lost. Slug is auto-detected from cwd when omitted (must be inside `.ecluse/worktrees/<slug>`).

```bash
ecluse sync <slug>          # discover + register
ecluse sync                 # same, slug auto-detected from cwd
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

Tears down services, frees the slot, and prompts before removing the worktree. Slug is auto-detected from cwd when omitted.

The worktree prompt always appears; if the worktree has uncommitted changes an extra warning is shown. Pass `--delete-worktree` to skip the prompt and delete, or `--keep-worktree` to skip the prompt and keep.

| Flag | Description |
|---|---|
| `--keep-volumes` | Preserve named Docker volumes |
| `--keep-branch` | Keep the git branch (no-op — branches are never deleted by ecluse) |
| `--keep-worktree` | Skip prompt, keep the worktree on disk |
| `--delete-worktree` | Skip prompt, delete the worktree (for CI/agents) |
| `--quiet` | Suppress step output |

## ecluse shutdown

Tears down all active sessions at once. Prompts before removing each worktree (same guard as `ecluse down`).

| Flag | Description |
|---|---|
| `--keep-volumes` | Preserve named Docker volumes |
| `--keep-worktrees` | Skip prompt, keep all worktrees on disk |
| `--delete-worktrees` | Skip prompt, delete all worktrees (for CI/agents) |
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

The table shows all allocated ports in a `PORTS` column as `name=value` pairs (e.g. `api=4445 postgres=5433 redis=6380`). A `TMUX` column appears when at least one session uses tmux — the value is the session name you can pass to `tmux attach -t <name>` or `ecluse shell <slug>`.

## ecluse validate

Validates port ranges in `.ecluse.toml` and checks for gaps or collisions. Use `--ports` to preview the full port allocation table across all slots. Also checks that the configured `process_manager` binary is installed (e.g. tmux or nohup).

| Flag | Description |
|---|---|
| `--ports` | Print the full port allocation table for all slots |
| `--quiet` | Suppress step output |

## ecluse status

Shows the health of each service for a session — whether the process is running (native) or the container is up (docker). Exits with code 1 if any service is down.

```bash
ecluse status feat-foo          # table output, exit 1 if any service down
ecluse status feat-foo --json   # machine-readable, same exit code semantics
ecluse status                   # auto-detect slug from cwd (must be inside a worktree)
```

For native services, ecluse matches running processes in the worktree by their command line. For docker services, it queries `docker ps` by container name.

The last column and the session header adapt to the process manager:

- **tmux** — header shows the tmux session name (`tmux=ecluse-<slug>`); last column is `WINDOW` showing the tmux window name for each native service. Health is verified by checking that a process in the pane's subtree owns the expected port — a port collision with an unrelated process correctly shows the service as down.
- **nohup** — last column is `PID` showing the stored process ID.
- **none** (container-only sessions) — no trailing column.

| Flag | Description |
|---|---|
| `--json` | Output as JSON (implies `--quiet`) |
| `--quiet` | Suppress table output — only the exit code is set |

## ecluse whose-pid

Resolves a PID to the ecluse session that owns it. Use this before any manual `kill` of a process on an ecluse-allocated port to avoid killing another agent's working service.

```bash
$ ecluse whose-pid 97469
PID 97469 is owned by session 'feat-payment' (slot 2, service 'api', port 3002)

$ ecluse whose-pid 12345
PID 12345 is not owned by any ecluse session
```

Lookup checks `.ecluse/pids/<slug>/*.pid` files and walks descendants up to 5 levels deep, so processes spawned by `task`, `make`, `npm run`, or any other launcher that's been registered via `ecluse sync` will resolve to the right session. For tmux-managed sessions it also checks pane PIDs and their subtrees.

| Flag | Description |
|---|---|
| `--json` | Output as JSON |

**Exit codes:** `0` if the PID is owned by a tracked ecluse session, `1` if not. This makes the command suitable for scripting:

```bash
if ecluse whose-pid "$PID" --json >/dev/null; then
  echo "PID $PID belongs to another agent — do not kill"
else
  kill "$PID"   # confirmed unowned
fi
```

See the [Agent workflow](agent-workflow.md) page for the canonical "killing services safely" policy.
