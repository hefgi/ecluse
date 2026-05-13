# Choosing a Mode

## When to use

You are running `ecluse init` and need to decide which mode fits your project, or you want to override the detected mode.

## The three modes

| Mode | What runs in containers | What runs on host | Best for |
|---|---|---|---|
| `container` | Everything (app + data) | Nothing | Full-stack containerized apps, devcontainer-style repos |
| `host` | Nothing | Everything | Pure native dev stacks with no compose, fast feedback loops |
| `hybrid` | Data services only (postgres, redis, etc.) | App code | Most Rails/Django/Node apps — data isolated, app runs natively |

## Decision guide

**Use `container` when:**
- Your repo has a `docker-compose.yml` with a service that has `build: .` (app built from the repo)
- Your team already does `docker compose up` as the primary dev workflow
- You want the strongest isolation (every service, including the app, is containerized)

**Use `host` when:**
- No compose file — your dev command is `npm run dev`, `bin/rails server`, etc.
- You use `mise`, `asdf`, `rbenv`, `nvm`, or similar version managers
- You have no Docker or prefer not to use it
- Fast iteration is the priority and you trust your local tooling

**Use `hybrid` when:**
- You have a compose file but it only defines data services (postgres, redis, etc.)
- Your app runs better natively (hot reload, native debugger, etc.)
- Your README says something like "run `docker compose up`, then `bin/dev`"
- You want database isolation (each session gets its own DB) but app-native speed

## Signal table (used by `ecluse init` auto-detection)

| Signal | Detection method | container | host | hybrid |
|---|---|---:|---:|---:|
| `docker-compose.yml` or `compose.yaml` at repo root | File stat | +2 | 0 | +2 |
| Compose has a service with `build: .` | Compose parse | +3 | 0 | 0 |
| All compose services match known data images | Compose parse + image-name match | −2 | 0 | +5 |
| Any compose service has label `ecluse.role: app` | Compose parse | 0 | 0 | +10 |
| Compose has `watch:` blocks | Compose parse | +2 | 0 | +1 |
| Compose has bind mounts of source | Compose parse | +2 | 0 | 0 |
| `.devcontainer/devcontainer.json` exists | File stat | +4 | 0 | 0 |
| No compose file | File absence | −5 | +4 | −5 |
| `bin/dev` exists | File stat | 0 | +3 | +2 |
| `Procfile.dev` exists | File stat | 0 | +3 | +2 |
| `package.json` with non-docker `dev` script | Parse | 0 | +2 | +2 |
| `Gemfile` + `bin/rails` present | File stat | 0 | +2 | +2 |
| Host Postgres reachable on `localhost:5432` | TCP probe (500ms) | 0 | +1 | 0 |
| Version manager file present | File stat | 0 | +1 | +1 |
| README: `docker compose up` + `bin/dev` within 10 lines | Regex | 0 | 0 | +3 |
| Docker not installed | which | −10 | 0 | −10 |
| `flake.nix` exists | File stat | **Unsupported** | | |
| Bazel files exist | File stat | **Unsupported** | | |

## Overriding detection

```bash
ecluse init --mode hybrid   # skip detection, use this mode
ecluse init --explain       # show full score breakdown before prompting
```

If you accept the wrong mode, re-run `ecluse init --mode <correct>`. Existing sessions keep their original mode; new sessions use the new one.

## Common failures

- **Unsupported (Nix flake)**: use your flake's `devShell` — `nix develop` gives you per-shell isolation.
- **Unsupported (Bazel)**: use Bazel's native sandbox features.
- **Detection recommends wrong mode**: run `ecluse init --explain` to see which signals fired, then override with `--mode`.

## See also

- `ecluse skills show container-mode`
- `ecluse skills show host-mode`
- `ecluse skills show hybrid-mode`
