# Environment variables

Every session writes a `.env.ecluse` file in the worktree. `ecluse up --json` and `ecluse env` return these as JSON.

| Variable | Description |
|---|---|
| `ECLUSE_SLOT` | Slot number (integer, 1–max_slots) |
| `ECLUSE_MODE` | Active mode (`container`, `host`, or `hybrid`) |
| `ECLUSE_SLUG` | Session name (the slug passed to `ecluse up`) |
| `PORT` | Alias for the first native `[[services]]` entry — framework-compatible |
| `ECLUSE_<NAME>_PORT` | Per-service port — one per `[[services]]` entry |

## Example

Config:

```toml
[[services]]
name = "api"
base_port = 3000

[[services]]
name = "postgres"
run = "docker"
base_port = 5432
```

Session `feat-foo` on slot 2:

```
ECLUSE_SLOT=2
ECLUSE_MODE=hybrid
ECLUSE_SLUG=feat-foo
PORT=3002
ECLUSE_API_PORT=3002
ECLUSE_POSTGRES_PORT=5434
```

## Using env vars in your app

For most frameworks, `PORT` is all you need. For multi-service apps, read `ECLUSE_<NAME>_PORT` directly:

```js
// Node
const port = process.env.PORT;
const dbPort = process.env.ECLUSE_POSTGRES_PORT;
```

```python
# Python
import os
port = os.environ["PORT"]
db_port = os.environ["ECLUSE_POSTGRES_PORT"]
```
