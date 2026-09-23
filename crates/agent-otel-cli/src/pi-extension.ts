// Managed by agent-otel-bridge. Native Pi event adapter; no model-visible output.
import { spawn } from "node:child_process";

const hook = __AGENT_OTEL_HOOK_PATH__;
export default function (pi: any) {
  const events = {
    session_start: "PreInvocation",
    tool_call: "PreToolUse",
    tool_result: "PostToolUse",
    agent_end: "Stop",
    session_shutdown: "PostInvocation",
  };
  for (const [source, target] of Object.entries(events)) {
    pi.on(source, (event: any, ctx: any) => {
      try {
        const payload = JSON.stringify({
          session_id: ctx.sessionManager.getSessionId(),
          cwd: ctx.cwd,
          hook_event_name: target,
          tool_name: event.toolName,
          tool_input: event.input,
          tool_use_id: event.toolCallId,
          is_error: event.isError,
        });
        const child = spawn(hook, [target, "--client", "pi"], {
          windowsHide: true,
          stdio: ["pipe", "ignore", "ignore"],
          timeout: 1000,
        });
        child.on("error", () => {});
        child.stdin.on("error", () => {});
        child.stdin.end(payload);
        child.unref();
      } catch {
        // Telemetry must not block or change Pi tool execution.
      }
    });
  }
}
