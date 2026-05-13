# `.env.ecluse` Variable Reference

All variables written to `.env.ecluse` in the worktree by `ecluse up`.
Source this file before starting your dev server: `source .env.ecluse`

## Always present

| Variable | Example | Description |
|---|---|---|
| `ECLUSE_SLOT` | `1` | Slot number (integer, 1–max_slots) |
| `ECLUSE_OFFSET` | `100` | Port offset (`slot × stride`) |
| `ECLUSE_MODE` | `hybrid` | Active mode: `container`, `host`, or `hybrid` |

## Port variables (host and hybrid modes)

| Variable | Example | Description |
|---|---|---|
| `PORT` | `3100` | App port — bind your server to this |
| `ECLUSE_APP_PORT` | `3100` | Same as `PORT`; explicit alias |

## Database variables (when provisioned)

| Variable | Example | Description |
|---|---|---|
| `DATABASE_URL` | `postgres://localhost:5432/myapp_feat_foo` | Full connection string |
| `ECLUSE_DATABASE` | `myapp_feat_foo` | Database name only |

## Per-service variables (container and hybrid modes)

For each data service in the compose file:

| Variable | Example | Description |
|---|---|---|
| `ECLUSE_<SERVICE>_PORT` | `ECLUSE_POSTGRES_PORT=5532` | Offset host port for the service |
| `REDIS_URL` | `redis://localhost:6479` | Redis connection string (if redis service found) |

## Slot 1 with stride 100 — example

```bash
ECLUSE_SLOT=1
ECLUSE_OFFSET=100
ECLUSE_MODE=hybrid
PORT=3100
ECLUSE_APP_PORT=3100
DATABASE_URL=postgres://localhost:5532/myapp_feat_foo
ECLUSE_DATABASE=myapp_feat_foo
ECLUSE_POSTGRES_PORT=5532
ECLUSE_REDIS_PORT=6479
REDIS_URL=redis://localhost:6479
```
