# AgentToast

**Desktop notifications for Claude Code and Antigravity, with the answer buttons in
the notification.**

You give your coding agent a task and switch to something else. It hits a command
that needs your approval and waits — and you don't notice for twenty minutes.

AgentToast puts that on your desktop the moment it happens:

```
┌──────────────────────────────────────────────┐
│  C   Claude Code · Needs approval        now │
│      Remove hello.txt                        │
│      rm "C:/Users/you/project/hello.txt"     │
│                                              │
│      [ Approve ]  [ Deny ]     Open Session  │
└──────────────────────────────────────────────┘
```

Answer it there and the agent carries on. No switching windows.

It also tells you when Claude **asks a question**, and when it **finishes**:

```
┌──────────────────────────────────────────────┐        ┌────────────────────────────────┐
│  C   Claude Code · Question              now │        │  C   Claude Code · Done    now │
│      Which target should I build?            │        │      Ran the test suite.       │
│                                 Open Session │        │                                │
└──────────────────────────────────────────────┘        └────────────────────────────────┘
   stays until you deal with it                            clears itself after 6s
```

## It respects your permission mode

This is the part that matters, and it is why AgentToast hooks
**`PermissionRequest`** rather than every tool call.

`PermissionRequest` fires only once Claude Code has decided it needs a human. So
in `auto` or `acceptEdits` — where you have deliberately told Claude to stop
asking — you get **no toasts**, because Claude is not asking. Switch back to
`default` and they return.

A tool that notified you on every tool call would override the very setting you
chose. This one cannot, because it never sees the calls Claude handles itself.

**Antigravity works differently.** It has no equivalent event — nothing that means
"the agent has decided it needs a human". Its only gate is `PreToolUse`, which
fires for every tool that matches, whether or not Antigravity would have asked. So
there the matcher does the filtering, and it is set by default to the calls that
change something:

```
run_command  write_to_file  create_file  replace_file_content  edit_notebook  delete_file
```

Reading a file, listing a directory or searching the codebase raises nothing.

**Antigravity has two gates, and they are not the same gate.** Shell commands go
through its permission rules in every execution mode. File edits go through the
mode itself: `default` pauses for a line-level diff review, `accept-edits` does
not pause at all, `plan` mostly stays read-only. A hook is told neither which
mode is running nor whether the call would have prompted — so **Raise a toast
for** in the Antigravity panel lets you say. *Commands and file edits* suits
`default`; *Commands only* suits `accept-edits`, where a toast per edit would put
back exactly the interruption that mode exists to remove.

**Approve works differently for Antigravity, and you have to turn it on.** An
Antigravity hook can block a call or force a prompt, but nothing it returns can
*grant* one — so out of the box a toast there is a heads-up and a kill switch,
and approving means going to the terminal. Switching on **Approve from the toast**
in the dashboard grants Antigravity the tools AgentToast already watches, which
leaves the toast as the only thing that gets asked. Deny still blocks the call,
and anything AgentToast cannot answer goes back to Antigravity's own prompt.

The catch is worth stating plainly: with that switch on, Antigravity no longer
asks for itself, and it treats a hook that fails or is missing as consent. If
AgentToast is running, the toast is the gate. If it has been uninstalled or its
bridge deleted without pressing Disconnect, nothing is. Disconnect gives the
grants back, and AgentToast withdraws them on start if it finds them with no
hook behind them.

## How it works

Both agents can run a program of your choosing when something happens. AgentToast
installs a tray app and one small bridge program per agent:

- **AgentToast** — a tray app that draws the toasts and waits for your click
- **agenttoast-bridge-claude** — run by Claude Code when it needs an answer
- **agenttoast-bridge-agy** — the same for Antigravity

A bridge asks the tray app and reports the answer back. They talk over a local
named pipe. Nothing leaves your machine.

```
agent needs permission
        │
        ▼
    bridge  ──── named pipe ────▶  AgentToast (tray)
        │                               │
        │                          shows a toast
        │                               │
        ◀────── your answer ◀───── you click a button
        │
   the agent continues
```

Claude Code hooks, none of them blocking longer than they must:

| Hook | Raises |
| :--- | :--- |
| `PermissionRequest` | the approval toast — the only one that waits for you |
| `Stop` | the auto-dismissing "finished" toast |
| `Notification` | "Claude is waiting for you", if you have a notification channel set |
| `SessionStart` / `SessionEnd` | no toast; keeps the session list current |

Antigravity hooks:

