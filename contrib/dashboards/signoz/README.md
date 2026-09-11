# SigNoz Dashboard: AI Agent Observability (OpenTelemetry)

This directory contains the production SigNoz dashboard template for `agent-otel-bridge`.

- **File**: [`ai-agent-observability.json`](./ai-agent-observability.json)
- **Schema Version**: SigNoz `v6` (Perses / V2 agent-native format)
- **Standards**: OpenTelemetry GenAI (`gen_ai.*`) and Agent Lifecycle (`agent.*`)
- **Compatibility**: Antigravity, Claude Code, OpenAI Codex, xAI Grok, Inflection Pi

---

## How to Import into SigNoz

1. Open your SigNoz UI (e.g., `http://localhost:8080` or your SigNoz Cloud instance).
2. In the left navigation menu, click **Dashboards**.
3. Click **+ New dashboard** in the top right corner.
4. Select the **Import JSON** tab.
5. Either:
   - Click **Upload JSON file** and select `ai-agent-observability.json`, OR
   - Open `ai-agent-observability.json` in an editor, copy its contents, and paste it into the JSON text box.
6. Click **Import**.

The dashboard will instantly render all panels, layout grids, and dropdown variables.

---

## Dashboard Structure & Operator Panels

### Section 1: Quota & Token Economics (Guardrails & Spend)
- **Current Quota Remaining**: Real-time gauge of the active provider quota fraction (`0.0..=1.0`), grouped by `bucket` and model `group`.
- **Seconds to Quota Reset**: Countdown timer until the current rate-limiting window resets.
- **Active Sessions**: Total distinct interactive conversations (`gen_ai.conversation.id`) observed in the time window.
- **Quota Burn-Down Timeline**: Time series showing the rate of quota depletion over time.

### Section 2: Agent Health, Concurrency & Loop Protection
- **Agent Loop Invocations (Runaway Loop Detection)**: Time series of `PostInvocation` events. Rapid spikes indicate infinite tool-calling or recursion loops.
- **Tool Calls Over Time**: Granular time series of `PostToolUse` events categorized by `gen_ai.tool.name`.
- **Top Tools & Execution Summary**: Table comparing total calls and distinct sessions per tool.
- **Model Distribution**: Donut breakdown of requests across Foundation Models (`gemini-2.5-pro`, `claude-3-7-sonnet`, `o3-mini`, etc.).

### Section 3: Turn Outcomes & Safety Governance
- **Why Turns Ended (Quiescence Breakdown)**: Categorizes `agent.termination_reason`:
  - `NO_TOOL_CALL`: Natural, healthy turn completion in Antigravity.
  - `USER_INTERRUPT`: Cancellation by the developer.
  - `TURN_LIMIT`: Safety threshold reached.
- **Turns Completed Over Time**: Completed reasoning turns per model.
- **Recent Turn Audit Feed**: Live tabular trace feed with timestamp, agent name, termination reason, model, session ID, and step index.

---

## Dynamic Filtering Variables

- **`$service_name`**: Filter by OpenTelemetry service name (default `agent-otel-bridge`).
- **`$agent_name`**: Filter to specific AI agents (`antigravity`, `claude-code`, `codex`, `grok`, `pi`, or `ALL`).