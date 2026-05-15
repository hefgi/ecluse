# Contributing

Issues and PRs are welcome. Check the [open issues](https://github.com/hefgi/ecluse/issues) for ideas — good first issues are tagged. If you're adding a new isolation mode or provider, open an issue first to discuss the approach.

## Prerequisites

- Rust 1.85+ (`rustup` or your preferred toolchain manager)
- For container and hybrid mode testing: [OrbStack](https://orbstack.dev) (macOS) or Docker Engine (Linux)

## Dev workflow

```bash
cargo fmt --check && cargo clippy -- -D warnings && cargo test   # run before pushing
cargo build
cargo test
cargo clippy -- -D warnings
cargo fmt
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
├── main.rs       command handlers (init/up/down/ls/shell/env/validate/shutdown)
├── cli.rs        clap CLI definitions
├── config.rs     .ecluse.toml parsing, Config struct, Mode enum
├── slot.rs       slot allocation (first free in 1..max_slots)
├── env.rs        .env.ecluse generation from slot + config
├── state.rs      state.json persistence with file locking (StateGuard)
├── error.rs      EcluseError variants with actionable messages
├── detect.rs     mode auto-detection via signal scoring
├── worktree.rs   git worktree create/remove wrappers
├── hooks.rs      pre_up/post_up/pre_down/post_down lifecycle hook execution
├── compose.rs    docker-compose.yml parsing + overlay generation
├── docker.rs     Docker CLI wrappers
└── modes/        ModeHandler trait + container/host/hybrid impls
```

## Adding a new isolation mode

1. Open an issue first to discuss the approach.
2. Add a new variant to `config::Mode` and `error::EcluseError` as needed.
3. Implement `ModeHandler` in a new file under `src/modes/`.
4. Register it in `modes::get_handler`.
5. Add detection signals in `detect.rs` if the mode can be auto-detected.
6. Add unit tests alongside the new code.

## Pull requests

- Keep commits focused — one logical change per commit.
- Run `cargo fmt --check && cargo clippy -- -D warnings && cargo test` before pushing; CI enforces all three.
- For bug fixes: a failing test that passes after the fix is ideal.
- For new features: unit tests are required, integration tests are a bonus.
