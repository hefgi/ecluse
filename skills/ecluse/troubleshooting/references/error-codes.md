# Error Code Reference

All error variants ecluse can produce, with their causes and remediation steps.

## Common errors

| Error | Cause | Fix |
|---|---|---|
| `SlugInvalid` | Slug doesn't match `^[a-z0-9][a-z0-9-]{0,30}[a-z0-9]$` | Use only lowercase letters, numbers, hyphens; 2–32 chars |
| `SlotsExhausted` | All `max_slots` are in use | `ecluse ls` then `ecluse down <slug>` |
| `SessionExists` | Slug already active | Pick a different slug or down the existing one |
| `SessionNotFound` | Slug not in state.json | Check `ecluse ls`; slug may have been already downed |
| `LockTimeout` | Another process holds state.lock for >10s | Check for other ecluse processes; remove stale lock |
| `StateCorrupt` | state.json is malformed JSON | Edit or delete `.ecluse/state.json` manually |
| `ConfigMissing` | No `.ecluse.toml` found in directory tree | Run `ecluse init` |
| `NotAGitRepo` | Not inside a git repository | `git init` first |

## Container mode errors

| Error | Cause | Fix |
|---|---|---|
| `ComposeFileNotFound` | No `docker-compose.yml` or `compose.yaml` at repo root | Add a compose file or switch to host mode |
| `ComposeParseFailed` | Compose file has invalid YAML | Fix YAML syntax errors |
| `DockerFailed` | `docker compose` exited non-zero | Check docker daemon; see stderr in output |

## Host mode errors

| Error | Cause | Fix |
|---|---|---|
| `PortInUse { port, pid }` | Port already bound by another process | `kill <pid>` then retry |
| `PostgresUnreachable` | Can't TCP-connect to configured postgres host:port | Start postgres; check `[database]` config |
| `DatabaseCreateFailed` | `CREATE DATABASE` returned an error | Check postgres permissions; see stderr |

## Hybrid mode

| Error | Cause | Fix |
|---|---|---|
| `AppLabelMissing` (warning) | No service labeled `ecluse.role: app` | Add the label; or ignore — all services treated as data |

Hybrid mode otherwise inherits container and host mode errors depending on which phase failed.
