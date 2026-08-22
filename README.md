# 🍞 AgentToast

**Cross-agent attention & action router for AI coding agents.**

AgentToast monitors AI coding agent sessions (Claude Code, Antigravity CLI) and instantly notifies you when they need your attention — with actionable buttons right in the notification.

## The Problem

You start an AI coding agent, give it a task, and switch to another application. The agent needs permission to run a command, but you don't notice for 10-20 minutes. Wasted time.

## The Solution

AgentToast detects when your agent is waiting and shows an interactive toast notification:

```
┌──────────────────────────────────────────────┐
│ 🔐 Claude Code needs your attention          │
│                                              │
│ Claude wants to run: npm run migrate         │
│                                              │
│ [Approve] [Deny] [Open Session]              │
└──────────────────────────────────────────────┘
```

Click **Approve** directly from the toast — no need to switch windows.

## How It Works

1. Agent hooks fire when a tool needs permission
2. A lightweight bridge script contacts the AgentToast daemon
3. A toast notification appears on your screen
4. You click a button
5. The response is routed back to the correct agent session

## Supported Agents

- **Claude Code** — via `PreToolUse` hooks
- **Antigravity CLI** — via `pre_tool_call` hooks (coming soon)

## Installation

### From Source

```bash
# Build everything
cargo build --release

# Install the bridge binary
cargo install --path crates/agenttoast-bridge-claude

# Run the daemon
cargo tauri dev
```

### Configure Claude Code Hooks

Add to your `~/.claude/settings.json` (or copy `config/hooks/claude-hooks.json`):

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": ".*",
        "hooks": [
          { "type": "command", "command": "agenttoast-bridge-claude" }
        ]
      }
    ],
    "SessionStart": [
      {
        "matcher": "startup|resume",
        "hooks": [
          { "type": "command", "command": "agenttoast-bridge-claude" }
        ]
      }
    ],
    "SessionEnd": [
      {
        "hooks": [
          { "type": "command", "command": "agenttoast-bridge-claude" }
        ]
      }
    ]
  }
}
```

The same binary handles all three: it dispatches on `hook_event_name`.
`PreToolUse` raises a toast and blocks on your answer; `SessionStart` and
`SessionEnd` only keep the session registry up to date and never block.

### Configuration

All settings are optional. Copy `config/default.toml` to
`~/.agenttoast/config.toml` and edit what you need:

```toml
# How long the bridge waits for you before deferring to the agent's own prompt
bridge_timeout = 600

[escalation]
enabled = true
reminder_intervals = [120, 300, 600]  # seconds
max_reminders = 0                     # 0 = unlimited
sound_on_reminder = true
```

An unanswered toast is re-surfaced on the `reminder_intervals` schedule, since
the agent stays blocked until you answer.

## Architecture

```
Claude Code / AGY
       │
   Hook fires
       │
       ▼
  Bridge Script ──── IPC (Named Pipe) ────▶ AgentToast Daemon
       │                                         │
       │                                    Show Toast
       │                                         │
       ◀──── User Response ◀──── Button Click ───┘
       │
  stdout response
       │
       ▼
  Agent continues
```

## Development

```bash
# Run in development mode
cargo tauri dev

# Run tests
cargo test --workspace

# Build release
cargo tauri build
```

## License

MIT
