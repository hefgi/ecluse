<div align="center">

<img src="banner.png" alt="ecluse" width="600" />

**Your coding agent can't verify its work if it has nowhere to run it.**

ecluse gives every agent its own full stack — isolated ports, isolated services,
isolated database. Run 8 agents in parallel, each verifying against a real running
system. No collisions, clean teardown.

[![CI](https://github.com/hefgi/ecluse/actions/workflows/ci.yml/badge.svg)](https://github.com/hefgi/ecluse/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/ecluse.svg)](https://crates.io/crates/ecluse)
[![Homebrew](https://img.shields.io/badge/homebrew-hefgi%2Ftap-orange)](https://github.com/hefgi/homebrew-tap)
[![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](LICENSE)
[![Docs](https://img.shields.io/badge/docs-ecluse.ai-blue)](https://ecluse.ai/)

<!-- TODO(demo): replace with demo.gif — 4 agents, 4 stacks, one laptop, no collisions.
     See docs/DEMO-SCRIPT.md for the shot list and recording commands. -->
<img src="docs/demo.gif" alt="Four coding agents, four isolated stacks, one laptop" width="800" />

---

**Works with every agent that runs in a terminal.**

![Claude Code](https://img.shields.io/badge/Claude_Code-d97706?style=flat-square)
![Cursor](https://img.shields.io/badge/Cursor-000?style=flat-square)
![Codex](https://img.shields.io/badge/Codex-10a37f?style=flat-square)
![OpenCode](https://img.shields.io/badge/OpenCode-6366f1?style=flat-square)
![Pi](https://img.shields.io/badge/Pi-333?style=flat-square)

</div>

## The problem

AI writes the code. It can't check the code.

Writing code needs a context window. *Verifying* code needs a running system — a
database to migrate, a port to bind, an endpoint to hit, a browser to click through.
An agent with no environment can only do the two weakest kinds of verification:
re-read its own diff, and run unit tests that mock away the parts where bugs live.

So you get plausible-looking code that nobody ran. And it lands in a human's review
queue, which is the one part of the pipeline that doesn't parallelize.

Now try to fix that by running four agents at once:

```
Agent A → needs Postgres, port 3000, a migrated schema
Agent B → needs Postgres, port 3000, a migrated schema
Agent C → needs Postgres, port 3000, a migrated schema
Agent D → needs Postgres, port 3000, a migrated schema
```

Port 3000 is taken. Agent B drops Agent A's database. Agent C waits. The verification
loop that was supposed to run in parallel is now sequential, and you're paying for four
agents to get the throughput of one.

## What ecluse does

One command per agent. Each gets its own slot: its own ports, its own services, its own
data, torn down cleanly when it's done.

```bash
ecluse up feat-foo    # new worktree, isolated ports, isolated services
ecluse up fix-bar     # parallel session, different slot, zero collisions
ecluse down feat-foo  # clean teardown, nothing left behind
```

All four agents spin up, run the full loop — build, migrate, test, e2e, hit the real
endpoints — and tear down independently.

<div align="center">

**Create worktree → Spin up env → Do work → Verify → PR → Teardown**

</div>

> ecluse is French for "canal lock" — each session gets its own chamber, everything is
> isolated, nothing leaks between them.

## Install

[![Homebrew](https://img.shields.io/badge/Homebrew-FBB040?style=flat-square&logo=homebrew&logoColor=black)](https://github.com/hefgi/homebrew-tap)

```bash
brew install hefgi/tap/ecluse
```

[![Crates.io](https://img.shields.io/badge/cargo-install-orange?style=flat-square&logo=rust&logoColor=white)](https://crates.io/crates/ecluse)

```bash
cargo install ecluse
```

Then install the agent skill, so your agent knows how to drive it:

```bash
npx skills add hefgi/ecluse -g
```

Requires Rust 1.85+. For container and hybrid modes, [OrbStack](https://orbstack.dev) is
recommended over Docker Desktop on macOS — faster, less memory.

## Get started

```bash
cd my-project
ecluse init              # detects mode, writes .ecluse.toml
ecluse up feat-foo       # creates worktree + slot
ecluse shell feat-foo    # drops into worktree with env loaded
npm run dev              # PORT already set — app binds to its own port
```

`ecluse init` writes a `.ecluse.toml` at repo root. A typical one:

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

Each `ecluse up` picks the next free slot, starts isolated services, and writes all ports
to `.env.ecluse` in the worktree. Type `exit` (or `ecluse down`) to tear everything down.

Migrations and seeding go in [hooks](https://ecluse.ai/hooks.html) — `post_up` runs with
the full environment available, so each agent gets a migrated database of its own.

📖 **[Full configuration reference →](https://ecluse.ai/configuration.html)**
· [Commands](https://ecluse.ai/commands.html)
· [Port allocation](https://ecluse.ai/ports.html)
· [Agent workflow](https://ecluse.ai/agent-workflow.html)
· [Known limits](https://ecluse.ai/limits.html)

## Choosing a mode

`ecluse init` detects the right mode automatically. You confirm before anything is written.

| Mode | What `ecluse up` does | Best for |
|---|---|---|
| `container` | Runs all services in Docker (app + data) | Fully containerized stacks, devcontainer repos |
| `hybrid` | Runs data services in Docker, writes env, optionally spawns app | Rails/Django/Node with a postgres+redis compose file |
| `host` | Writes env vars, optionally spawns native services | Pure native stacks with no Docker |

## How it works

The central concept is a **slot** — an integer from 1 to `max_slots`. Every resource is
derived from it:

- Per-service port: `base_port + slot` (`api` at `base_port=3000` → slot 1 gets 3001, slot 2 gets 3002)
- Compose project name: `<prefix>_<slug>`
- Named volumes: `<volume>_<prefix>_<slug>`

Three thin mode implementations share this one primitive. Mode is selected once at `init`
time and stored in `.ecluse.toml`.

**How services start depends on mode:**

- `container` — everything runs via Docker Compose. ecluse generates a per-slot overlay and calls `docker compose up`.
- `host` / `hybrid` — native services are spawned using your system's process manager: **tmux** if available (one detached session per slot, one window per service), falling back to **nohup** (logs at `.ecluse/logs/<slug>/`). Docker data services in hybrid mode still go through Compose. Set `command` on a `[[services]]` entry to opt in; services without `command` are not spawned.

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
ecluse whose-pid <pid> [--json]
```

`ecluse up` is idempotent, auto-detects the slug from your cwd, auto-registers existing
worktrees, and accepts branch names directly (`ecluse up feat/add-auth`). `ecluse env`
emits the worktree path and every `ECLUSE_*` variable as JSON, which is how agents
discover their own environment.

📖 **[Full command reference →](https://ecluse.ai/commands.html)**

**Port discovery** — `ecluse ls` and `ecluse status` also report the port each service is
*actually* listening on next to the one ecluse assigned, so a service that bound the wrong
port shows up as wrong rather than merely down. Discovery runs on invocation; there's no daemon.

```
$ ecluse status feat-a
SERVICE  TYPE     EXPECTED  ACTUAL  STATUS
api      native   4010      4020    ✗ wrong port 4020 (slot 2)

warning: service 'api' is listening on 4020 but ecluse assigned 4010; 4020 belongs
to slot 2 (session 'feat-b') — do not kill it, run: ecluse down feat-a
--keep-worktree && ecluse up feat-a
```

Assignment stays the source of truth — a discovered port is reported, never written back
over it. Under parallel sessions the process on a neighbouring port is almost always
another agent's working service, so the remedy is `down --keep-worktree` + `up`, not `kill`.

**Soft restart** — tear down services without losing your worktree, then spin them up fresh:

```bash
ecluse down feat-foo --keep-worktree   # services torn down, worktree + branch kept, slot reserved
ecluse up feat-foo                      # resumes at the same slot; ports are re-probed (stopped session auto-detected)
```

A stopped session keeps its slot until its worktree is deleted. Running `--keep-worktree` again does not free it. If you hit "all N slots are in use", check `ecluse ls` for `(stopped)` sessions and run `ecluse down <slug> --delete-worktree` on the ones you no longer need. When you are done with a branch, use `--delete-worktree` rather than `--keep-worktree`.

## How ecluse compares

There's a healthy ecosystem of tools for running coding agents in parallel. Most of them
solve *orchestration* — giving each agent a directory, a terminal, a session. ecluse
solves the layer underneath: giving each agent a **running system** to verify against.

| | Worktree per agent | Isolated ports | Isolated services + DB | Clean teardown | Cross-platform |
|---|---|---|---|---|---|
| **ecluse** | ✅ | ✅ | ✅ | ✅ | ✅ macOS/Linux/WSL2 |
| [Superset](https://github.com/superset-sh/superset) | ✅ | ✅ detection | ⚠️ setup scripts | — | macOS (Linux exp.) |
| [claude-squad](https://github.com/smtg-ai/claude-squad) | ✅ | — | — | — | ✅ |
| [Gastown](https://github.com/gastownhall/gastown) | ✅ | — | — | — | ✅ |
| [ccmanager](https://github.com/kbwo/ccmanager) | ✅ | — | — | — | ✅ |
| [cmux](https://github.com/craigsc/cmux) | ✅ | — | — | — | macOS |
| plain `git worktree` | ✅ | — | — | — | ✅ |

**They're complements, not competitors** — and ecluse is designed to compose with them:

- **[Superset](https://github.com/superset-sh/superset)** gives you a polished agentic IDE with a diff viewer, in-app browser, and 100+ parallel agents. It detects ports but leaves service and database provisioning to your own setup scripts. Point those scripts at `ecluse up` and each workspace gets a real stack.
- **[Gastown](https://github.com/gastownhall/gastown)** coordinates 20–30 agents with git-backed work tracking. ecluse uses tmux as a process manager too, so the two sit naturally side by side.
- **[cmux](https://github.com/craigsc/cmux)**'s docs recommend pairing it with a worktree manager. That's this.
- **[nono](https://github.com/nolabs-ai/nono)** confines what an agent is *allowed* to touch, at the kernel. ecluse provisions what an agent *needs to run*. Security and capability are different problems — use both.

If you want a GUI, agent orchestration, or session multiplexing, use one of the above.
Use ecluse when you need the agents to have somewhere real to verify.

## Known limits

Three things worth knowing before you rely on this:

**Ports are checked, not reserved.** ecluse finds a free port at `ecluse up` time and
writes it to `.env.ecluse`. There's a small window before your process binds — if
something else takes the port in between, the value in `.env.ecluse` is wrong. Fix by
recreating the session (`ecluse down --keep-worktree && ecluse up`) or pinning
(`ecluse up feat-foo --port api=4001`).

**Process management is spawn-and-kill only.** Services with `command` are spawned on
`up` and killed on `down`. ecluse doesn't monitor or restart crashed processes, though
`ecluse ls` warns if a nohup-managed process has died.

**`command` only works if your app reads its port from the environment.** ecluse injects
the full `.env.ecluse` into the spawned process, but it can't help if the port is
hardcoded in source or set in a config file. Use `port_env` for custom variable names, or
pass it through the command (`command = "next dev --port $PORT"`).

📖 **[Full details and workarounds →](https://ecluse.ai/limits.html)**

## Contributing

Issues and PRs are welcome — check the [open issues](https://github.com/hefgi/ecluse/issues),
where good first issues are tagged. If you're adding an isolation mode or an execution
provider, open an issue first so we can talk through the approach.

Questions and ideas are welcome in
[Discussions](https://github.com/hefgi/ecluse/discussions).

## License

Apache 2.0. See [LICENSE](LICENSE).
