# ecluse

**Ephemeral local environments for coding agents — any stack.**

Each git worktree gets its own slot — isolated ports, isolated services, isolated data. Works whether your stack runs in Docker, on the host, or a mix. No collisions, clean teardown.

---

You're running 4 Claude Code sessions in parallel. Each agent finishes its task and wants to verify — run the test suite, spin up the app, hit the endpoints. But port 3000 is taken. Agent 2 kills agent 1's server. Agent 3 waits. The verification loop that was supposed to run in parallel is now sequential. You're paying for 4 agents and getting the throughput of one.

ecluse gives each agent its own slot: isolated ports, its own services, its own infra. All 4 agents spin up, verify, and tear down independently. The full AI verification loop — build, migrate, test, e2e — runs in parallel, without collisions, without waiting.

```
Create worktree → Spin up env → Do work → Verify → PR → Teardown
```

```bash
ecluse up feat-foo    # new worktree, isolated ports, isolated services
ecluse up fix-bar     # parallel session, different slot, zero collisions
ecluse down feat-foo  # clean teardown, nothing left behind
```

> ecluse is French for "canal lock" — each session gets its own chamber, everything is isolated, nothing leaks between them.

## How it works

The central concept is a **slot** — an integer from 1 to `max_slots`. Every resource is derived from the slot:

- Per-service port: `base_port + slot` (e.g. `api` at `base_port=3000`, slot 1 → 3001, slot 2 → 3002)
- Compose project name: `<prefix>_<slug>`
- Named volumes: `<volume>_<prefix>_<slug>`

Three thin mode implementations share this slot primitive. Mode is selected once at `init` time and stored in `.ecluse.toml`.
