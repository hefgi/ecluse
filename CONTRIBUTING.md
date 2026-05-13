# Contributing to ecluse

## Prerequisites

- Rust 1.85+ (`rustup` or your preferred toolchain manager)
- For container and hybrid mode testing: [OrbStack](https://orbstack.dev) (macOS) or Docker Engine (Linux)
- [Task](https://taskfile.dev) for the dev workflow: `brew install go-task`

## Dev workflow

```bash
task          # fmt + lint + test (run this before pushing)
task build    # cargo build
task test     # cargo test
task lint     # cargo clippy -- -D warnings
task fmt:fix  # auto-format
```

Run a specific test module:

```bash
cargo test slot::tests
cargo test config::tests
cargo test env::tests
cargo test detect::tests
```

## Project structure

```
src/
├── main.rs          entry point, command handlers (init/up/down/ls)
├── cli.rs           clap CLI definitions
├── config.rs        .ecluse.toml parsing and Config struct
├── slot.rs          slot allocation logic
├── env.rs           .env.ecluse generation
├── detect.rs        mode auto-detection heuristics
├── state.rs         session state with file locking
├── error.rs         typed EcluseError variants
├── modes/           ModeHandler trait + container/host/hybrid impls
├── compose.rs       docker-compose file parsing and interaction
├── docker.rs        Docker CLI wrapper
├── postgres.rs      host Postgres provisioning
└── worktree.rs      git worktree management
```

## Adding a new isolation mode

1. Open an issue first to discuss the approach.
2. Add a new variant to `config::Mode` and `error::EcluseError` as needed.
3. Implement `ModeHandler` in a new file under `src/modes/`.
4. Register it in `modes::get_handler`.
5. Add detection signals in `detect.rs` if the mode can be auto-detected.
6. Add unit tests alongside the new code.

## Adding a new database provider

Open an issue first. The current provider interface lives in `postgres.rs` — a trait abstraction will be introduced when a second provider is added.

## Pull requests

- Keep commits focused — one logical change per commit.
- Run `task` before pushing; CI enforces fmt, clippy, and tests.
- For bug fixes: a failing test that passes after the fix is ideal.
- For new features: unit tests are required, integration tests are a bonus.
