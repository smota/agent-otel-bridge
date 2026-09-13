# Changelog

All notable changes to `agent-otel-bridge` are documented in this file.
The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [0.5.0] - 2026-09-13

### Highlights
- **Decoupled Platform Provider Architecture**: Introduces contract-driven traits (`PlatformDescriptor`, `PlatformQuotaProvider`, `ClientAdapter`) completely isolating platform specifics from core logic.
- **3-Byte Wire Protocol Expansion (65,535 Platforms Support)**: Replaced the legacy 1-byte (4-bit, 15 platforms) nibble packing with a canonical 3-byte `WireHeader` (`[u8 event_id, u16 client_id]`), unlocking up to 65,535 AI agent platforms and 256 lifecycle events with zero hot-path overhead.
- **Automated Platform Conformance Suite**: Built-in static (`PlatformStaticValidator`) and dynamic (`PlatformDynamicValidator`) test suites executed via `cargo guardrails`, ensuring ID hygiene, zero collision, wire roundtrip parity, and finite quota snapshot invariants.
- **Machine-Adaptive Organic Quota Engine**: Discovers installed platforms dynamically on the host (`is_installed(&home)`) to completely prevent phantom gauges and metrics emission for uninstalled AI harnesses.
- **Dynamic Workspace Discovery**: CLI scanner crawls developer project directories using platform-declared `workspace_markers` rather than hardcoded heuristics.
- **FDE Fleet Operations & Observability**: Complete SigNoz dashboard suite for FDE fleet operations, developer identity harvesting, and tokenomics.
- **Comprehensive Architectural Documentation**: Added `docs/WHY_AGENT_OTEL_BRIDGE.md`, updated `docs/LIMITATIONS.md` removing the 15-platform ceiling, and expanded `docs/CLIENTS.md` with the Hermes integration blueprint.

### Added
- `crates/agent-otel-core/src/model.rs`: Added `WireHeader` with 3-byte memory layout (`encode` and `decode`), and wire mapping functions `HookEvent::from_wire`/`to_wire` and `ClientKind::from_wire`/`to_wire`.
- `crates/agent-otel-core/src/platform.rs`: Added `PlatformDescriptor` trait, built-in descriptors for Antigravity, Claude Code, Codex, Grok, and Pi, and `find_platform_by_wire_id(u16)`.
- `crates/agent-otel-core/src/validation.rs`: Added `PlatformStaticValidator` and `PlatformDynamicValidator` suites.
- `crates/agent-otel-core/tests/platform_conformance.rs`: 8 static and dynamic conformance tests, including `MockHermesPlatform`.
- `crates/agent-otel-daemon/src/platform.rs`: Added `PlatformQuotaProvider` trait.
- `crates/agent-otel-daemon/src/platforms/`: Modularized platform providers (`gemini.rs`, `claude.rs`, `codex.rs`, `grok.rs`, `pi.rs`, and registry in `mod.rs`).
- `crates/agent-otel-cli/src/hooks.rs`: Added `workspace_markers` declaration per adapter and support for `ClientTarget::Named(String)`.
- `dashboards/signoz/fleet-operations-fde.json`: Production-ready SigNoz dashboard for FDE fleet operations.
- `docs/WHY_AGENT_OTEL_BRIDGE.md`: Enterprise architectural decision guide comparing the bridge with native telemetry.

### Changed
- `crates/agent-otel-client/src/main.rs`: Migrated `agent-hook` to dispatch 3-byte `WireHeader` prefixed payloads; maintained static binary footprint at **145.5 KB** and execution time **< 1.0 ms**.
- `crates/agent-otel-daemon/src/quota.rs`: Refactored `QuotaEngine` to register only installed providers on the host (`detect_installed_providers(&home)`).
- `crates/agent-otel-cli/src/guardrails.rs`: Added step 4 to guardrails verifying platform contract conformance automatically.
- `docs/LIMITATIONS.md`: Removed 15-platform wire limit constraint and re-indexed limitations.
- `docs/CLIENTS.md`: Updated contract specifications, diagrams, and blueprints to reflect 16-bit wire client IDs.

---

## [0.4.0] - 2026-09-12

### Added
- **Cross-Platform Unix Domain Sockets**: Native non-blocking IPC for Linux and macOS via POSIX domain sockets (`agent-otel-ipc`).
- **Pure Rust Guardrail Tooling**: Integrated `cargo guardrails` and `agent-otel-bridge check-guardrails` replacing shell scripts with cross-platform verification.
- **Native CLI Instrumentation Guide**: Comprehensive architectural specification in `docs/NATIVE_CLI_INSTRUMENTATION.md`.
- **SigNoz Dashboard Visual Gallery**: Embedded dashboards and visual references in documentation.

### Changed
- Refined tokenomics and velocity dashboards in SigNoz.
- Harvest developer identity from Git config with machine fallback.

---

## [0.3.0] - 2026-09-12

### Added
- **Behavioral Execution Archetypes**: `FilterCompressor`, `StructuredParser`, `InspectorDiff`, `SearchRetrieval`, `BuildTestVerify`, `GenericExec`.
- **Multi-Tier Context Harvester**: Zero-process filesystem harvester for `.git/HEAD`, origin URLs, and branch metadata in $< 150\ \mu\text{s}$.
- **Cross-Agent W3C Tracing Propagation**: Environment propagation via `$env:TRACEPARENT` across nested agent lifecycles.
- **Tokenomics Dashboard**: Complete SigNoz JSON dashboard for token burn and compression analysis.

---

## [0.2.0] - 2026-09-11

### Added
- Multi-provider fleet quota tracking (Antigravity, Claude Code, Codex, Grok, Pi).
- English execution modes (`interactive`, `automation`).
- Claude Code session parser for dynamic token usage harvesting.

---

## [0.1.0] - 2026-09-11

### Added
- Initial release of `agent-otel-bridge`.
- Ultra-fast `agent-hook.exe` (< 1ms execution, < 300 KB binary size).
- Win32 Overlapped Named Pipe IPC server and client.
- OTLP Protobuf exporter for traces and metrics.
- Support for Google Antigravity, Anthropic Claude Code, OpenAI Codex, xAI Grok, and Pi (`pi.dev`).
