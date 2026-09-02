# ecluse

**Your coding agent can't verify its work if it has nowhere to run it.**

ecluse gives every agent its own full stack — isolated ports, isolated services, isolated database. Run 8 agents in parallel, each verifying against a real running system. No collisions, clean teardown.

---

AI writes the code. It can't check the code.

Writing code needs a context window. *Verifying* code needs a running system — a database to migrate, a port to bind, an endpoint to hit, a browser to click through. An agent with no environment can only do the two weakest kinds of verification: re-read its own diff, and run unit tests that mock away the parts where bugs live.

So you get plausible-looking code that nobody ran. And it lands in a human's review queue, which is the one part of the pipeline that doesn't parallelize.

Now try to fix that by running four agents at once. Each needs Postgres, port 3000, and a migrated schema. Port 3000 is taken. Agent B drops Agent A's database. Agent C waits. The verification loop that was supposed to run in parallel is now sequential, and you're paying for four agents to get the throughput of one.

ecluse gives each agent its own slot: isolated ports, its own services, its own data. All four spin up, run the full loop — build, migrate, test, e2e, hit the real endpoints — and tear down independently.

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

## Where ecluse sits

There's a healthy ecosystem of tools for running coding agents in parallel — [Superset](https://github.com/superset-sh/superset), [Gastown](https://github.com/gastownhall/gastown), [claude-squad](https://github.com/smtg-ai/claude-squad), [cmux](https://github.com/craigsc/cmux), [ccmanager](https://github.com/kbwo/ccmanager). Most of them solve *orchestration*: giving each agent a directory, a terminal, a session.

ecluse solves the layer underneath — giving each agent a **running system** to verify against. They compose well together: use an orchestrator for the agents, ecluse for the environments those agents need. And [nono](https://github.com/nolabs-ai/nono) confines what an agent is *allowed* to touch at the kernel level, which is a different problem again — use both.
