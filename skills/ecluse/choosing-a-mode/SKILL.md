---
name: ecluse-choosing-a-mode
description: >
  Use this skill when the user asks which ecluse mode to use, runs
  `ecluse init` and needs to confirm or override the recommended mode,
  says "what mode should I pick", or wants to understand the difference
  between container, host, and hybrid. Also use when detection confidence
  is low and the user needs to understand why.
tags:
  - ecluse
  - modes
  - init
  - configuration
---

# Choosing a Mode

## The three modes

| Mode | What runs in containers | What runs on host | Best for |
|---|---|---|---|
| `container` | Everything — app and data | Nothing | Fully containerized stacks, devcontainer repos |
| `host` | Nothing | Everything | Pure native stacks (`npm run dev`, `bin/rails server`) |
| `hybrid` | Data services only (postgres, redis, etc.) | App code | Rails/Django/Node apps with a compose data plane |

## Decision guide

**Use `container` when:**
- Your repo has a `docker-compose.yml` with `build: .` (app built from the repo)
- Your team's primary workflow is `docker compose up`
- You want the strongest isolation — every service in a container

**Use `host` when:**
- No compose file — dev command is `npm run dev`, `bin/rails server`, etc.
- You use `mise`, `asdf`, `rbenv`, `nvm`, or similar version managers
- Docker is absent or you prefer not to use it
- Fast native iteration matters more than isolation depth

**Use `hybrid` when:**
- Your compose file defines only data services (postgres, redis, etc.) — no app service with `build: .`
- Your README says "run `docker compose up`, then `bin/dev`"
- You want database isolation per session but native app speed
- Hot reload and native debuggers matter

## Detection

`ecluse init` auto-detects the recommended mode using 20 signals. The full signal table is in [references/signals.md](references/signals.md).

```bash
ecluse init             # auto-detect, prompt to confirm
ecluse init --explain   # show full score breakdown
ecluse init --mode hybrid  # skip detection, use this mode
```

Confidence levels:

| Gap (winner − runner-up) | Behaviour |
|---|---|
| ≥ 4 | High — one-line recommendation, `<enter>` accepts |
| 2–3 | Medium — one-line recommendation + key signals printed |
| 0–1 | Low — full breakdown printed automatically |
| All ≤ 0 | None — `--mode` required |

## Recovering from a wrong choice

Re-run `ecluse init --mode <correct>`. The config file is overwritten. Existing sessions keep their original mode (stored in `state.json`); new sessions use the new mode.

## Common failures

- **Unsupported (Nix flake)** — use `nix develop` for per-shell isolation.
- **Unsupported (Bazel)** — use Bazel's native sandbox.
- **Wrong recommendation** — run `ecluse init --explain` to see which signals fired, then override with `--mode`.

## See also

- [container-mode](../container-mode/SKILL.md)
- [host-mode](../host-mode/SKILL.md)
- [hybrid-mode](../hybrid-mode/SKILL.md)
- [Signal table](references/signals.md)
