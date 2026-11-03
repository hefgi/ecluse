#!/usr/bin/env bash
# Run on your laptop. Forwards every port of an ecluse session running in a
# Daytona sandbox to the same port on localhost, so http://localhost:<port>
# URLs work in your browser exactly as they do inside the sandbox.
#
#   ./forward-ports.sh <ssh-destination> <repo-path-in-sandbox> <slug>
#
# <ssh-destination> is whatever `ssh` accepts for your sandbox — e.g. the
# host alias Daytona adds to ~/.ssh/config, or <token>@ssh.app.daytona.io.
set -euo pipefail

if [ $# -ne 3 ]; then
  echo "usage: $0 <ssh-destination> <repo-path-in-sandbox> <slug>" >&2
  exit 1
fi
dest="$1" repo="$2" slug="$3"

# shellcheck disable=SC2029 # $repo/$slug are meant to expand locally
ports=$(ssh "$dest" "cd '$repo' && ecluse env '$slug'" 2>/dev/null \
  | jq -r '.env | to_entries[] | select(.key | test("^ECLUSE_.*_PORT$")) | .value' \
  | sort -un)

if [ -z "$ports" ]; then
  echo "No ports found for session '$slug'; is it up? (ssh $dest 'cd $repo && ecluse ls')" >&2
  exit 1
fi

args=()
for p in $ports; do
  args+=(-L "$p:localhost:$p")
  echo "localhost:$p -> sandbox:$p"
done

echo "Forwarding; Ctrl-C to stop."
exec ssh -N "${args[@]}" "$dest"
