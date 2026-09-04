# Port allocation

## Why not let Docker assign ports?

Docker prevents container bind failures — but ecluse solves a different problem.

A session has native processes (your app server, dev watcher) alongside containers. Docker has no authority over native ports, so two branches both binding port 3000 natively will still collide. And even for containers, Docker can only assign a port *after* startup — but your native app needs `DATABASE_URL` with the right port *before* it starts.

Ecluse pre-allocates a consistent, non-overlapping port set for the entire session — native and container alike — so everything has the right addresses before anything starts.

## Formula

Each service gets a port derived from the slot:

```
port = base_port + slot × slot_stride
```

With `base_port = 3000`, `max_slots = 8`, and the default `slot_stride = 1`:

| Slot | PORT |
|---|---|
| 1 | 3001 |
| 2 | 3002 |
| 3 | 3003 |
| … | … |
| 8 | 3008 |

### Spacing slots further apart with `slot_stride`

When parallel agents work on the same repo, adjacent-slot ports (`3001` and `3002`) are easy to confuse — an agent might see "a process on the port next to mine" and assume it's a stale leftover when it actually belongs to another worktree. Set `slot_stride` in `.ecluse.toml` to widen the gap:

```toml
slot_stride = 10
```

With `slot_stride = 10`:

| Slot | PORT |
|---|---|
| 1 | 3010 |
| 2 | 3020 |
| 3 | 3030 |
| … | … |

Wider stride doesn't prevent every confusion — the canonical fix for "the wrong process is on my port" is still `ecluse down --keep-worktree` + `ecluse up --reuse-worktree` (see the [Agent workflow](agent-workflow.md) page) — but it makes adjacent-slot ports visually distinct in `lsof` output and gives `extra_ports` more room to coexist with primary service ports.

## Collision handling

By default, ecluse searches for a free port if the nominal one is taken, trying:

```
nominal + i × max_slots × slot_stride
```

This keeps search candidates out of other slots' territory. For example, with `slot_stride = 1` and `max_slots = 8`, if slot 1's nominal port 3001 is taken, ecluse tries 3009, 3017, …

Set `strict_port = true` in `.ecluse.toml` to fail immediately instead of searching.

`port_search_range` controls how many alternatives to try (default: 10).

## Validation

Run `ecluse validate --ports` to preview the full port allocation table and check for overlaps:

```
$ ecluse validate --ports
slot  api    postgres  redis
1     3001   5433      6380
2     3002   5434      6381
3     3003   5435      6382
…
```

## Port override

Pin a specific service to a port for a session (useful when the auto-assigned port conflicts with something ecluse can't detect):

```bash
ecluse up feat-foo --port api=4001 --port postgres=5444
```

## Discovery: what is actually listening

Assignment answers "which port should this service use". Discovery answers "which port is it really on". `ecluse ls` and `ecluse status` report both, so a service that bound the wrong port is visible instead of just looking down.

Discovery runs when you invoke a command — there is no background daemon. It takes one snapshot of every listening TCP socket plus the process table (two subprocess calls total, regardless of how many sessions exist), then attributes each listener to the session whose process tree owns it.

```
$ ecluse status feat-a
SERVICE  TYPE     EXPECTED  ACTUAL  STATUS
api      native   4010      4020    ✗ wrong port 4020 (slot 2)
```

Because ports are derived from the slot, the formula inverts: a discovered port can be mapped back to the slot that owns it. When the wrong port belongs to another slot, ecluse names that slot and its session, and says explicitly not to kill it — under parallel sessions the process on a neighbouring port is almost always another agent's working service.

**Discovery never overwrites assignment.** `state.json` remains the source of truth. A discovered port that disagrees with the assigned one is evidence of a bug — typically an external task runner (`task`, `make`, `npm run`) that re-read `.env.local` instead of `.env.ecluse` — not a better value to adopt. Trusting discovery is what once hid a wrong-slot spawn behind a green check while three agents killed each other's services. The fix for a mismatch is always:

```bash
ecluse down <slug> --keep-worktree && ecluse up <slug>
```

Only a *missing* assigned port counts as a mismatch. The extra sockets a dev server opens (HMR, debug, inspector) show up in the discovered set without flagging anything.

## Known limitation

Ports are checked, not reserved. ecluse finds a free port at `ecluse up` time and writes it to `.env.ecluse`. There is a small window between the check and when your process actually binds — if something else takes the port in between, the port in `.env.ecluse` will be wrong. The fix:

```bash
ecluse down feat-foo --keep-worktree
ecluse up feat-foo --reuse-worktree
```
