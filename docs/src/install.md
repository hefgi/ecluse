# Install

## Homebrew (recommended)

```bash
brew install hefgi/tap/ecluse
```

## Cargo

```bash
cargo install ecluse
```

Requires Rust 1.85+.

## Agent skill

Install the agent skill so your coding agent knows every command, mode, and workflow:

```bash
npx skills add hefgi/ecluse -y
```

| | Command |
|---|---|
| Global | `npx skills add hefgi/ecluse -y` |
| Project-local | `npx skills add hefgi/ecluse -y --out .` |

## Dependencies

For container and hybrid modes, [OrbStack](https://orbstack.dev) is recommended over Docker Desktop on macOS — faster, less memory. Docker Engine works on Linux.
