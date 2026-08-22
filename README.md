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

Add to your `~/.claude/settings.json`:

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": ".*",
        "hooks": [
          {
            "type": "command",
            "command": "agenttoast-bridge-claude"
          }
        ]
      }
    ]
  }
}
```

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
