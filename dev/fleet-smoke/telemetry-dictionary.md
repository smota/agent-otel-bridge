# Fleet smoke telemetry dictionary

This dictionary records attributes emitted by `dev/fleet-smoke`. Values are
laboratory observations or planned expectations; they do not establish native
bridge propagation or provider model identity. Attributes are omitted when no
evidence exists. No speculative attributes are defined here.

## Resource attributes

| Attribute | Scope | Meaning |
|---|---|---|
| `service.name` | resource | `agent-otel-fleet-smoke` |
| `service.version` | resource | Laboratory package version |
| `agent.smoke.origin` | resource/span | Source of the observation, usually `fleet-smoke-lab` or a fixture origin |
| `agent.smoke.run_id` | resource | Unique run identity |
| `agent.smoke.trace_id` | resource | W3C trace identity |
| `agent.smoke.root_span_id` | resource | Reserved root span identity |
| `agent.smoke.seed` | resource | Deterministic fixture seed |
| `agent.smoke.profile` | resource | Selected scenario profile |
| `agent.smoke.scenario_version` | resource | Scenario contract version (`1`) |
| `agent.smoke.plan_hash` | resource | SHA-256 hash of canonical scenario JSON |
| `agent.smoke.plan_hash_algorithm` | resource | `sha256-canonical-json-v1` |
| `agent.smoke.contract_version` | resource | Report/telemetry contract version (`2`) |
| `agent.smoke.mode` | resource | `synthetic` or `live` |
| `agent.smoke.divergence_count` | resource, terminal batch | Number of case assertions not passing |

## Span attributes

| Attribute | Meaning |
|---|---|
| `agent.smoke.phase` | Span lifecycle phase |
| `agent.smoke.task_id` | Planned task identity |
| `agent.smoke.platform` | Planned harness platform |
| `agent.smoke.operation` | Fixture operation |
| `agent.smoke.model` | Configured model for the task |
| `agent.smoke.reasoning` | Configured reasoning level |
| `agent.smoke.model.planned` | Model specified by the plan |
| `agent.smoke.reasoning.planned` | Reasoning specified by the plan |
| `agent.smoke.expected_fault` | Expected injected fault, or `none` when explicitly applicable |
| `agent.smoke.observed_fault` | Observed fault when evidence exists |
| `agent.smoke.expected_outcome` | Plan-bound expected outcome |
| `agent.smoke.evaluation` | Case evaluation result |
| `agent.smoke.requirement_ref` | Requirement/specification reference |
| `agent.smoke.implementation_ref` | Implementation reference |
| `agent.smoke.specification_ref` | Specification reference |
| `agent.smoke.cases.planned` | Planned case count on terminal control span |
| `agent.smoke.cases.started` | Started case count on terminal control span |
| `agent.smoke.cases.ended` | Ended case count on terminal control span |
| `agent.smoke.cases.not_executed` | Cases not started on terminal control span |
| `agent.smoke.cases.not_executed_ids` | JSON array of task IDs that never started, bounded to 2 KiB |
| `agent.smoke.cases.passed` | Number of passing case assertions |
| `agent.smoke.cases.failed` | Number of failed case assertions |
| `agent.smoke.cases.inconclusive` | Number of inconclusive case assertions |

Configured model fields describe the plan. An observed provider model is not
emitted unless independently supplied by provider evidence. URLs, credentials,
prompts, raw stdout, and secrets are excluded. Text values are bounded to 2 KiB with an inline truncation marker. `observed_fault` is omitted for an UNSET case without fault evidence; an observed fault-free result uses `none`.
