#!/usr/bin/env bash
#
# Generate macOS icons for AgentToast.
#
# Produces `macos/icons/icon.icns`, which is what the bundle uses, plus the
# individual PNGs it is built from. The result is committed, so this only needs
# running when the mark itself changes.
#
# An .icns is an .iconset directory — one PNG per size, named by convention —
# handed to `iconutil`. Every size is drawn from scratch rather than downscaled
# from one large render: the smallest ones are redrawn with a single bolder bar,
# because two bars at 16px are three grey pixels.
#
#   ./macos/scripts/generate-icons.sh
#
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
icons="$root/macos/icons"
iconset="$icons/AgentToast.iconset"

command -v iconutil >/dev/null || {
  echo "iconutil not found. It ships with Xcode's command line tools:" >&2
  echo "  xcode-select --install" >&2
  exit 1
}

rm -rf "$iconset"
mkdir -p "$iconset"

# The names are fixed by iconutil, not by us. Each logical size appears twice —
# once at 1x and once at 2x — and the 2x file is simply the next size up.
render() { python3 "$here/render-icon.py" "$1" "$iconset/$2"; }

render 16   "icon_16x16.png"
render 32   "icon_16x16@2x.png"
render 32   "icon_32x32.png"
render 64   "icon_32x32@2x.png"
render 128  "icon_128x128.png"
render 256  "icon_128x128@2x.png"
render 256  "icon_256x256.png"
render 512  "icon_256x256@2x.png"
render 512  "icon_512x512.png"
render 1024 "icon_512x512@2x.png"

iconutil --convert icns "$iconset" --output "$icons/icon.icns"
rm -rf "$iconset"

echo
echo "wrote $icons/icon.icns"
