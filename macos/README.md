# AgentToast on macOS

The same application, in a menu bar instead of a system tray. Everything the
[main README](../README.md) describes — the approval toast, the question and
done toasts, the dashboard, both connectors, the phone page — works the same
way here. This file covers only what is different, and why.

Everything Mac-specific lives in this folder. Nothing in it is compiled on a
Windows build, and the Windows behaviour is unchanged.

## What is different

| | Windows | macOS |
| :--- | :--- | :--- |
| Bridge ↔ app transport | named pipe `\\.\pipe\agenttoast` | Unix socket `~/.agenttoast/agenttoast.sock` |
| Where a toast appears | bottom right, above the taskbar | top right, under the menu bar |
| Lives in | the system tray | the menu bar, with no Dock icon |
| Tray click | left opens the dashboard, right opens the menu | the same — which is *not* Tauri's default |
| Open Session | finds the terminal's window and raises it | finds the terminal's `.app` and activates it |
| Firewall prompt on the Remote tab | Windows Defender Firewall | macOS "accept incoming connections?" |

Both agents keep their config in the same place on both platforms —
`~/.claude/settings.json`, `~/.gemini/config/hooks.json` — so Connect,
Disconnect, the backups and the per-project rows all behave identically.

## Requirements

- **macOS 10.15 or newer**, Apple silicon or Intel.
- **At least one supported agent**: Claude Code with a version that has the
  `PermissionRequest` hook, or Antigravity (`agy`) with a version that reads
  `hooks.json`.

