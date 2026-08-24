# AgentToast

**Desktop notifications for Claude Code, with the answer buttons in the notification.**

You give Claude Code a task and switch to something else. It hits a command that
needs your approval and waits — and you don't notice for twenty minutes.

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

Answer it there and Claude carries on. No switching windows.

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

## How it works

Claude Code can run a program whenever it needs a permission decision. AgentToast
installs two pieces:

- **AgentToast** — a tray app that draws the toasts and waits for your click
- **agenttoast-bridge-claude** — a small program Claude Code runs when it needs an
  answer, which asks the tray app and reports back

They talk over a local named pipe. Nothing leaves your machine.

```
Claude Code needs permission
        │
        ▼
    bridge  ──── named pipe ────▶  AgentToast (tray)
        │                               │
        │                          shows a toast
        │                               │
        ◀────── your answer ◀───── you click a button
        │
   Claude continues
```

Four Claude Code hooks are used, and none of them block longer than they must:

| Hook | Raises |
| :--- | :--- |
| `PermissionRequest` | the approval toast — the only one that waits for you |
| `Stop` | the auto-dismissing "finished" toast |
| `Notification` | "Claude is waiting for you", if you have a notification channel set |
| `SessionStart` / `SessionEnd` | no toast; keeps the session list current |

## Requirements

- **Windows 10 or 11.** The tray app, the named pipe and the window handling are
  Windows-specific today. macOS and Linux are not supported.
- **Claude Code**, with a version that supports the `PermissionRequest` hook.

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
3. Under **Claude Code**, press **Connect** on *Every project*.
4. **Restart any Claude Code session** that is already running. Hooks are read when
   a session starts.

That's it. Next time Claude needs your approval, you get a toast.

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
- **Connect writes to Claude Code's settings file.** That is the whole feature, but
  it is your config — hence the backup, and the fact that other tools' hooks are
  never touched.
- A toast can only ever answer a question Claude Code actually asked.

## Building

Needs [Rust](https://rustup.rs/) and the MSVC build tools.

```bash
git clone https://github.com/AryanMahajan/claude_notifier
cd claude_notifier

# Run it
cargo run -p agenttoast

# Tests
cargo test --workspace

# Build the installer
cargo tauri build --config src-tauri/tauri.bundle.conf.json
```

The installer lands in `target/release/bundle/nsis/`.

**That `--config` flag is required.** The bridge is a second binary that has to be
bundled alongside the app, and Tauri checks resource paths at compile time — so
declaring it in the main config would break a plain `cargo build`. The extra file
adds it only when packaging.

## Status

Working and in daily use, but early. Known gaps:

- **Windows only.**
- **Open Session** raises the right terminal window when it can identify it, and
  otherwise raises all of them so you can pick. It cannot switch to the right *tab*
  — no supported API for that.
- **Antigravity** is not supported yet, despite the name suggesting a general tool.
  The architecture allows for it; the adapter does not exist.

## License

MIT — see [LICENSE](LICENSE).
