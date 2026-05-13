---
name: ecluse-limits
description: >
  Use this skill when the user asks whether ecluse supports something it
  doesn't, wants to understand what's out of scope, or is deciding whether
  ecluse is the right tool for their use case.
tags:
  - ecluse
  - limits
  - scope
---

# Limits

What ecluse intentionally does not do in v0. These are not bugs — they are deliberate scope decisions.

## No auto-detection of mode after `init`

Mode is set once at `init` and stored in `.ecluse.toml`. To change it: `ecluse init --mode <new>`. Running `up` never re-detects mode.

## No monorepo with multiple compose files

ecluse reads one compose file per repo root. For monorepos with multiple compose files, run `ecluse init` separately in each subdirectory.

## No public URLs or tunnels

ecluse allocates `localhost:<port>` only. For public preview URLs, use cloudflared or ngrok separately.

## No agent sandboxing

Container mode puts services in Docker, but the agent process itself runs on the host. ecluse does not sandbox the agent.

## No process launching

ecluse does not start your dev server, launch an agent, or open a tmux session. It sets up the environment and writes `.env.ecluse`. You (or your harness) start the process.

## No Windows native support

macOS and Linux only. WSL2 is acceptable but untested. Native Windows (cmd.exe, PowerShell) is not supported.

## Postgres only for database provisioning

`host` and `hybrid` modes support only `database.provider = "postgres-host"`. MySQL, MongoDB, and SQLite are not supported in v0.

## No daemon or background process

Every ecluse command is a single-shot process. There is no background daemon, no socket, no control plane.

## No telemetry

ecluse collects no data. No analytics, no crash reports, no network calls except the optional host Postgres TCP probe during `init`.

## No plugin system

No hooks, no extensions, no per-mode escape hatches beyond mode selection.

## See also

- [troubleshooting](../troubleshooting/SKILL.md)
- [choosing-a-mode](../choosing-a-mode/SKILL.md)
