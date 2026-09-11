# Community Benchmark Submission Guide

`agent-otel-bridge` is engineered for ultra-low latency (< 3ms hot-path SLA). We invite developers and operators across different CPU architectures, OS releases, and virtualization environments to benchmark and contribute their hardware performance figures to the **Community Benchmark Registry**.

---

## 1. Why Benchmark?

AI CLI coding agents invoke lifecycle hooks dozens to hundreds of times per pairing session. While traditional scripting languages (PowerShell, Python, Node.js) take **150ms to 1,200ms per tool invocation**, `agent-otel-bridge` guarantees:
- **Win32 Named Pipe RTT**: **< 100 µs** (SLA < 3,000 µs)
- **OTLP Span Construction**: **< 2 µs** (Throughput > 500,000 spans/sec)
- **Full Process Lifecycle (`agent-hook.exe`)**: **< 6 ms** (SLA < 10.0 ms)

By submitting your hardware measurements, you help the OpenTelemetry and AI agent communities verify cross-hardware performance invariants across Windows 10, Windows 11, Windows Server, AMD Ryzen, Intel Core, and ARM64.

---

## 2. Running Benchmarks

Ensure your binaries are built in release mode:
```powershell
cargo build --release
```

### Option A: 1-Click Automated Submission (Recommended)
Run:
```powershell
cargo run --release -p agent-otel-bench -- --submit --open-browser
```
Or with the unified CLI:
```powershell
agent-otel-bridge benchmark --submit --open-browser
```

**What happens automatically:**
1. Collects system specifications (CPU brand, logical core count, OS version/build, RAM, Rust compiler version).
2. Executes 1,000 pipe roundtrips, 10,000 span parses, and 250 real process spawns.
3. Saves a local JSON report to `benchmarks/reports/benchmark-<cpu>-<timestamp>.json`.
4. **If GitHub CLI (`gh`) is authenticated**: Prompts you to create the issue automatically on GitHub.
5. **Otherwise**: Launches your default browser with a pre-filled GitHub Issue Form containing your hardware specs and URL-encoded details.

---

### Option B: Machine-Readable JSON Export
To export JSON for CI, scripts, or manual inspection:
```powershell
cargo run --release -p agent-otel-bench -- --json --export my_benchmark.json
```

---

## 3. Automated Bot Verification

When you submit a benchmark issue:
1. Our GitHub Actions bot (`benchmark_validator.yml`) triggers automatically.
2. It parses the benchmark JSON payload and verifies that your measurements satisfy latency SLAs:
   - Named Pipe p99 < 3,000 µs
   - Process Spawn p99 < 10,000 µs
   - Span Throughput > 50,000 spans/sec
3. The bot adds an automated validation report comment with status badges:
   ```markdown
   ### 🤖 Automated Benchmark Validation Result
   | Measurement Stage | Value | SLA Target | Status |
   |---|---:|---:|:---:|
   | Named Pipe RTT (p50) | 23 µs | < 3,000 µs | ✅ PASS |
   | Process Spawn (p50) | 4.47 ms | < 10.0 ms | ✅ PASS |
   ```
4. The issue receives the `benchmark-verified` label and is merged into [`docs/COMMUNITY_BENCHMARKS.md`](COMMUNITY_BENCHMARKS.md).

---

## 4. Guidelines for Reliable Benchmarking

To ensure reproducible, high-quality results:
1. **Idle System**: Close heavy games, video rendering, or active Docker workloads before running the benchmark.
2. **Laptops & Power Plan**: Plug laptops into AC power and select the **"Best Performance"** power plan in Windows Settings.
3. **Warm-up**: The benchmark suite includes automatic warm-up iterations, but running the benchmark twice and submitting the second result helps avoid cold file-system caching variance.
