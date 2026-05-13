<div align="center">

<img src="banner.png" alt="ecluse" width="600" />

**Per-worktree isolation. Pick what you need isolated.**

Each git worktree gets its own slot — isolated ports, its own database, its own infra.

[![CI](https://github.com/hefgi/ecluse/actions/workflows/ci.yml/badge.svg)](https://github.com/hefgi/ecluse/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/ecluse.svg)](https://crates.io/crates/ecluse)
[![Homebrew](https://img.shields.io/badge/homebrew-hefgi%2Ftap-orange)](https://github.com/hefgi/homebrew-tap)
[![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](LICENSE)

---

**Built for coding agents running tasks in parallel.**

![Claude Code](https://img.shields.io/badge/Claude_Code-d97706?style=flat-square)
![Cursor](https://img.shields.io/badge/Cursor-000?style=flat-square)
![Codex](https://img.shields.io/badge/Codex-10a37f?style=flat-square)
![OpenCode](https://img.shields.io/badge/OpenCode-6366f1?style=flat-square)
![Pи](https://img.shields.io/badge/Pi-111?style=flat-square)

and any agent that can run shell commands.

</div>

## The problem

You're running 4 Claude Code sessions in parallel. Each agent finishes its task and wants to verify — run the test suite, spin up the app, hit the endpoints. But port 3000 is taken. Agent 2 kills agent 1's server. Agent 3 waits. The verification loop that was supposed to run in parallel is now sequential. You're paying for 4 agents and getting the throughput of one.

ecluse gives each agent its own slot: isolated ports, its own database, its own infra. All 4 agents spin up, verify, and tear down independently. The full AI verification loop — build, migrate, test, e2e — runs in parallel, without collisions, without waiting.

```bash
ecluse up feat-foo    # new worktree, isolated ports, isolated database
ecluse up fix-bar     # parallel session, different slot, zero collisions
ecluse down feat-foo  # clean teardown, nothing left behind
```

> ecluse is French for "canal lock" — each session gets its own chamber, everything is isolated, nothing leaks between them.

## Install

[![Homebrew](https://img.shields.io/badge/Homebrew-FBB040?style=flat-square&logo=homebrew&logoColor=black)](https://github.com/hefgi/homebrew-tap)

```bash
brew install hefgi/tap/ecluse
```

[![Crates.io](https://img.shields.io/badge/cargo-install-orange?style=flat-square&logo=rust&logoColor=white)](https://crates.io/crates/ecluse)

```bash
cargo install ecluse
```

Then install the agent skill:

```bash
npx skills add hefgi/ecluse -y
```

Requires Rust 1.85+. For container and hybrid modes, [OrbStack](https://orbstack.dev) is recommended over Docker Desktop on macOS — faster, less memory.

## Get started

```bash
cd my-project
ecluse init              # detects mode, writes .ecluse.toml
ecluse up feat-foo       # creates worktree + slot
ecluse shell feat-foo    # drops into worktree with env loaded
npm run dev              # PORT, DATABASE_URL, etc. already set
```

Your app runs on a unique port. Other sessions run in parallel without touching yours. Type `exit` to leave the session.

## Agent skills

The skill teaches your agent every command, mode, and workflow. Install globally or project-local:

| | Command |
|---|---|
| Global | `npx skills add hefgi/ecluse -y` |
| Project-local | `npx skills add hefgi/ecluse -y --out .` |

Canonical agent loop:

```bash
# 1. Create session — get worktree path + full env in one call
ecluse up <task-slug> --json   # returns JSON with worktree_path and all env vars

# 2. Work in the worktree (path from JSON above)
# Edit files, run commands — env vars are in the JSON output

# 3. Tear down
ecluse down <task-slug>
```

Or query an existing session anytime:
```bash
ecluse env <task-slug>   # JSON: worktree_path + all env vars
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

## Choosing a mode

`ecluse init` detects the right mode automatically. You confirm before anything is written.

| Mode | What gets isolated | Best for |
|---|---|---|
| `container` | Everything — app and data run in Docker | Fully containerized stacks, devcontainer repos |
| `host` | Ports and databases — app runs natively | Pure native stacks (`npm run dev`, `bin/rails server`) |
| `hybrid` | Data services in containers, app on host | Rails/Django/Node apps with a postgres+redis compose file |

## How it works

The central concept is a **slot** — an integer from 1 to `max_slots`. Every resource is derived from the slot:

- Port: `base_port + slot × stride` (defaults: base 3000, stride 100)
- Compose project name: `<prefix>_<slug>`
- Named volumes: `<volume>_<prefix>_<slug>`
- Database name: `<base>_<slug>`

Three thin mode implementations share this slot primitive. Mode is selected once at `init` time and stored in `.ecluse.toml`.

## Commands

```
ecluse init [--mode container|host|hybrid] [--explain] [--yes]
ecluse up <slug> [--branch <name>] [--watch] [--json]
ecluse shell <slug>
ecluse env [<slug>]
ecluse down <slug> [--keep-volumes] [--keep-branch]
ecluse ls [--json]
```

## Configuration

`.ecluse.toml` lives at repo root, written by `ecluse init`:

```toml
mode = "hybrid"
max_slots = 8
base_port = 3000
stride = 100
prefix = "ecluse"
worktree_dir = ".ecluse/worktrees"
app_label = "ecluse.role"
app_label_value = "app"

# Optional: lifecycle hooks — run in the worktree with all env vars set
[hooks]
on_up = "npx prisma migrate deploy"
on_down = "psql $DATABASE_URL -c 'DROP DATABASE $ECLUSE_SLUG'"
```

Hooks run as shell commands inside the worktree directory with all `.env.ecluse` variables pre-loaded. Use them for migrations, seeding, or teardown. ecluse doesn't manage databases directly — your app's own tooling handles that via `on_up`.

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

## Contributing

Issues and PRs are welcome. Check the [open issues](https://github.com/hefgi/ecluse/issues) for ideas — good first issues are tagged. If you're adding a new isolation mode or provider, open an issue first to discuss the approach.

## License

Apache 2.0. See [LICENSE](LICENSE).
