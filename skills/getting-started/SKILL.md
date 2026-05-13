---
name: ecluse-getting-started
description: >
  Use this skill when the user asks how to install ecluse, how to get started,
  or says "I want to use ecluse on my project" for the first time. Covers
  install, init, the three commands, and the five-minute on-ramp.
tags:
  - ecluse
  - onboarding
  - install
---

# Getting Started with ecluse

## Prerequisites

- macOS 14+ or Linux
- Git repository
- Homebrew (recommended) or Rust toolchain for `cargo install`

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

# 1. Initialize — detects recommended mode, prompts for confirmation
ecluse init

# 2. Create a session for a new feature
ecluse up feat-foo

# 3. The output tells you the worktree path and connection details:
#   Session:   feat-foo (slot 1)
#   Worktree:  .ecluse/worktrees/feat-foo
#   Mode:      hybrid
#   App port:  3100
#   Database:  myapp_feat_foo
#   Next step: cd .ecluse/worktrees/feat-foo && source .env.ecluse && npm run dev

# 4. Work in the worktree; source .env.ecluse for connection strings
cd .ecluse/worktrees/feat-foo
source .env.ecluse
npm run dev   # or bin/dev, python manage.py runserver, etc.

# 5. List active sessions
ecluse ls

# 6. Tear down when done
ecluse down feat-foo
```

## Common failures

- **"run `ecluse init` first"** — no `.ecluse.toml` found in this repo or any parent directory.
- **"all N slots are in use"** — run `ecluse ls` to see active sessions and `ecluse down <slug>` to free one.
- **Docker not available** — for container/hybrid mode, ensure Docker Desktop or OrbStack is running.

## See also

- [choosing-a-mode](../choosing-a-mode/SKILL.md) — pick the right mode for your stack
- [agent-workflow](../agent-workflow/SKILL.md) — canonical loop for coding agents
