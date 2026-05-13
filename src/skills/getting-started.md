# Getting Started with ecluse

## When to use

You want to run multiple isolated development environments for the same repo — e.g., parallel coding agent sessions, feature branches, or A/B experiments — without port collisions or shared database state.

## Prerequisites

- macOS 14+ or Linux
- Git repository
- Rust (for `cargo install`) or Homebrew

For `container` or `hybrid` mode: Docker installed and running.
For `host` mode: nothing extra (optionally a host Postgres for database isolation).

## Install

```bash
# Via Homebrew (recommended)
brew install ecluse/tap/ecluse

# Via cargo
cargo install --git https://github.com/ecluse/ecluse
```

## Five-minute on-ramp

```bash
cd my-project

# 1. Initialize ecluse (detects recommended mode, prompts for confirmation)
ecluse init

# 2. Create a session for a new feature
ecluse up feat-foo

# 3. The session output tells you the worktree path and connection details
# e.g.:
#   Worktree:  .ecluse/worktrees/feat-foo
#   Mode:      hybrid
#   Slot:      1
#   App port:  3100
#   Database:  myapp_feat_foo
#   Next step: cd .ecluse/worktrees/feat-foo && source .env.ecluse && npm run dev

# 4. Work in the worktree; source .env.ecluse for connection strings
cd .ecluse/worktrees/feat-foo
source .env.ecluse
npm run dev  # or bin/dev, python manage.py runserver, etc.

# 5. List active sessions
ecluse ls

# 6. Tear down when done
ecluse down feat-foo
```

## Common failures

- **"run `ecluse init` first"**: no `.ecluse.toml` found in this repo or any parent directory.
- **"all N slots are in use"**: run `ecluse ls` to see active sessions and `ecluse down <slug>` to free one.
- **docker not available**: for container/hybrid mode, ensure Docker Desktop or OrbStack is running.

## See also

- `ecluse skills show choosing-a-mode` — pick the right mode for your stack
- `ecluse skills show agent-workflow` — canonical loop for coding agents
