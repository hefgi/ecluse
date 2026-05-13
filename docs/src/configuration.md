# Configuration

`.ecluse.toml` lives at repo root and is written by `ecluse init`. All fields are optional except `mode`.

```toml
mode = "hybrid"
max_slots = 8
prefix = "ecluse"
worktree_dir = ".ecluse/worktrees"

# Port collision handling (both optional)
# strict_port = false        # default: search for a free port on collision
# port_search_range = 10     # how many alternatives to try (bump by max_slots each time)

# One [[services]] block per service. port = base_port + slot.
# Native services run on the host; docker services run in containers.
# The first native entry also sets the PORT alias for framework compatibility.

[[services]]
name = "api"
base_port = 3000   # slot 1 → ECLUSE_API_PORT=3001 + PORT, slot 2 → 3002

[[services]]
name = "postgres"
run = "docker"
base_port = 5432   # slot 1 → ECLUSE_POSTGRES_PORT=5433, slot 2 → 5434

# Optional: lifecycle hooks — run in the worktree with all env vars set
[hooks]
on_up = "npx prisma migrate deploy"
on_down = "npx prisma migrate reset --force"
```

## Fields

| Field | Type | Default | Description |
|---|---|---|---|
| `mode` | string | — | `container`, `host`, or `hybrid` |
| `max_slots` | integer | `8` | Maximum parallel sessions |
| `prefix` | string | `"ecluse"` | Prefix for compose project names and volume names |
| `worktree_dir` | string | `".ecluse/worktrees"` | Directory for git worktrees |
| `strict_port` | bool | `false` | Fail immediately on port collision instead of searching |
| `port_search_range` | integer | `10` | How many alternatives to try on collision |

## `[[services]]`

Each `[[services]]` block defines one service. Each gets a stable, collision-free port per slot (`base_port + slot`).

| Field | Type | Description |
|---|---|---|
| `name` | string | Service name — becomes `ECLUSE_<NAME>_PORT` |
| `base_port` | integer | Port formula: `base_port + slot` |
| `run` | string | `"docker"` to run in a container; omit for native |

The first native (non-docker) service entry also sets `PORT` for framework compatibility.

Omit `[[services]]` entirely for single-service projects — ecluse falls back to `PORT = 3000 + slot`.

## `[hooks]`

| Field | Description |
|---|---|
| `on_up` | Shell command run after services start, inside the worktree with all env vars loaded |
| `on_down` | Shell command run before services stop |

Use hooks for migrations, seeding, or teardown. ecluse doesn't manage databases directly — your app's own tooling handles that via `on_up`.