| Hook | Raises |
| :--- | :--- |
| `PreToolUse` | the toast, for matching tools only — Deny blocks the call; Approve needs the dashboard switch, without which it hands back to Antigravity's own prompt |
| `Stop` | the auto-dismissing "finished" toast |

Antigravity has no session lifecycle hooks, so its sessions appear in the
dashboard the first time they raise something.

## Requirements

- **Windows 10 or 11.** The tray app, the named pipe and the window handling are
  Windows-specific today. macOS and Linux are not supported.
- **At least one supported agent**:
  - **Claude Code**, with a version that supports the `PermissionRequest` hook, or
  - **Antigravity** (`agy`), with a version that supports `hooks.json`.

## Install

Download the latest `AgentToast_x.y.z_x64-setup.exe` from
[Releases](../../releases) and run it.

Windows will warn you that the publisher is unknown, because the installer is not
code-signed. Choose **More info → Run anyway**. If you would rather not, build it
yourself — see [Building](#building) below.

Updating over an existing install is fine: the installer closes AgentToast first.
(Before 0.1.1 it did not, and silently left the old version in place.)

Then:

1. Launch **AgentToast**. It appears in the system tray. (Windows 11 hides new tray
   icons — click the `^` next to the clock, and drag it out to pin it.)
2. **Left-click the tray icon** to open the dashboard.
3. Press **Connect** — under **Claude Code** on *Every project*, under
   **Antigravity** on *This machine*, or both.
4. **Restart any session** that is already running. Both agents read their hooks
   when a session starts.

That's it. Next time the agent needs your approval, you get a toast.

### Connecting one project instead of all of them

Press **Add a project…**, pick the folder, then **Connect** on its row. Only that
project sends toasts. Projects stay in the list until you remove them with the `×`.

### What Connect actually does

It writes a `hooks` block into Claude Code's settings — `~/.claude/settings.json`
for every project, or `<project>/.claude/settings.json` for one.

It treats that file as yours: it is backed up to `settings.json.agenttoast-backup`
first, key order is preserved, hooks belonging to other tools are left alone, and
pressing Connect twice updates AgentToast's entry rather than adding a second one.

**Disconnect before uninstalling.** Hooks left pointing at a deleted program make
Claude Code report an error on every tool call.

If you would rather wire it up by hand, copy `config/hooks/claude-hooks.json` into
your settings and replace `agenttoast-bridge-claude` with the full path to the
installed bridge.

### Connecting Antigravity

Under **Antigravity**, press **Connect** on *This machine*, then start a new `agy`
session.

**A session that was already open will not pick this up.** Antigravity reads
`hooks.json` once, at startup, and says so only in its own log (`loaded 0 named
hooks`). An older session goes on prompting in the terminal exactly as it did
before, with nothing to indicate that AgentToast is connected but not being asked.
Close it and start a new one.

Two more things differ from Claude Code:

- **There is nothing to set up per project.** Antigravity's other place for hooks
  is a workspace's `.agents/hooks.json`, which is meant to be committed and shared
  with your team, and a machine-local absolute path does not belong in a
  repository. AgentToast only writes the global file, `~/.gemini/config/hooks.json`.
- **The install path must not contain a space.** Antigravity splits a hook command
  on whitespace and does not honour quotes, so there is no way to express one. The
  default install location is fine; `C:\Program Files\...` is not, and Connect
  will say so rather than writing a hook that fails on every tool call.

The file is backed up to `hooks.json.agenttoast-backup` first, and hooks belonging
to other tools — which Antigravity files under their own names — are left alone.

To wire it up by hand instead, copy `config/hooks/agy-hooks.json` into
`~/.gemini/config/hooks.json` and replace `agenttoast-bridge-agy` with the full
path to the installed bridge. For a working Approve button, merge
`config/hooks/agy-settings-permissions.json` into
`~/.gemini/antigravity-cli/settings.json` as well — that is exactly what the
dashboard switch writes.

**Disconnect before uninstalling, if you turned Approve on.** Antigravity treats a
hook that fails, or that points at a binary no longer there, as a hook with no
opinion — it does not fail the tool call, it lets it through. That is forgiving
right up until the grants are in place, at which point a missing bridge means
every call sails past unasked. Disconnect gives the grants back. AgentToast also
withdraws them on start if it finds them with no hook behind them, but it cannot
do that once it has been uninstalled.

## Using it

**The toast.** Approve or Deny answers Claude directly. **Open Session** raises the
terminal and hands the decision back, so Claude asks you there instead.

**Questions.** When Claude asks something, the question itself is the headline.
A hook cannot send an answer back, so the toast takes you to the session.

**Finishing.** A "Done" toast appears when Claude ends its turn and clears itself
after six seconds.

**Closing a toast does not answer it.** The `×` hides the card; the request stays
pending and Claude keeps waiting. Reopen it from the dashboard, or wait — an
unanswered toast comes back on the reminder schedule.

**The dashboard** (left-click the tray icon) lists active sessions and lets you
answer from there too.

## Configuration

Optional. Copy `config/default.toml` to `~/.agenttoast/config.toml` and edit:

```toml
# How long the bridge waits before giving up and letting Claude ask you itself
bridge_timeout = 600

[escalation]
enabled = true
reminder_intervals = [120, 300, 600]  # seconds
max_reminders = 0                     # 0 = unlimited
sound_on_reminder = true
```

An unanswered toast is re-surfaced on that schedule, because Claude stays blocked
until you answer.

## Security

AgentToast can approve commands on your machine, so it is worth being clear about
what that means.

- The bridge and tray app talk over a **local named pipe**, guarded by a token
  written to `~/.agenttoast/auth_token`. Nothing is exposed to the network.
- **Connect writes to the agent's own config file** — Claude Code's settings, or
  Antigravity's `hooks.json`. That is the whole feature, but it is your config —
  hence the backup, and the fact that other tools' hooks are never touched.
- A toast can only ever answer a question the agent actually asked.

## Building

Needs [Rust](https://rustup.rs/) with the MSVC build tools, and
[Node](https://nodejs.org/) 20 or newer for the front end.

```bash
git clone https://github.com/AryanMahajan/claude_notifier
cd claude_notifier
npm install

# Run it — Vite serves the windows with hot reload
cargo tauri dev

# Tests
cargo test --workspace   # Rust
npm run typecheck        # TypeScript

# Build the installer
cargo tauri build --config src-tauri/tauri.bundle.conf.json
```

The installer lands in `target/release/bundle/nsis/`.

**The front end is built by Vite, into `dist/`.** `cargo tauri build` runs
`npm run build` for you, so a checkout only needs `npm install` once. There are
two entry points, one per window:

| Window | Entry | Stack |
| :--- | :--- | :--- |
| Dashboard | `ui/dashboard.html` → `ui/dashboard/` | React + TypeScript + Tailwind |
| Toast | `ui/index.html` → `ui/toast.js` | Plain TypeScript-free JS and CSS |

The toast stays framework-free on purpose. It opens on the critical path of an
agent waiting for an answer, and the tray app pre-warms a hidden copy at startup
to hide WebView2's cold start — a framework there would cost first paint and buy
nothing. `ui/tokens.css` holds the palette both windows share; the dashboard
re-exports it to Tailwind with `@theme inline`, so a colour is written once.

Running `cargo run -p agenttoast` directly works too, but it serves whatever is
already in `dist/` — run `npm run build` first if the front end has changed.

**That `--config` flag is required.** The bridges are separate binaries that have
to be bundled alongside the app, and Tauri checks resource paths at compile time —
so declaring them in the main config would break a plain `cargo build`. The extra
file adds them only when packaging.

## Status

Working and in daily use, but early. Known gaps:

- **Windows only.**
- **Open Session** raises the right terminal window when it can identify it, and
  otherwise raises all of them so you can pick. It cannot switch to the right *tab*
  — no supported API for that.
- **Approve for Antigravity is all-or-nothing.** Its hooks cannot grant a single
  call, so the switch grants `command(*)` and `write_file(*)` for the whole
  machine and relies on AgentToast being there to answer. There is no way to
  scope that per call, per project or per session, and no way for the bridge to
  cover its own absence.
- **Antigravity toasts cannot follow its execution mode.** Antigravity gates
  commands and file edits differently: commands go through its permission rules
  in every mode, but file edits go through the *mode* — `default` pauses for a
  diff review, `accept-edits` deliberately does not pause at all. Nothing in a
  hook payload says which mode is running (`HookArgsCommon` carries no such
  field), and `--mode` and Shift+Tab both change it without writing it down, so
  AgentToast cannot follow it. **Raise a toast for** in the Antigravity panel is
  the manual version: set it to *Commands only* for an `accept-edits` session.
- **Antigravity toasts cannot tell an auto-approved call from a real question.**
  Its `PreToolUse` hook fires for every matching tool whether or not Antigravity
  would have prompted, so the matcher is the only filter there is.

## License

MIT — see [LICENSE](LICENSE).
