#!/usr/bin/env bash
#
# Run AgentToast from source, with the front end hot-reloading.
#
#   ./macos/scripts/dev.sh
#
# The `--config` carries the mac bundle settings. A dev run needs almost none of
# them, but passing the same file the release build uses keeps the two from
# drifting — and it costs nothing.
#
# The bridges are *not* bundled in a dev run, and do not need to be: they are
# built into `target/debug` beside the app, and `install::bridge_path` looks
# there when there is no bundled copy. So Connect works from a dev build, and
# points the agents at the binaries in the target directory.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"

[ -d node_modules ] || npm install

# The dev app finds these beside itself in target/debug; without them the
# dashboard shows both connectors as "bridge missing".
cargo build -p agenttoast-bridge-claude -p agenttoast-bridge-agy

cargo tauri dev --config macos/tauri.macos.conf.json "$@"
