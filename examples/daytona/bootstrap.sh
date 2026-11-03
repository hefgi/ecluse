#!/usr/bin/env bash
# Prepare a Daytona sandbox for `ecluse up`. Safe to run more than once.
#
#   ecluse-bootstrap                 # SSH port-forwarding mode (default)
#   ecluse-bootstrap <sandbox-id>    # Daytona preview-URL mode
set -euo pipefail

sandbox_id="${1:-}"

# 1. Start the Docker daemon (hybrid mode runs Postgres/Redis in containers).
if ! docker info >/dev/null 2>&1; then
  echo "Starting dockerd..."
  (dockerd >/tmp/dockerd.log 2>&1 &)
  for _ in $(seq 1 30); do
    docker info >/dev/null 2>&1 && break
    sleep 1
  done
  docker info >/dev/null 2>&1 || {
    echo "dockerd did not start; see /tmp/dockerd.log (the sandbox may lack the privileges Docker-in-Docker needs)" >&2
    exit 1
  }
fi

# 2. Check every binary ecluse and the hooks shell out to.
missing=()
for bin in ecluse git tmux lsof jq pg_isready npm; do
  command -v "$bin" >/dev/null 2>&1 || missing+=("$bin")
done
if [ ${#missing[@]} -gt 0 ]; then
  echo "Missing binaries: ${missing[*]}; rebuild the snapshot from examples/daytona/Dockerfile" >&2
  exit 1
fi

# 3. Tell the pre_spawn hook which URL the browser should use for the API.
if [ -n "$sandbox_id" ]; then
  template="https://%s-${sandbox_id}.proxy.daytona.works"
else
  template="http://localhost:%s"
fi
sed -i '/^export PUBLIC_URL_TEMPLATE=/d' ~/.bashrc 2>/dev/null || true
echo "export PUBLIC_URL_TEMPLATE=\"$template\"" >> ~/.bashrc
echo "PUBLIC_URL_TEMPLATE=$template (open a new shell or 'source ~/.bashrc')"

# 4. Untracked files ecluse needs from the main checkout.
if root="$(git rev-parse --show-toplevel 2>/dev/null)"; then
  for f in .ecluse.toml .env; do
    [ -e "$root/$f" ] || echo "warning: $root/$f is missing; copy it from your laptop before 'ecluse up'" >&2
  done
fi

echo "Sandbox ready. Next: cd <repo> && ecluse up <slug>"
