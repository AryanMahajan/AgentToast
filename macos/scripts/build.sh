#!/usr/bin/env bash
#
# Build AgentToast.app and the disk image that carries it.
#
#   ./macos/scripts/build.sh                                  # for this Mac
#   ./macos/scripts/build.sh --target universal-apple-darwin  # for any Mac
#   ./macos/scripts/build.sh --debug                          # faster, unoptimised
#
# Anything you pass is handed on to `cargo tauri build`.
#
# ## Why this is a script and not one command
#
# Two things have to be true of a macOS build, and neither is true of a bare
# `cargo tauri build`.
#
# **The config.** `macos/tauri.macos.conf.json` carries the `.icns`, the `app`
# and `dmg` bundle targets, and the two bridge binaries as bundle resources
# under the names they actually have here rather than the `.exe` ones the
# Windows bundle config uses. Build without it and you get an app with no icon
# and no bridges — which installs cleanly and then cannot connect to anything.
#
# (`macOSPrivateApi`, which is what lets a toast window be transparent, is *not*
# here. It has to match a Cargo feature that `tauri-build` checks on every
# platform, so it lives in `src-tauri/tauri.conf.json` where a plain
# `cargo build` sees it too.)
#
# **The bridges' architecture.** Bundle resource paths are fixed strings, so the
# app always takes its bridges from `target/release/`. Left to itself that is
# whatever architecture this machine happens to be, which is fine until the app
# is built universal — and then an Intel Mac gets an app it can run and two
# bridges it cannot, which shows up as an agent that hangs on every hook rather
# than as anything resembling an error. So the bridges are built here, for the
# same target as the app, and put where the bundler will look. The mac config
# drops them from `beforeBuildCommand` for the same reason: it would rebuild
# them for the host and undo this.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"

cargo tauri --version >/dev/null 2>&1 || {
  echo "The Tauri CLI is not installed. Either:" >&2
  echo "  cargo install tauri-cli --version '^2' --locked" >&2
  echo "  cargo binstall tauri-cli --version '^2' --locked   # prebuilt, much faster" >&2
  exit 1
}

[ -d node_modules ] || {
  echo "Installing front-end packages…"
  npm install
}

[ -f macos/icons/icon.icns ] || {
  echo "Generating icons…"
  ./macos/scripts/generate-icons.sh
}

# --- which target, if any ---------------------------------------------------

target=""
prev=""
for arg in "$@"; do
  case "$prev" in
    --target|-t) target="$arg" ;;
  esac
  case "$arg" in
    --target=*) target="${arg#--target=}" ;;
  esac
  prev="$arg"
done

BRIDGES=(agenttoast-bridge-claude agenttoast-bridge-agy)

echo "Building the bridges…"
case "$target" in
  universal-apple-darwin)
    # `cargo` has no universal target; `lipo` glues the two real ones together,
    # which is exactly what Tauri does for the app binary itself.
    for arch in aarch64-apple-darwin x86_64-apple-darwin; do
      rustup target add "$arch" >/dev/null 2>&1 || true
      cargo build --release --target "$arch" \
        -p agenttoast-bridge-claude -p agenttoast-bridge-agy
    done
    for bridge in "${BRIDGES[@]}"; do
      lipo -create \
        "target/aarch64-apple-darwin/release/$bridge" \
        "target/x86_64-apple-darwin/release/$bridge" \
        -output "target/release/$bridge"
      echo "  $bridge: $(lipo -archs "target/release/$bridge")"
    done
    ;;
  "")
    cargo build --release -p agenttoast-bridge-claude -p agenttoast-bridge-agy
    ;;
  *)
    rustup target add "$target" >/dev/null 2>&1 || true
    cargo build --release --target "$target" \
      -p agenttoast-bridge-claude -p agenttoast-bridge-agy
    mkdir -p target/release
    for bridge in "${BRIDGES[@]}"; do
      cp "target/$target/release/$bridge" "target/release/$bridge"
    done
    ;;
esac

# --- the app ----------------------------------------------------------------

cargo tauri build --config macos/tauri.macos.conf.json "$@"

out="target/release/bundle"
[ -n "$target" ] && out="target/$target/release/bundle"

echo
echo "Bundles are in $out/"
ls -1 "$out"/dmg/*.dmg 2>/dev/null || true
ls -1d "$out"/macos/*.app 2>/dev/null || true

cat <<'NOTE'

The build is not code-signed or notarised. Opening it on the machine that built
it is fine. A copy that has been downloaded carries a quarantine flag, and
Gatekeeper will refuse it outright — the way past that is right-click → Open, or

  xattr -dr com.apple.quarantine /Applications/AgentToast.app
NOTE
