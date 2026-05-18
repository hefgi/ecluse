# Agent workflow

ecluse is designed for coding agents running tasks in parallel. The canonical loop:

```bash
# 1. Create session — get worktree path + full env in one call
ecluse up <task-slug> --json   # returns JSON with worktree_path and all env vars

# 2. Work in the worktree (path from JSON above)
# Edit files, run commands — env vars are in the JSON output

# 3. Tear down
ecluse down <task-slug>
```

## JSON output

`ecluse up --json` returns everything the agent needs:

```json
{
  "worktree_path": "/path/to/repo/.ecluse/worktrees/feat-foo",
  "slot": 2,
  "mode": "hybrid",
  "slug": "feat-foo",
  "env": {
    "ECLUSE_SLOT": "2",
    "ECLUSE_MODE": "hybrid",
    "ECLUSE_SLUG": "feat-foo",
    "PORT": "3002",
    "ECLUSE_API_PORT": "3002",
    "ECLUSE_POSTGRES_PORT": "5434"
  }
}
```

Query an existing session anytime:

```bash
ecluse env <task-slug>   # same JSON shape
```

## Parallel sessions

Each agent gets its own slot. Sessions never share ports or volumes:

```bash
# Agent 1
ecluse up feat-payment --json   # slot 1, PORT=3001

# Agent 2
ecluse up feat-auth --json      # slot 2, PORT=3002

# Agent 3
ecluse up fix-bug-123 --json    # slot 3, PORT=3003
```

All three run the full verification loop simultaneously — build, migrate, test, e2e — without waiting for each other.

## Recovering a manually-started environment

If services were started by hand (not via `ecluse up`) or `state.json` was lost, use `ecluse sync` to register the running session:

```bash
ecluse sync <slug> --json   # discover processes, register session, return env JSON
```

After sync, `ecluse ls`, `ecluse env`, and `ecluse down` all work normally. The discovered ports appear in `port_overrides` and `.env.ecluse` is rewritten to reflect reality.

## Install the skill

The skill teaches your agent every command, failure mode, and config option. Install it so your agent doesn't have to figure this out from scratch:

```bash
npx skills add hefgi/ecluse -g
```
