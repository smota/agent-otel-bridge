# AI Agent Harness Integration Guide

`agent-otel-bridge` is designed to provide zero-overhead OpenTelemetry observability across modern AI CLI agent harnesses and custom developer agent loops.

---

## 1. Supported Client Overview

| Harness | Configuration Location (Global) | Configuration Location (Project) | Hook Format |
|---|---|---|---|
| **Google Antigravity (`agy`)** | `~/.gemini/config/hooks.json` | `.gemini/hooks.json` | JSON Array of Event Handlers |
| **Claude Code** | `~/.claude/settings.json` | `.claude/settings.json` | JSON `hooks` Dictionary with Command Arrays |
| **OpenAI Codex CLI** | `~/.codex/hooks.json` | `.codex/hooks.json` | JSON Array of Event Handlers |
| **xAI Grok CLI** | `~/.grok/hooks.json` | `.grok/hooks.json` | JSON Array of Event Handlers |
| **Pi CLI (`pi`)** | `~/.pi/hooks.json` | `.pi/hooks.json` | JSON Array of Event Handlers |
| **Custom Agent Harness** | Environment / Custom Config | Custom Config | Direct Process Execution (`agent-hook.exe`) |

---

## 2. Automated Hook Registration

The fastest way to configure all installed AI agents on your machine:

```powershell
# Automatically detect installed agents and register hooks
agent-otel-bridge install-hooks

# Or target a specific client
agent-otel-bridge install-hooks --client antigravity
agent-otel-bridge install-hooks --client claude
agent-otel-bridge install-hooks --client codex
agent-otel-bridge install-hooks --client grok
agent-otel-bridge install-hooks --client pi

# Force configure all 5 clients
agent-otel-bridge install-hooks --client all

# Register hooks locally at the current project level
agent-otel-bridge install-hooks --client claude --project
```

To verify registration status across all clients:
```powershell
agent-otel-bridge hooks status
```

---

## 3. Harness Configuration Details

### 3.1 Google Antigravity (`agy`)

Antigravity executes hook commands configured in `hooks.json`. When lifecycle events occur, `agy` streams a JSON payload to `stdin`.

**Global Config**: `~/.gemini/config/hooks.json` (or `%USERPROFILE%\.gemini\config\hooks.json`)

```json
{
  "agent-otel-bridge": {
    "PreToolUse": [
      {
        "matcher": ".*",
        "hooks": [
          {
            "type": "command",
            "command": "agent-hook PreToolUse",
            "timeout": 5
          }
        ]
      }
    ],
    "PostToolUse": [
      {
        "matcher": ".*",
        "hooks": [
          {
            "type": "command",
            "command": "agent-hook PostToolUse",
            "timeout": 5
          }
        ]
      }
    ],
    "PreInvocation": [
      {
        "type": "command",
        "command": "agent-hook PreInvocation",
        "timeout": 5
      }
    ],
    "PostInvocation": [
      {
        "type": "command",
        "command": "agent-hook PostInvocation",
        "timeout": 5
      }
    ],
    "Stop": [
      {
        "type": "command",
        "command": "agent-hook Stop",
        "timeout": 5
      }
    ]
  }
}
```

**Payload Schema streamed to `stdin`:**
```json
{
  "conversationId": "38d58ff9-8e43-43cf-bf24-e9188be08a1c",
  "stepIdx": 42,
  "toolCall": {
    "id": "call_12345",
    "name": "run_command",
    "arguments": {
      "CommandLine": "cargo test"
    }
  },
  "modelName": "gemini-2.5-pro",
  "executionNum": 1,
  "fullyIdle": false
}
```

---

### 3.2 Claude Code

Claude Code registers lifecycle hooks under the `"hooks"` key in `settings.json`.

