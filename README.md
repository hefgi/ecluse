# ecluse

**Per-worktree isolation. Pick what you need isolated.**

ecluse (French for "canal lock") gives you a parallel isolated development environment for each git worktree. Like canal lock chambers — each session gets its own slot, and nothing leaks between them.

```bash
ecluse up feat-foo    # new worktree, isolated ports, isolated database
ecluse up fix-bar     # parallel session, different slot, zero collisions
ecluse down feat-foo  # clean teardown, nothing left behind
```

## Choosing a mode

ecluse supports three isolation modes. Pick the one that fits your stack:

| Mode | What gets isolated | Best for |
|---|---|---|
| `container` | Everything — app and data run in Docker | Fully containerized stacks, devcontainer repos |
| `host` | Ports and databases — app runs natively | Pure native stacks (`npm run dev`, `bin/rails server`) |
| `hybrid` | Data services in containers, app on host | Rails/Django/Node apps with a postgres+redis compose file |

`ecluse init` detects the right mode automatically. You confirm before anything is written.

For a full decision guide: `ecluse skills show choosing-a-mode`

## Install

**macOS (Homebrew — recommended):**

```bash
brew install ecluse/tap/ecluse
```

**From source:**

```bash
cargo install --git https://github.com/ecluse/ecluse
```

Requires Rust 1.85+. For macOS, [OrbStack](https://orbstack.dev) is recommended over Docker Desktop for container and hybrid modes — it's faster and uses less memory.

## Quick start

```bash
cd my-project
ecluse init              # detects mode, writes .ecluse.toml
ecluse up feat-foo       # creates worktree + slot
cd .ecluse/worktrees/feat-foo
source .env.ecluse       # loads PORT, DATABASE_URL, etc.
npm run dev              # or bin/dev, bin/rails server, etc.
```

That's it. Your app runs on a unique port. If database isolation is configured, it has its own database. Other sessions run in parallel without touching yours.

## For coding agents

ecluse is designed as a first-class tool for coding agents running tasks in parallel.

Canonical agent loop:

```bash
ecluse up <task-slug>
cd $(ecluse ls --json | jq -r '.[] | select(.slug=="<task-slug>") | .worktree_path')
source .env.ecluse
# do work
ecluse down <task-slug>
```

The `.env.ecluse` file in every worktree contains everything the agent needs:

| Variable | Description |
|---|---|
| `ECLUSE_SLOT` | Slot number |
| `ECLUSE_OFFSET` | Port offset (`slot × stride`) |
| `ECLUSE_MODE` | Active mode |
| `PORT` | App port (host and hybrid modes) |
| `DATABASE_URL` | Postgres connection string |
| `REDIS_URL` | Redis connection string (if redis present) |

Full agent workflow: `ecluse skills show agent-workflow`

## OrbStack note

On macOS, [OrbStack](https://orbstack.dev) is the recommended Docker runtime for ecluse. It provides faster container startup (~200ms vs ~2s for Docker Desktop) and lower memory overhead. Both runtimes are supported; OrbStack is preferred for developer workstations running multiple parallel sessions.

## How it works

The central concept is a **slot** — an integer from 1 to `max_slots`. Every resource is derived from the slot:

- Port offset: `slot × stride` (default stride: 100)
- Compose project name: `<prefix>_<slug>`
- Named volumes: `<volume>_<prefix>_<slug>`
- Database name: `<base>_<slug>`

Three thin mode implementations share this slot primitive. Mode is selected once at `init` time and stored in `.ecluse.toml`. All three modes use the same four commands.

## Commands

```
ecluse init [--mode container|host|hybrid] [--explain] [--yes]
ecluse up <slug> [--branch <name>] [--watch]
ecluse down <slug> [--keep-volumes] [--keep-database] [--keep-branch]
ecluse ls [--json]
ecluse skills [list | show <name> | install]
```

## Skills

ecluse embeds documentation for humans and agents:

```
ecluse skills list              # list all 8 skills
ecluse skills show agent-workflow
ecluse skills show choosing-a-mode
ecluse skills show hybrid-mode
ecluse skills show troubleshooting
ecluse skills install           # write to .ecluse/skills/ for agent harness access
```

## Configuration

`.ecluse.toml` (written by `init`, lives at repo root):

```toml
mode = "hybrid"
max_slots = 8
stride = 100
prefix = "ecluse"
worktree_dir = ".ecluse/worktrees"
app_label = "ecluse.role"
app_label_value = "app"

# Optional: database provisioning for host and hybrid modes
[database]
provider = "postgres-host"
host = "localhost"
port = 5432
user = "postgres"
base = "myapp"
# password: use PGPASSWORD env var or ~/.pgpass
```

## Hybrid mode setup

Add the `ecluse.role: app` label to your app service in `docker-compose.yml`:

```yaml
services:
  web:
    build: .
    labels:
      ecluse.role: app
    ports: ["3000:3000"]
  postgres:
    image: postgres:16   # no label = data service = containerized
  redis:
    image: redis:7
```

`ecluse up feat-foo` starts postgres and redis in containers with offset ports, creates the worktree, and writes `.env.ecluse` with connection strings pointing at the containerized data services. You run the app yourself.

## Positioning

ecluse is not "Docker for worktrees" — container mode is just one option. It's not "DNS/HTTPS per branch" — those come later. It's a **slot allocator** that promises: for each worktree, you get a clean chamber, and every resource you ask to be isolated is allocated off that chamber's slot.

Compared to alternatives:
- **[branchbox](https://github.com/branchbox/branchbox):** container-only, devcontainer-focused. ecluse supports host and hybrid modes that branchbox doesn't treat as first-class.
- **[Sub-Xaero/wtenv](https://github.com/Sub-Xaero/wtenv):** macOS-only, DNS/HTTPS focus. ecluse is cross-platform and uses `localhost:<port>` — simpler, more portable.

## License

Apache 2.0. See [LICENSE](LICENSE).
