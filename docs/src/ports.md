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

## Known limitation

Ports are checked, not reserved. ecluse finds a free port at `ecluse up` time and writes it to `.env.ecluse`. There is a small window between the check and when your process actually binds — if something else takes the port in between, the port in `.env.ecluse` will be wrong. The fix:

```bash
ecluse down feat-foo --keep-worktree
ecluse up feat-foo --reuse-worktree
```