**Global Config**: `~/.claude/settings.json` (or `%USERPROFILE%\.claude\settings.json`)

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "",
        "hooks": [
          {
            "type": "command",
            "command": "agent-hook PreToolUse"
          }
        ]
      }
    ],
    "PostToolUse": [
      {
        "matcher": "",
        "hooks": [
          {
            "type": "command",
            "command": "agent-hook PostToolUse"
          }
        ]
      }
    ],
    "Stop": [
      {
        "matcher": "",
        "hooks": [
          {
            "type": "command",
            "command": "agent-hook Stop"
          }
        ]
      }
    ]
  }
}
```

**Payload Schema streamed to `stdin`:**
```json
{
  "session_id": "9a12c8b0-51ea-4bb3-93cf-7e3f8ba01c22",
  "turn_index": 12,
  "tool_use_id": "toolu_01ABCD1234",
  "tool_name": "Bash",
  "tool_input": {
    "command": "git status"
  },
  "tool_result": {
    "output": "On branch main\nnothing to commit"
  },
  "model": "claude-3-5-sonnet"
}
```
`agent-otel-bridge` automatically parses both camelCase and snake_case representations, normalizes `session_id` into `gen_ai.conversation.id`, and maps `Bash` tool calls to canonical OpenTelemetry spans.

---

### 3.3 OpenAI Codex CLI

OpenAI Codex CLI hooks are configured via `~/.codex/hooks.json`.

```json
{
  "hooks": [
    {
      "event": "PreToolUse",
      "command": "agent-hook PreToolUse"
    },
    {
      "event": "PostToolUse",
      "command": "agent-hook PostToolUse"
    },
    {
      "event": "Stop",
      "command": "agent-hook Stop"
    }
  ]
}
```
Spans emitted from Codex sessions automatically receive:
- `gen_ai.provider.name`: `"openai"`
- `gen_ai.agent.name`: `"codex"`
- `gen_ai.request.model`: e.g. `gpt-4o`, `o1-preview`, `o3-mini`

---

### 3.4 xAI Grok CLI

xAI Grok CLI hooks are configured via `~/.grok/hooks.json`.

```json
{
  "hooks": [
    {
      "event": "PreToolUse",
      "command": "agent-hook PreToolUse"
    },
    {
      "event": "PostToolUse",
      "command": "agent-hook PostToolUse"
    },
    {
      "event": "Stop",
      "command": "agent-hook Stop"
    }
  ]
}
```
Spans emitted from Grok sessions automatically receive:
- `gen_ai.provider.name`: `"xai"`
- `gen_ai.agent.name`: `"grok"`
- `gen_ai.request.model`: e.g. `grok-2`, `grok-beta`

---

### 3.5 Pi CLI (pi.dev)

[Pi CLI](https://pi.dev/) hooks are configured via `~/.pi/hooks.json`.

```json
{
  "hooks": [
    {
      "event": "PreToolUse",
      "command": "agent-hook PreToolUse"
    },
    {
      "event": "PostToolUse",
      "command": "agent-hook PostToolUse"
    },
    {
      "event": "Stop",
      "command": "agent-hook Stop"
    }
  ]
}
```
Spans emitted from Pi sessions automatically receive:
- `gen_ai.provider.name`: `"pi"`
- `gen_ai.agent.name`: `"pi"`

---

## 4. Custom LLM Agents & Developer Harnesses

You can instrument any custom agent loop (Python, Node.js, Go, Rust, or Bash) using `agent-hook.exe`.

### How It Works
1. When your agent executes a tool or completes a turn, spawn `agent-hook.exe <EventName>` as a child process.
2. Write the JSON payload to the child's `stdin` and close it.
3. Read `stdout` (`{}`) and exit code (`0`).
4. `agent-hook.exe` executes in **< 1ms**, writes to the local Named Pipe asynchronously, and never blocks your agent.

### Python Example

```python
import subprocess
import json

def emit_agent_hook(event: str, session_id: str, tool_name: str, args: dict, model: str = "custom-agent"):
    payload = json.dumps({
        "conversationId": session_id,
        "toolCall": {
            "name": tool_name,
            "arguments": args
        },
        "modelName": model
    }).encode("utf-8")

    try:
        proc = subprocess.run(
            ["agent-hook.exe", event],
            input=payload,
            capture_output=True,
            timeout=0.010 # 10ms hard timeout guard
        )
    except Exception as e:
        # Fails open: Telemetry must never crash or block agent execution
        pass

# Example usage during tool execution
emit_agent_hook("PostToolUse", "sess-123", "grep_search", {"Query": "pattern"})
```

### Node.js Example

```typescript
import { spawn } from "child_process";

function emitHook(event: string, payload: Record<string, any>): void {
  try {
    const child = spawn("agent-hook.exe", [event], {
      stdio: ["pipe", "ignore", "ignore"],
      windowsHide: true,
    });

    child.stdin.write(JSON.stringify(payload));
    child.stdin.end();

    // Do not wait for exit - allow agent event loop to continue uninterrupted
    child.unref();
  } catch (err) {
    // Fail open
  }
}
```
