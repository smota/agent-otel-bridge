# Community Benchmark Matrix & Leaderboard

This living document indexes verified hardware performance benchmarks submitted by the community using `agent-otel-bench --submit`.

---

## Verified Benchmarks

| Processor (CPU) | Cores | Operating System | RAM | Pipe RTT (p50) | Pipe RTT (p99) | Spawn (p50) | Spawn (p99) | Throughput | Verdict | Submission |
|---|---:|---|---:|---:|---:|---:|---:|---:|:---:|:---:|
| **AMD Ryzen AI 9 HX 470 w/ Radeon 890M** | 24 | Windows 11 Pro 25H2 (x86_64) | 94 GB | **23 µs** | **180 µs** | **4.47 ms** | **8.81 ms** | **565k spans/s** | ✅ PASS | Baseline (Reference) |

---

## Legend & Verification Standards

- **Named Pipe RTT**: Measured over 1,000 Overlapped I/O writes across Win32 Named Pipe (`\\.\pipe\agent-otel`). **SLA: < 3,000 µs (3 ms)**.
- **Process Spawn (`agent-hook.exe`)**: Measured over 250 real Windows process lifecycles (`CreateProcessW` → stdin JSON pipe → stdout `{}` write → exit code 0). **SLA: < 10,000 µs (10 ms)**.
- **Throughput**: In-memory ProtoJSON parsing and OTLP Protobuf span construction rate.
- **Verification Criteria**: All entries are automatically validated by the GitHub Actions bot (`benchmark_validator.yml`) against schema and latency thresholds before addition.

---

## How to Submit Your Hardware

To benchmark your workstation or server and have your results added to this table:

```powershell
# 1. Build release binaries
cargo build --release

# 2. Run automated benchmark & submission tool
cargo run --release -p agent-otel-bench -- --submit --open-browser
```

See the full [Community Benchmark Submission Guide](BENCHMARK_SUBMISSION.md) for details and guidelines.
