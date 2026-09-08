#!/usr/bin/env bash
#
# Remove AgentToast from this Mac — the app, its data, and every hook it wrote.
#
#   ./macos/scripts/uninstall.sh              # remove everything
#   ./macos/scripts/uninstall.sh --keep-data  # keep ~/.agenttoast (paired phones)
#   ./macos/scripts/uninstall.sh --dry-run    # say what it would do, change nothing
#
# ## Why deleting the app is not enough
#
# AgentToast works by writing itself into other programs' configuration. Drag it
# to the Trash and those entries stay behind, now pointing at a binary that no
# longer exists:
#
#   ~/.claude/settings.json            hooks on five events
#   <project>/.claude/settings.json    the same, per project it was connected to
#   ~/.gemini/config/hooks.json        an `agenttoast` block
#   ~/.gemini/antigravity-cli/…        `command(*)` and `write_file(*)` grants
#
# The last one is the one that matters. Those grants are a standing instruction
# to Antigravity to stop asking before it runs a command or writes a file. They
# are safe only while AgentToast's hook is there to answer for them — with the
# hook gone they are a blanket approval with nothing behind it, and nothing in
# Antigravity will ever mention them again.
#
# Everything here is scoped to what AgentToast wrote. Hooks belonging to anyone
# else are matched by command path and left exactly as they are, and so is every
# permission rule that is not one of the two above.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
app="/Applications/AgentToast.app"
lsregister="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"

keep_data=false
dry_run=false
for arg in "$@"; do
  case "$arg" in
    --keep-data) keep_data=true ;;
    --dry-run)   dry_run=true ;;
    -h|--help)   sed -n '2,8p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "Unknown option: $arg" >&2; exit 1 ;;
  esac
done

$dry_run && echo "DRY RUN — nothing will be changed." && echo

run() { $dry_run || "$@"; }

# --- stop it ----------------------------------------------------------------

if pgrep -f "$app/Contents/MacOS/agenttoast" >/dev/null 2>&1; then
  echo "Stopping AgentToast…"
  run pkill -f "$app/Contents/MacOS/agenttoast" || true
  $dry_run || sleep 2
fi

# A bridge blocked on a toast holds its agent up until it times out.
if pgrep -f "agenttoast-bridge-" >/dev/null 2>&1; then
  echo "Releasing bridges that were waiting on a toast…"
  run pkill -f "agenttoast-bridge-" || true
fi

# --- the configuration it wrote into other programs -------------------------

DRY_RUN=$dry_run python3 <<'PY'
import json, os, pathlib

dry = os.environ["DRY_RUN"] == "true"
home = pathlib.Path.home()
MARK = "agenttoast-bridge"
GRANTS = ("command(*)", "write_file(*)")

def save(path, data):
    if dry:
        return
    path.write_text(json.dumps(data, indent=2) + "\n")

def strip_claude_hooks(path, label):
    """Drop only the hook entries whose command is an AgentToast bridge."""
    if not path.exists():
        return
    try:
        settings = json.loads(path.read_text())
    except json.JSONDecodeError:
        print(f"  ! {path} is not valid JSON; left alone")
        return

    hooks = settings.get("hooks")
    if not isinstance(hooks, dict):
        return

    touched = []
    for event in list(hooks):
        groups, kept = hooks[event], []
        for group in groups:
            inner = [h for h in group.get("hooks", []) if MARK not in str(h.get("command", ""))]
            if len(inner) != len(group.get("hooks", [])):
                touched.append(event)
            if inner:
                kept.append({**group, "hooks": inner})
        if kept:
            hooks[event] = kept
        else:
            del hooks[event]

    if not touched:
        return
    if not hooks:
        settings.pop("hooks")
    save(path, settings)
    print(f"  {label}: removed {', '.join(sorted(set(touched)))}")

# Global, then every project the app remembered being connected to. Read the
# list before the data directory goes, because that is where it lives.
strip_claude_hooks(home / ".claude/settings.json", "~/.claude/settings.json")

projects = home / ".agenttoast/projects.json"
if projects.exists():
    try:
        for project in json.loads(projects.read_text()):
            strip_claude_hooks(pathlib.Path(project) / ".claude/settings.json", project)
    except (json.JSONDecodeError, TypeError):
        print("  ! ~/.agenttoast/projects.json is unreadable; project hooks not checked")

# Antigravity keeps ours under one key of its own.
path = home / ".gemini/config/hooks.json"
if path.exists():
    try:
        hooks = json.loads(path.read_text())
        if hooks.pop("agenttoast", None) is not None:
            save(path, hooks)
            print("  ~/.gemini/config/hooks.json: removed the agenttoast block")
    except json.JSONDecodeError:
        print(f"  ! {path} is not valid JSON; left alone")

# The grants. See the note at the top of this script.
path = home / ".gemini/antigravity-cli/settings.json"
if path.exists():
    try:
        settings = json.loads(path.read_text())
        allow = settings.get("permissions", {}).get("allow", [])
        withdrawn = [rule for rule in allow if rule in GRANTS]
        if withdrawn:
            settings["permissions"]["allow"] = [r for r in allow if r not in GRANTS]
            save(path, settings)
            print(f"  antigravity: withdrew {', '.join(withdrawn)}")
    except json.JSONDecodeError:
        print(f"  ! {path} is not valid JSON; left alone")
PY

# --- the app and its own files ----------------------------------------------

if [ -d "$app" ]; then
  echo "Removing $app"
  run rm -rf "$app"
fi

if $keep_data; then
  echo "Keeping ~/.agenttoast (paired devices and settings)"
else
  if [ -d "$HOME/.agenttoast" ]; then
    echo "Removing ~/.agenttoast (auth token, socket, paired devices)"
    run rm -rf "$HOME/.agenttoast"
  fi
fi

for leftover in \
  "$HOME/Library/Caches/com.agenttoast.app" \
  "$HOME/Library/Caches/agenttoast" \
  "$HOME/Library/WebKit/com.agenttoast.app" \
  "$HOME/Library/WebKit/agenttoast" \
  "$HOME/Library/Saved Application State/com.agenttoast.app.savedState"; do
  [ -e "$leftover" ] || continue
  echo "Removing ${leftover/#$HOME/~}"
  run rm -rf "$leftover"
done

# Staging bundles from local builds, which are what put duplicate entries in
# Spotlight in the first place.
if [ -d "$root/target/release/bundle" ]; then
  echo "Removing locally built bundles in target/release/bundle"
  run rm -rf "$root/target/release/bundle/macos/AgentToast.app" "$root/target/release/bundle/dmg"
fi

# --- forget it --------------------------------------------------------------

if [ -x "$lsregister" ]; then
  "$lsregister" -dump 2>/dev/null \
    | grep -oE "^path: +\S.*AgentToast\.app" \
    | sed 's/^path: *//' \
    | sort -u \
    | while read -r path; do
        echo "Unregistering $path"
        $dry_run || "$lsregister" -u "$path" 2>/dev/null || true
      done
fi

echo
$dry_run && echo "Nothing was changed." && exit 0
echo "AgentToast is gone. Reinstall with ./macos/scripts/install.sh"
