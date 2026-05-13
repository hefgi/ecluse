# Limits

## What ecluse does not do

This is an honest list of things ecluse intentionally omits in v0. If you need these, look at the alternatives listed.

### No auto-detection of mode after `init`

Mode is set once at `init` and persisted in `.ecluse.toml`. If your repo shape changes (you added a compose file, or removed Docker), re-run `ecluse init` to recalibrate. The mode is never auto-detected during `up` or `down`.

### No monorepo support with multiple compose files

ecluse reads one compose file per repo. If your monorepo has multiple compose files in different subdirectories, run `ecluse init` in each subdirectory separately.

### No Cloudflare tunnels or public URLs

ecluse allocates `localhost:<port>` — not public hostnames. If you need `*.feature-foo.test` or public preview URLs, use a separate tunneling tool (cloudflared, ngrok, etc.).

### No agent auto-detection or launching

ecluse does not detect which coding agent is running, launch an agent, or integrate with tmux, Claude Code, Cursor, or Codex harnesses directly. It creates the environment; you (or your harness) start the agent.

### No agent sandboxing

ecluse does not sandbox the agent process itself. Container mode puts services in containers, but the agent code still runs on the host. For agent sandboxing, use a dedicated VM or container.

### No Windows native support

WSL2 is acceptable (untested). Native Windows (cmd.exe, PowerShell) is not supported.

### No daemon or control plane

ecluse has no background daemon. Every command is a single-shot process that reads and writes state files. There is no port to connect to, no socket to authenticate against.

### Postgres only for host-mode database provisioning

`host` and `hybrid` modes with `database.provider = "postgres-host"` support Postgres only. MySQL, MongoDB, SQLite-per-worktree, etc. are not supported in v0.

### No telemetry

ecluse collects no data. No analytics, no crash reports, no opt-in/opt-out.

### No devcontainer pre-built image management

ecluse does not manage devcontainer images. For that workflow, see [branchbox](https://github.com/branchbox/branchbox).

### No plugin system

There are no hooks, plugins, or extensibility surfaces beyond mode selection. If you need custom behavior per-mode, fork and patch.

## See also

- `ecluse skills show troubleshooting`
- `ecluse skills show choosing-a-mode`
