# Port allocation

## Formula

Each service gets a port derived from the slot:

```
port = base_port + slot
```

With `base_port = 3000` and `max_slots = 8`:

| Slot | PORT |
|---|---|
| 1 | 3001 |
| 2 | 3002 |
| 3 | 3003 |
| … | … |
| 8 | 3008 |

## Collision handling

By default, ecluse searches for a free port if the nominal one is taken, trying:

```
nominal + i × max_slots
```

This keeps search candidates out of other slots' territory. For example, if slot 1's nominal port 3001 is taken, it tries 3009, 3017, …

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
