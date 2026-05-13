#!/usr/bin/env bash
# Generates llms.txt (index) and llms-full.txt (concatenated content)
# into docs/book/ after mdbook build has run.
set -euo pipefail

BOOK_DIR="$(dirname "$0")/book"
SRC_DIR="$(dirname "$0")/src"

# llms.txt — structured index following the llmstxt.org spec
cat > "$BOOK_DIR/llms.txt" <<'EOF'
# ecluse

> Per-worktree isolation for parallel coding agents. Each git worktree gets its own slot — isolated ports, isolated services, nothing shared.

## Getting started

- [Introduction](https://docs.ecluse.dev/introduction.html): What ecluse is and how it works
- [Install](https://docs.ecluse.dev/install.html): Homebrew, cargo, agent skill
- [Quick start](https://docs.ecluse.dev/quickstart.html): First session in 4 commands
- [Choosing a mode](https://docs.ecluse.dev/modes.html): container vs host vs hybrid

## Reference

- [Commands](https://docs.ecluse.dev/commands.html): All CLI flags and options
- [Configuration](https://docs.ecluse.dev/configuration.html): .ecluse.toml schema
- [Environment variables](https://docs.ecluse.dev/env-vars.html): ECLUSE_* vars written to .env.ecluse
- [Port allocation](https://docs.ecluse.dev/ports.html): Formula, collision handling, validation

## Guides

- [Agent workflow](https://docs.ecluse.dev/agent-workflow.html): Canonical loop for coding agents
- [Hybrid mode setup](https://docs.ecluse.dev/hybrid-setup.html): Label your compose file
- [Hooks](https://docs.ecluse.dev/hooks.html): on_up / on_down lifecycle commands

## Development

- [Contributing](https://docs.ecluse.dev/contributing.html): Dev workflow, project structure, PRs
- [Known limits](https://docs.ecluse.dev/limits.html): Intentional v0 limitations
EOF

# llms-full.txt — full content of every doc page concatenated
OUTPUT="$BOOK_DIR/llms-full.txt"
> "$OUTPUT"

# Order matches SUMMARY.md
pages=(
  introduction
  install
  quickstart
  modes
  commands
  configuration
  env-vars
  ports
  agent-workflow
  hybrid-setup
  hooks
  contributing
  limits
)

for page in "${pages[@]}"; do
  file="$SRC_DIR/${page}.md"
  if [[ -f "$file" ]]; then
    echo "---" >> "$OUTPUT"
    echo "" >> "$OUTPUT"
    cat "$file" >> "$OUTPUT"
    echo "" >> "$OUTPUT"
  fi
done

echo "llms.txt and llms-full.txt written to $BOOK_DIR"
