<div align="center">

<img src="banner.png" alt="ecluse" width="600" />

**Per-worktree isolation. Pick what you need isolated.**

Each git worktree gets its own slot — isolated ports, its own services, nothing shared.

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
![Pi](https://img.shields.io/badge/Pi-333?style=flat-square)

and any agent that can run shell commands.

</div>

## The problem

You're running 4 Claude Code sessions in parallel. Each agent finishes its task and wants to verify — run the test suite, spin up the app, hit the endpoints. But port 3000 is taken. Agent 2 kills agent 1's server. Agent 3 waits. The verification loop that was supposed to run in parallel is now sequential. You're paying for 4 agents and getting the throughput of one.

ecluse gives each agent its own slot: isolated ports, its own services, its own infra. All 4 agents spin up, verify, and tear down independently. The full AI verification loop — build, migrate, test, e2e — runs in parallel, without collisions, without waiting.

<div align="center">

**Create worktree → Spin up env → Do work → Verify → PR → Teardown**

</div>

```bash
ecluse up feat-foo    # new worktree, isolated ports, isolated services
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
| `ECLUSE_MODE` | Active mode |
| `ECLUSE_SLUG` | Session name |
| `PORT` | Alias for the first native `[[services]]` entry (framework-compatible) |
| `ECLUSE_<NAME>_PORT` | Per-service port — one per `[[services]]` entry |

## Choosing a mode

`ecluse init` detects the right mode automatically. You confirm before anything is written.

| Mode | What gets isolated | Best for |
|---|---|---|
| `container` | Everything — app and data run in Docker | Fully containerized stacks, devcontainer repos |
| `host` | Ports and databases — app runs natively | Pure native stacks (`npm run dev`, `bin/rails server`) |
| `hybrid` | Data services in containers, app on host | Rails/Django/Node apps with a postgres+redis compose file |

## How it works

The central concept is a **slot** — an integer from 1 to `max_slots`. Every resource is derived from the slot:

- Per-service port: `base_port + slot` (e.g. `api` at `base_port=3000`, slot 1 → 3001, slot 2 → 3002)
- Compose project name: `<prefix>_<slug>`
- Named volumes: `<volume>_<prefix>_<slug>`

Three thin mode implementations share this slot primitive. Mode is selected once at `init` time and stored in `.ecluse.toml`.

## Commands

```
ecluse init [--mode container|host|hybrid] [--explain] [--yes]
ecluse up <slug> [--branch <name>] [--watch] [--json]
ecluse shell <slug>
ecluse env [<slug>]
ecluse down <slug> [--keep-volumes] [--keep-branch]
ecluse ls [--json]
ecluse validate [--ports]
```

## Configuration

`.ecluse.toml` lives at repo root, written by `ecluse init`:

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

**`[[services]]` for monorepos and multi-service stacks:** define one block per service. Each gets a stable, collision-free port per slot (`base_port + slot`). Omit `[[services]]` entirely for single-service projects — ecluse falls back to a single `PORT = 3000 + slot`.

**Port collision handling** — by default ecluse searches for a free port if the nominal one is taken, trying `nominal + i × max_slots` to stay out of other slots' territory. Set `strict_port = true` to fail immediately instead. Run `ecluse validate` to check your config and preview the full port allocation table.

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

Or define `run = "docker"` services in `[[services]]` to explicitly control which services stay in containers. Either approach works — the label is the simpler default for single-app stacks.

`ecluse up feat-foo` starts postgres and redis in containers with per-slot ports, creates the worktree, and writes `.env.ecluse` with `ECLUSE_POSTGRES_PORT` and `ECLUSE_REDIS_PORT` pointing at the containerized data services. You run the app yourself.

## Contributing

Issues and PRs are welcome. Check the [open issues](https://github.com/hefgi/ecluse/issues) for ideas — good first issues are tagged. If you're adding a new isolation mode or provider, open an issue first to discuss the approach.

## License

Apache 2.0. See [LICENSE](LICENSE).
