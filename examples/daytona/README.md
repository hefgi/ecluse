# daytona

Run ecluse inside a [Daytona](https://daytona.io) sandbox instead of on your laptop.

ecluse keeps its state per repo (`<repo>/.ecluse/state.json`), so a sandbox gets its own slots and never collides with sessions you run locally. The commands are the same everywhere. What changes is the machine around them: the sandbox needs a Docker daemon, the binaries your hooks call, your untracked files, and a way for your browser to reach the ports.

The example app is an Express API + Vite frontend with Postgres and Redis, the same shape as [`node-hybrid`](../node-hybrid).

## Mode

`hybrid`: Postgres and Redis run in containers on the sandbox's Docker daemon (Docker-in-Docker); API and web run natively in tmux.

## Files

| File | Purpose |
|------|---------|
| `.ecluse.toml` | Hybrid config; `pre_spawn` writes `.env.local`, waits for Postgres, migrates |
| `docker-compose.yml` | Postgres + Redis; ecluse rewrites host ports per slot |
| `Dockerfile` | Sandbox image: `docker:dind` + git, tmux, lsof, jq, Node, `pg_isready`, ecluse |
| `bootstrap.sh` | Run once per sandbox: starts `dockerd`, checks binaries, sets `PUBLIC_URL_TEMPLATE` |
| `forward-ports.sh` | Run on your laptop: SSH-forwards every port of a session to `localhost` |

## Setup

### 1. Build a snapshot from the Dockerfile

```sh
daytona snapshot create ecluse-hybrid \
  --dockerfile examples/daytona/Dockerfile \
  --context examples/daytona   # the Dockerfile COPYs bootstrap.sh from here
```

Pin a different ecluse release with `--build-arg ECLUSE_VERSION=vX.Y.Z` if needed. Check `daytona snapshot create --help` for the flags your CLI version supports.

### 2. Create a sandbox and clone the repo

```sh
daytona create --snapshot ecluse-hybrid
daytona ssh <sandbox>
git clone <your-repo> app && cd app
```

### 3. Copy untracked files in

`git clone` only brings tracked files. Copy in anything ecluse or your hooks read that is gitignored, such as `.env` secrets, a symlinked `.ecluse.toml`, or a compose file under `.ecluse/`:

```sh
# from your laptop
scp .env <ssh-destination>:app/.env
```

### 4. Bootstrap and bring a session up

```sh
ecluse-bootstrap            # SSH-forwarding mode (browser uses localhost)
# or
ecluse-bootstrap <sandbox-id>   # preview-URL mode (browser uses *.proxy.daytona.works)

source ~/.bashrc
ecluse up feat-foo
```

## Reaching the app from your browser

The services listen on `localhost` **inside the sandbox**. Server-to-server traffic (API → Postgres, API → Redis) works without changes. URLs that your browser follows are different, because the browser runs on your laptop.

**Option A: SSH port forwarding (recommended).** All URLs stay `http://localhost:<port>`, same as running locally:

```sh
./examples/daytona/forward-ports.sh <ssh-destination> app feat-foo
# localhost:3001 -> sandbox:3001
# localhost:5174 -> sandbox:5174
# ...
```

**Option B: Daytona preview URLs.** Run `ecluse-bootstrap <sandbox-id>`. The `pre_spawn` hook then writes `VITE_API_URL=https://3001-<sandbox-id>.proxy.daytona.works` instead of `http://localhost:3001`. Anything else that hardcodes `localhost` in browser-facing config needs the same treatment. Check your API's CORS settings too.

## Environment variables set by ecluse

| Variable               | Slot 1 | Description |
|------------------------|--------|-------------|
| `ECLUSE_SLUG`          | —      | Session slug |
| `ECLUSE_API_PORT`      | 3001   | Express API |
| `ECLUSE_WEB_PORT`      | 5174   | Vite dev server (bound to `0.0.0.0` so preview URLs can reach it) |
| `ECLUSE_POSTGRES_PORT` | 5433   | Per-slot Postgres host port |
| `ECLUSE_REDIS_PORT`    | 6380   | Per-slot Redis host port |

## Hooks

- `pre_spawn`: writes `.env.local` (DB/Redis URLs + browser-facing API URL), waits up to 30s for Postgres with `pg_isready`, then runs `npm ci` and `npm run db:migrate`. Everything happens before api/web start, so they boot into a ready environment.

## Gotchas

- **No Docker daemon, no hybrid mode.** If `bootstrap.sh` can't start `dockerd`, the sandbox lacks the privileges Docker-in-Docker needs. Use `host` mode with Postgres/Redis installed natively in the image instead.
- **Hooks that reference laptop paths** (`~/Desktop/...`) silently do nothing in the sandbox if they're guarded with `[ -f ... ]`. Clone those repos into the sandbox too, or drop the step.
- **Missing `pg_isready`** makes a wait loop like the one above spin for its full timeout and then fail on the migration. The explicit `pg_isready` after the loop surfaces the real error.
- **Stopped sandboxes** keep `.ecluse/state.json` but not running processes or containers. After a restart, run `ecluse-bootstrap`, then `ecluse flush` and `ecluse up` again.