No Accessibility or Automation permission is needed, and AgentToast will never
ask for one — see [Open Session](#open-session) for why.

## Install

Build it yourself for now; there is no signed release build. See
[Building](#building) below, then:

```bash
cp -R target/release/bundle/macos/AgentToast.app /Applications/
open /Applications/AgentToast.app
```

If you install from a `.dmg` that was downloaded rather than built locally,
macOS quarantines it and Gatekeeper refuses to open it at all — the build is not
signed or notarised. Right-click → **Open** gets past that once, or:

```bash
xattr -dr com.apple.quarantine /Applications/AgentToast.app
```

Then:

1. **AgentToast appears in the menu bar.** There is no Dock icon and no window;
   that is deliberate — see [No Dock icon](#no-dock-icon).
2. **Left-click the menu bar icon** to open the dashboard. Right-click for the
   menu, which is where Quit is.
3. Press **Connect** — under **Claude Code** on *Every project*, under
   **Antigravity** on *This machine*, or both.
4. **Restart any session that is already running.** Both agents read their hooks
   when a session starts.

### Keep it out of a path with a space

`/Applications` is fine. A folder like `~/My Apps` is not, and only for
Antigravity: it splits a hook command on whitespace and honours no quoting, so
there is no way to express such a path at all. Connect says so rather than
writing a hook that fails on every tool call.

Claude Code is unaffected — it runs hooks through a shell, so AgentToast quotes
the path for it.

### Starting it at login

Not automated. System Settings → General → Login Items → **Open at Login** →
add `AgentToast.app`. AgentToast does not install a launch agent, because a
background app that silently arranges to start forever is not something a tool
should do to you on first run.

## Open Session

The toast's **Open Session** button raises the terminal the agent is running in.
On macOS a process does not own a window — an application does — so the search
walks up from the agent to the first ancestor running out of a `.app` bundle,
and activates that bundle through Launch Services:

```text
claude                                                    ← no bundle
└── zsh                                                   ← no bundle
    └── login
        └── Terminal   /System/…/Terminal.app/…/Terminal  ← this one
```

An editor works the same way. VS Code runs its integrated terminal from a helper
that is itself a bundle, so the *outermost* `.app` is the answer — the helper has
no user interface to raise.

**No permission prompt, ever.** The obvious call, `NSRunningApplication.activate`,
has been progressively restricted: since Sonoma a background app cannot simply
take the foreground, and the call fails silently. `System Events` can do it, but
only after an Automation grant the user can decline — leaving a button that
quietly stops working. Launch Services is allowed to move the foreground, needs
no grant, and activates a running app rather than starting a second copy.

**It cannot pick the tab.** Terminal.app, iTerm2 and Ghostty are raised as a
whole. There is no supported way to select the split or tab the session is
actually in — the same limitation as on Windows.

If the agent's terminal cannot be identified at all, every running terminal is
raised so you can pick, rather than guessing one and burying the one you wanted.

## No Dock icon

AgentToast sets `NSApplicationActivationPolicyAccessory` at startup: menu bar
only, no Dock tile, no Cmd-Tab entry. It has no window of its own to return to —
a Cmd-Tab entry for it is a switch to nothing.

The policy is set in code rather than declared as `LSUIElement` in the bundle, so
that a `cargo tauri dev` run behaves like an installed one. The cost is a Dock
tile for the fraction of a second before it runs. To remove even that, add an
`Info.plist` alongside `src-tauri/tauri.conf.json` containing:

```xml
<key>LSUIElement</key><true/>
```

## The Remote tab

Unchanged, except for the prompt. The first time the phone page binds a port,
macOS asks whether **AgentToast** may accept incoming network connections. Say
yes; a phone cannot reach it otherwise.

Everything the main README says about that feature still applies — same network
only, plain HTTP, nothing listening until you switch it on.

## Building

Needs [Rust](https://rustup.rs/) (1.88 or newer), [Node](https://nodejs.org/) 20
or newer, and Xcode's command line tools (`xcode-select --install`).

```bash
git clone https://github.com/AryanMahajan/claude_notifier
cd claude_notifier
npm install
cargo install tauri-cli --version "^2" --locked   # or: cargo binstall

./macos/scripts/dev.sh         # run it, front end hot-reloading
./macos/scripts/install.sh     # build it and put it in /Applications
./macos/scripts/uninstall.sh   # take it back off this Mac, hooks and all
./macos/scripts/build.sh       # just the bundles, left in target/
```

`install.sh` is the one to use. It builds, quits any running copy, replaces
`/Applications/AgentToast.app`, and then removes the staging bundle it built
from and takes every other path back out of the Launch Services database.

That last step is not tidiness. Launch Services indexes every app bundle it ever
sees, and `cargo tauri build` produces two each time — one in
`target/release/bundle/macos/` and one inside the disk image it mounts to build
the `.dmg`. After a few builds Spotlight, Raycast and the Open dialog all offer
several identical "AgentToast" entries, most of them pointing at a staging copy
or at a `/Volumes/dmg.XXXXXX` that was unmounted long ago. Choosing one of those
runs a stale build, or nothing at all.

`install.sh` builds the `.app` alone. Producing a `.dmg` means *mounting* it,
which throws a Finder window on screen partway through the build — pass
`--with-dmg` when you actually want a disk image to hand to someone.

`build.sh` leaves the bundles in `target/release/bundle/` and makes no such
promise — expect the duplicate entries until they are installed or deleted.

## Uninstalling

```bash
./macos/scripts/uninstall.sh              # remove everything
./macos/scripts/uninstall.sh --keep-data  # keep ~/.agenttoast, paired phones and all
./macos/scripts/uninstall.sh --dry-run    # say what it would do, change nothing
```

Dragging the app to the Trash is not enough, because AgentToast works by writing
itself into other programs' configuration. Left behind, those entries point at a
binary that no longer exists:

| | |
|---|---|
| `~/.claude/settings.json` | hooks on five events |
| `<project>/.claude/settings.json` | the same, for every project it was connected to |
| `~/.gemini/config/hooks.json` | an `agenttoast` block |
| `~/.gemini/antigravity-cli/settings.json` | `command(*)` and `write_file(*)` |

The last row is the one that matters. Those two grants are a standing
instruction to Antigravity to stop asking before it runs a command or writes a
file, and they are only safe while AgentToast's hook is there to answer for
them. With the hook gone they are a blanket approval with nothing behind it, and
nothing in Antigravity will bring them up again.

The script also takes back `~/.agenttoast`, the WebKit and cache directories,
any bundle left in `target/`, and every Launch Services registration.

Everything it touches is scoped to what AgentToast wrote: hooks are matched by
the bridge path in their command, so anybody else's are left alone, as is every
permission rule that is not one of those two. `--dry-run` prints the whole list
without changing a thing.

**Use the scripts, not a bare `cargo tauri build`.** Both pass
`--config macos/tauri.macos.conf.json`, which carries the `.icns`, the `app` and
`dmg` targets, and — the part that actually bites — the two bridge binaries as
bundle resources, named without the `.exe` the Windows bundle config uses. They
are what the agents run. An app bundled without them installs cleanly and then
cannot connect to anything.

`build.sh` also builds those bridges for the same architecture as the app. Bundle
resource paths are fixed strings, so the app always takes its bridges from
`target/release/`; left alone that is whatever this machine happens to be, which
only shows up once the app is built universal and an Intel Mac gets two bridges
it cannot execute.

**Transparency is not in that file.** A toast window has to be transparent — the
card's shadow and its slide-in live in a gutter that must not paint — and on
macOS that goes through an API Apple does not publish, so Tauri keeps it behind
the `macos-private-api` Cargo feature. `tauri-build` refuses to build unless
that feature and `macOSPrivateApi` in `tauri.conf.json` agree, and it checks on
*every* platform, so both are set unconditionally in the shared files rather
than in this folder. They are inert off macOS. Do not try to move them here: a
`[target.'cfg(target_os = "macos")'.dependencies]` entry is read by
`tauri-build` whatever it is building for, and breaks the Windows build.

Tests are shared with the rest of the project:

```bash
cargo test --workspace
npm run typecheck
```

### Icons

`macos/icons/icon.icns` is committed, so a build does not need to regenerate it.
When the mark changes:

```bash
./macos/scripts/generate-icons.sh
```

Every size is drawn from scratch by `render-icon.py` — standard library only, no
`pip install` — rather than downscaled from one large render, because below 20px
the two bars in the mark turn to mush and a single bolder bar is drawn instead.
It is the same mark and the same thresholds as `scripts/generate-icons.ps1`
draws for Windows.

## What is in this folder

```text
macos/
  README.md                 this file
  tauri.macos.conf.json     bundle config: private API, .icns, bridge resources
  icons/icon.icns           app icon, 16px to 1024px
  scripts/
    dev.sh                  run from source
    install.sh              build, install into /Applications, leave one bundle
    uninstall.sh            remove the app, its data, and every hook it wrote
    build.sh                .app and .dmg
    generate-icons.sh       rebuild icon.icns
    render-icon.py          draw the mark at one size, no dependencies
  src/
    mod.rs                  what is here and why
    focus.rs                Open Session: process tree → .app → Launch Services
    anchor.rs               toast geometry: top right, growing downward
    runtime.rs              accessory app, menu bar click, Spaces
```

The shared code reaches all of it through `#[cfg(target_os = "macos")]` arms in
`src-tauri/src/{main,focus,window,tray}.rs`,
`src-tauri/src/{hooks,agy_hooks}.rs` and
`crates/agenttoast-{core,ipc}/src/`. Each of those is an added branch beside the
Windows one, never a change to it.

## Known gaps

- **Not signed or notarised.** A downloaded build is refused by Gatekeeper until
  the quarantine flag is removed.
- **The menu bar icon is in colour, not a template image.** A template made from
  a coloured icon is a filled silhouette with none of the shape that makes it
  findable, so colour is the better of the two; a purpose-drawn monochrome glyph
  would be better than either, and does not exist yet.
- **Open Session raises the application, not the tab.**
- **No launch-at-login.** Add it yourself in Login Items.
- Everything in the main README's [Status](../README.md#status) section still
  applies — in particular, Antigravity's Approve switch is all-or-nothing there
  too.
