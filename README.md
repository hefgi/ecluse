<div align="center">

<img src="banner.png" alt="ecluse" width="600" />

**Ephemeral local environments for coding agents — any stack.**

Each git worktree gets its own slot — isolated ports, isolated services, isolated data.
Works whether your stack runs in Docker, on the host, or a mix. No collisions, clean teardown.

[![CI](https://github.com/hefgi/ecluse/actions/workflows/ci.yml/badge.svg)](https://github.com/hefgi/ecluse/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/ecluse.svg)](https://crates.io/crates/ecluse)
[![Homebrew](https://img.shields.io/badge/homebrew-hefgi%2Ftap-orange)](https://github.com/hefgi/homebrew-tap)
[![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](LICENSE)
[![Docs](https://img.shields.io/badge/docs-hefgi.github.io%2Fecluse-blue)](https://hefgi.github.io/ecluse/)

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
npx skills add hefgi/ecluse -g
```

Requires Rust 1.85+. For container and hybrid modes, [OrbStack](https://orbstack.dev) is recommended over Docker Desktop on macOS — faster, less memory.

## Get started

```bash
cd my-project
ecluse init              # detects mode, writes .ecluse.toml
ecluse up feat-foo       # creates worktree + slot
ecluse shell feat-foo    # drops into worktree with env loaded
npm run dev              # PORT already set — app binds to its own port
```

`ecluse init` writes a `.ecluse.toml` at repo root. Here's what a typical one looks like:

```toml
mode = "hybrid"          # container | host | hybrid

[[services]]
name = "api"
base_port = 3000         # slot 1 → PORT=3001, slot 2 → PORT=3002
command = "npm run dev"  # ecluse spawns this; each session gets its own port

[[services]]
name = "postgres"
run = "docker"
base_port = 5432         # slot 1 → ECLUSE_POSTGRES_PORT=5433, slot 2 → 5434

[[services]]
name = "redis"
run = "docker"
base_port = 6379         # slot 1 → ECLUSE_REDIS_PORT=6380, slot 2 → 6381
```

Each `ecluse up` picks the next free slot, starts isolated services, and writes all ports to `.env.ecluse` in the worktree. Type `exit` (or `ecluse down`) to tear everything down.

## Choosing a mode

`ecluse init` detects the right mode automatically. You confirm before anything is written.

| Mode | What `ecluse up` does | Best for |
|---|---|---|
| `container` | Runs all services in Docker (app + data) | Fully containerized stacks, devcontainer repos |
| `hybrid` | Runs data services in Docker, writes env, optionally spawns app | Rails/Django/Node with a postgres+redis compose file |
| `host` | Writes env vars, optionally spawns native services | Pure native stacks with no Docker |

## How it works

The central concept is a **slot** — an integer from 1 to `max_slots`. Every resource is derived from the slot:

- Per-service port: `base_port + slot` (e.g. `api` at `base_port=3000`, slot 1 → 3001, slot 2 → 3002)
- Compose project name: `<prefix>_<slug>`
- Named volumes: `<volume>_<prefix>_<slug>`

Three thin mode implementations share this slot primitive. Mode is selected once at `init` time and stored in `.ecluse.toml`.

**How services are started depends on mode:**

- `container` — everything runs via Docker Compose. ecluse generates a per-slot overlay and calls `docker compose up`.
- `host` / `hybrid` — native services are spawned using your system's process manager. ecluse uses **tmux** if available (one detached session per slot, one window per service), falling back to **nohup** otherwise (background processes with logs at `.ecluse/logs/<slug>/`). Docker data services in hybrid mode still go through Compose. Set `command` on a `[[services]]` entry to opt in; services without `command` are not spawned.

## Commands

```
ecluse init [--mode container|host|hybrid] [--explain] [--yes]
ecluse up [<slug>] [--watch] [--json] [--reuse-worktree] [--port <name>=<value>] [--services <name>,...] [--force] [--skip <name>,...]
ecluse sync [<slug>] [--json]
ecluse shell <slug>
ecluse env [<slug>]
ecluse down [<slug>] [--keep-volumes] [--keep-branch] [--keep-worktree] [--delete-worktree]
ecluse shutdown [--keep-volumes] [--keep-worktrees] [--delete-worktrees]
ecluse flush [--yes]
ecluse ls [--json]
ecluse validate [--ports]
ecluse status [<slug>] [--json] [--quiet]
```

**Env** — get the worktree path and all env vars for a running session as JSON:

```bash
ecluse env feat-foo          # full JSON: worktree_path, slot, all ECLUSE_* vars
ecluse env                   # auto-detects session if run from inside a worktree
```

**Branch names as argument** — pass your git branch name directly; ecluse sanitizes it to a valid slug and uses the original as the branch:

```bash
ecluse up feat/add-auth   # slug=feat-add-auth, branch=feat/add-auth
ecluse up                 # inside a git worktree → auto-detects branch from cwd
ecluse up                 # in repo root / main worktree → prompts for branch name
```

**Auto-register existing worktrees** — running `ecluse up` from inside any git worktree (even one not created by ecluse) auto-detects the branch, registers the session, and starts services. No `--reuse-worktree` flag needed.

**Idempotent up** — `ecluse up` is safe to run on an existing session. It reuses the worktree and slot, checks which services are running, and starts only the ones that are down. Slug is auto-detected from cwd:

```bash
ecluse up feat-foo    # existing session: starts only downed services, skips running ones
ecluse up             # same, slug auto-detected from cwd
ecluse up --force     # kill all running services on allocated ports, restart all
ecluse up --skip api  # skip api; start everything else
```

**Soft restart** — tear down services without losing your worktree, then spin them up fresh:

```bash
ecluse down feat-foo --keep-worktree   # services torn down, worktree + branch kept
ecluse up feat-foo --reuse-worktree    # new slot, fresh ports, worktree reused
```

**Port override** — pin a specific service to a port for this session (useful when the auto-assigned port conflicts with something ecluse can't detect):

```bash
ecluse up feat-foo --port api=4001 --port postgres=5444
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
# Add command = "..." to have ecluse spawn the process on ecluse up.

[[services]]
name = "api"
base_port = 3000             # slot 1 → ECLUSE_API_PORT=3001 + PORT, slot 2 → 3002
command = "npm run dev"      # optional — ecluse spawns this on ecluse up
# port_env = "DJANGO_PORT"  # also inject the port under a custom var name
# port_env = ["DJANGO_PORT", "APP_PORT"]  # or multiple aliases

[[services]]
name = "postgres"
run = "docker"
base_port = 5432             # slot 1 → ECLUSE_POSTGRES_PORT=5433, slot 2 → 5434

# Optional: lifecycle hooks — run in the worktree with all env vars set
[hooks]
on_up = "npx prisma migrate deploy"
on_down = "npx prisma migrate reset --force"
```

`ecluse init` writes `~/.config/ecluse/config.toml` with the detected process manager (`tmux` if installed, otherwise `nohup`). Services with `command` are spawned on `ecluse up` and killed on `ecluse down`. Set `process_manager = "none"` to opt out.

**`[[services]]` for monorepos and multi-service stacks:** define one block per service. Each gets a stable, collision-free port per slot (`base_port + slot`). Omit `[[services]]` entirely for single-service projects — ecluse falls back to a single `PORT = 3000 + slot`.

**Multiple compose files in a monorepo:** point each docker service at its own compose file with the `compose` field (path relative to repo root). Services without `compose` fall back to the root compose file. ecluse generates one overlay per compose file and brings them all up under the same project name.

```toml
[[services]]
name = "api"
base_port = 3000               # native — no compose needed

[[services]]
name = "postgres"
run = "docker"
base_port = 5432               # uses root docker-compose.yml (default)

[[services]]
name = "worker-queue"
run = "docker"
base_port = 6379
compose = "services/worker/docker-compose.yml"   # its own compose file
```

**Port collision handling** — by default ecluse searches for a free port if the nominal one is taken, trying `nominal + i × max_slots` to stay out of other slots' territory. Set `strict_port = true` to fail immediately instead. Run `ecluse validate` to check your config and preview the full port allocation table.

Hooks run as shell commands inside the worktree directory with all `.env.ecluse` variables pre-loaded. Use them for migrations, seeding, or teardown. ecluse doesn't manage databases directly — your app's own tooling handles that via `on_up`.

## Known limits

**Ports are checked, not reserved.** ecluse finds a free port at `ecluse up` time and writes it to `.env.ecluse`. There is a small window between the check and when your process actually binds — if something else takes the port in between, the port in `.env.ecluse` will be wrong. The fix is to tear down and recreate the session:

```bash
ecluse down feat-foo --keep-worktree
ecluse up feat-foo --reuse-worktree
```

Or pin a specific port manually:

```bash
ecluse up feat-foo --port api=4001
```

**Process management is spawn-and-kill only.** For `host` and `hybrid` modes, services with `command` are spawned on `up` and killed on `down`. ecluse does not monitor or restart crashed processes — `ecluse ls` warns if a nohup-managed process has died. For a fresh start, use `ecluse down feat-foo --keep-worktree && ecluse up feat-foo --reuse-worktree`.

**`command` only works if the app reads its port from the environment.** ecluse injects the full `.env.ecluse` contents (all `ECLUSE_*` vars, `PORT`, `port_env` aliases) directly into the spawned process environment — no separate sourcing needed. It cannot help if:
- The port is **hardcoded in source code** — the app must be changed to read `$PORT`.
- The port is **set in a config file** (e.g. `config/puma.rb`, `vite.config.ts`, `.env`) — ecluse does not modify app config files; update the config to read from the environment instead.

If the app reads a custom env var, use `port_env` to inject it under that name:
```toml
port_env = "DJANGO_PORT"                  # single alias
port_env = ["DJANGO_PORT", "APP_PORT"]    # multiple aliases
```

If the framework accepts a CLI flag, pass the var through the command:
```toml
command = "next dev --port $PORT"
command = "bundle exec rails s -p $PORT"
```

## Contributing

Issues and PRs are welcome. Check the [open issues](https://github.com/hefgi/ecluse/issues) for ideas — good first issues are tagged. If you're adding a new isolation mode or provider, open an issue first to discuss the approach.

## License

Apache 2.0. See [LICENSE](LICENSE).
