#!/usr/bin/env bash
#
# Build AgentToast and put it in /Applications — the whole install, one command.
#
#   ./macos/scripts/install.sh              # build, install, clean up
#   ./macos/scripts/install.sh --with-dmg   # also produce a disk image to hand out
#
# ## Why this exists rather than "run build.sh and drag it across"
#
# `build.sh` leaves an `AgentToast.app` in `target/release/bundle/macos/`, and
# `cargo tauri build` mounts a disk image containing another one. Launch
# Services indexes every bundle it ever sees, so after a few builds Spotlight,
# Raycast and the Open dialog all offer several identical "AgentToast" entries —
# most pointing at a staging copy or at a `/Volumes/dmg.XXXXXX` that was
# unmounted long ago. Picking the wrong one runs a stale build, or nothing.
#
# So this builds the `.app` on its own — no `dmg` target unless asked for. That
# matters for more than tidiness: producing a disk image means *mounting* it,
# which throws a Finder window on screen in the middle of the build and adds yet
# another `/Volumes/dmg.XXXXXX/AgentToast.app` for Launch Services to remember.
#
# Then it deletes the staging copy and takes every path that is not
# `/Applications/AgentToast.app` back out of the Launch Services database, so a
# machine that has been building for a while ends up clean too.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"

target="/Applications/AgentToast.app"
staging="target/release/bundle/macos/AgentToast.app"
lsregister="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"

with_dmg=false
args=()
for arg in "$@"; do
  case "$arg" in
    --with-dmg) with_dmg=true ;;
    *) args+=("$arg") ;;
  esac
done

# --- build ------------------------------------------------------------------

# `--bundles app` overrides the `["app", "dmg"]` in the macOS config. Without it
# the build mounts a disk image, which opens a Finder window over whatever you
# were doing and leaves a `/Volumes` entry behind.
$with_dmg || args+=(--bundles app)

./macos/scripts/build.sh ${args+"${args[@]}"}

[ -d "$staging" ] || {
  echo "Expected a bundle at $staging and there isn't one." >&2
  exit 1
}

# --- replace what is there --------------------------------------------------

# Quitting first matters: copying over a running bundle leaves the old code
# mapped and the new files on disk, which fails in ways that look like anything
# but the real cause.
if pkill -f "$target/Contents/MacOS/agenttoast" 2>/dev/null; then
  echo "Stopped the running AgentToast."
  sleep 2
fi

rm -rf "$target"
cp -R "$staging" "$target"

# A locally built app is unsigned. It has no quarantine flag unless it travelled
# somewhere, but clearing it costs nothing and saves a Gatekeeper refusal.
xattr -dr com.apple.quarantine "$target" 2>/dev/null || true

echo "Installed $target"

# --- leave exactly one bundle behind ----------------------------------------

rm -rf "$staging"
$with_dmg || rm -rf target/release/bundle/dmg

if [ -x "$lsregister" ]; then
  "$lsregister" -dump 2>/dev/null \
    | grep -oE "^path: +\S.*AgentToast\.app" \
    | sed 's/^path: *//' \
    | sort -u \
    | while read -r path; do
        [ "$path" = "$target" ] && continue
        "$lsregister" -u "$path" 2>/dev/null || true
        echo "  unregistered stale bundle: $path"
      done
fi

# Register the one that is left, explicitly. Unregistering its neighbours
# leaves the database mid-edit, and an `open` issued into that window can fail
# without saying anything — which is exactly how an install finishes looking
# successful with nothing running.
if [ -x "$lsregister" ]; then
  "$lsregister" -f "$target" 2>/dev/null || true
fi

remaining=$("$lsregister" -dump 2>/dev/null | grep -c "path:.*AgentToast\.app" || true)
echo "Launch Services now knows about $remaining AgentToast bundle(s)."

# --- start it ---------------------------------------------------------------

open "$target"

# `open` returns as soon as it has handed the request over, so success there is
# not evidence the app came up. Wait for the process rather than assume it.
started=false
for _ in $(seq 1 20); do
  if pgrep -f "$target/Contents/MacOS/agenttoast" >/dev/null 2>&1; then
    started=true
    break
  fi
  sleep 0.5
done

echo
if $started; then
  echo "AgentToast is running — look for it in the Dock, and in the menu bar."
  echo "Open the dashboard from either, then press Connect for each agent."
else
  echo "Installed, but it did not start. Try:" >&2
  echo "  open \"$target\"" >&2
  exit 1
fi
