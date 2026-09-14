"""Current acceptance controller; historical mixed benchmarks are not gates."""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import sys
import time
import uuid

sys.path.insert(0, str(Path(__file__).resolve().parent))
import architecture_suite as architecture
from perf_campaign import run_subprocess_json
from perf_driver import collect_environment, inspect_source_state

SCHEMA = "agent-otel-production-round/v1"
BENCH_SCHEMA = "agent-otel-production-path/v1"
BENCH_SEED = 0x5EED_2026_0914
BENCH_ITERATIONS = 50_000
LOOKUP_SAMPLES = 10_000
IPC_TIMED_ROUNDS = 1_000
IPC_WARMUP_ROUNDS = 20
IPC_P99_LIMIT_US = 3_000.0
MODEL_ROLES = {
    "author_initial_assignment": "agy/gemini-3.8-flash-low",
    "author_actual_execution": "agy/gemini-3.8-flash-medium relay escalation",
    "integration": "gpt-5.6-sol",
    "adapters": "gpt-5.6-luna",
    "acceptance": "Codex root",
}
MAX_ATTEMPTS = 3
HERE = Path(__file__).resolve().parent


def sha256(path):
    try:
        with open(path, "rb") as stream:
            return hashlib.file_digest(stream, "sha256").hexdigest()
    except OSError:
        return None


def finite(value, positive=False):
    return (type(value) in (int, float) and math.isfinite(value)
            and (value > 0 if positive else value >= 0))


def validate_rate(value):
    if not isinstance(value, dict):
        return False
    count, elapsed, rate = (value.get(k) for k in ("iterations", "elapsed_secs", "ops_per_sec"))
    return (type(count) is int and count > 0 and finite(elapsed, True)
            and finite(rate, True) and math.isclose(rate, count / elapsed, rel_tol=1e-6))


def validate_latency(value):
    if not isinstance(value, dict):
        return False
    distribution = value.get("distribution", value)
    if not isinstance(distribution, dict) or type(distribution.get("count")) is not int or distribution["count"] <= 0:
        return False
    values = [distribution.get(key) for key in ("p50_us", "p95_us", "p99_us", "max_us")]
    if not all(finite(v) for v in values) or values != sorted(values):
        return False
    if "distribution" in value:
        counts = [v for k, v in value.items() if k.endswith("_count")]
        if not counts or not all(type(n) is int and n >= 0 for n in counts) or sum(counts) != distribution["count"]:
            return False
    return True


def validate_benchmark(report):
    transform = report.get("production_transform", {}) if isinstance(report, dict) else {}
    boundary = transform.get("boundary", "")
    boundary_parts = ("decode_frame", "parse_slice", "context_cache_lookup", "normalize",
                      "build_span_from_resolved", "excludes quota", "clock", "batch", "export")
    return (isinstance(report, dict) and report.get("schema") == BENCH_SCHEMA
            and report.get("profile") == "release"
            and type(report.get("seed")) is int and report["seed"] == BENCH_SEED
            and type(report.get("warmup_iterations")) is int
            and report["warmup_iterations"] == 2_000
            and report.get("corpus_items") == 4
            and report.get("corpus_identity") == "deterministic_mixed_v1_0x01_0x04_tokens_ws"
            and report.get("semantic_assert_passed") is True
            and report.get("gate_parser_gt_50k_passed") is True
            and validate_rate(report.get("parser_only"))
            and report["parser_only"]["iterations"] == BENCH_ITERATIONS
            and report["parser_only"]["ops_per_sec"] > 50000
            and validate_rate(transform)
            and transform["iterations"] == BENCH_ITERATIONS
            and isinstance(boundary, str)
            and all(part in boundary for part in boundary_parts)
            and all(validate_latency(report.get(k)) for k in ("lookup_hit", "lookup_miss", "lookup_stale"))
            and all(report[k].get("distribution", report[k]).get("count") == LOOKUP_SAMPLES
                    for k in ("lookup_hit", "lookup_miss", "lookup_stale"))
            and isinstance(report.get("boundary_notes"), str)
            and bool(report["boundary_notes"].strip()))


def validate_ipc_benchmark(report):
    if not isinstance(report, dict) or report.get("overall_passed") is not True:
        return False
    if (report.get("conversation_id") != "performance-fixture"
            or report.get("latency_label") != "client_send_through_receipt_one_way_observer_micros"
            or not isinstance(report.get("pipe_name"), str) or not report["pipe_name"]
            or not isinstance(report.get("normative_ref"), str) or not report["normative_ref"]
            or not isinstance(report.get("impl_ref"), str) or not report["impl_ref"]
            or report.get("not_measured") != ["hook_internal_execution_duration_us"]):
        return False
    results = report.get("results")
    if not isinstance(results, list) or len(results) != 2:
        return False
    expected_types = {"HookPayload", "HookPayloadWithContext"}
    if {item.get("msg_type") for item in results if isinstance(item, dict)} != expected_types:
        return False
    for item in results:
        if not isinstance(item, dict):
            return False
        stats = item.get("latency_stats")
        if (item.get("expected_rounds") != IPC_TIMED_ROUNDS
                or item.get("warmup_rounds") != IPC_WARMUP_ROUNDS
                or item.get("success_count") != IPC_TIMED_ROUNDS
                or item.get("failure_count") != 0
                or item.get("p99_sla_passed") is not True
                or not isinstance(stats, dict) or stats.get("count") != IPC_TIMED_ROUNDS):
            return False
        values = [stats.get(key) for key in ("min_us", "p50_us", "p95_us", "p99_us", "max_us")]
        if (not all(finite(value) for value in values)
                or values != sorted(values)
                or not finite(stats.get("mean_us"))
                or not values[0] <= stats["mean_us"] <= values[-1]
                or stats["p99_us"] >= IPC_P99_LIMIT_US):
            return False
    return True


def validate_architecture(report):
    if not isinstance(report, dict) or report.get("schema") != "agent-otel-new-architecture/v1":
        return False
    scenarios = report.get("scenarios", [])
    required = {"cold_cache", "warm_cache", "multi_workspace", "stale_refresh", "exporter_blocked", "concurrent_delivery"}
    observed = {s.get("id", s.get("scenario_id")) for s in scenarios}
    return (report.get("overall_verdict") == "passed" and observed == required
            and len(scenarios) == len(required)
            and all(s.get("verdict") == "passed" for s in scenarios)
            and all(report.get("integrity_gates", {}).get(k) is True for k in
                    ("source_intact", "candidate_binaries_intact", "installed_bridge_intact")))


def reserve(path, attempt):
    path = Path(path)
    path.mkdir(parents=True, exist_ok=True)
    if not 1 <= attempt <= MAX_ATTEMPTS:
        raise ValueError("attempt outside 1..3")
    # Directory creation is atomic across controllers; interrupted attempts stay consumed.
    reservation = path / f"attempt-{attempt}.reserved"
    if attempt > 1 and not (path / f"attempt-{attempt-1}.reserved").exists():
        raise ValueError("attempts must be sequential")
    try:
        reservation.mkdir()
    except FileExistsError as exc:
        raise ValueError("attempt already consumed") from exc
    record = {"attempt": attempt, "attempt_id": uuid.uuid4().hex, "reserved_at_ms": int(time.time()*1000)}
    (reservation / "reservation.json").write_text(json.dumps(record), encoding="utf-8")
    return record


def snapshot(paths):
    home = Path.home()
    hooks = [home / p for p in (".gemini/config/hooks.json", ".claude/settings.json", ".codex/hooks.json", ".grok/hooks/agent-otel.json", ".pi/hooks.json")]
    return {"source": inspect_source_state(), "active": architecture.inspect_installed_bridge(),
            "hooks": {str(p): sha256(p) for p in hooks},
            "binaries": {name: sha256(path) for name, path in paths.items() if path}}


def snapshot_valid(value):
    hashes = value.get("binaries", {})
    return (architecture.source_state_available(value.get("source", {}))
            and architecture.installed_bridge_available(value.get("active", {}))
            and bool(hashes) and all(architecture._valid_sha(h) for h in hashes.values()))


def run_probe(name, argv, output, deadline, limit=180):
    remaining = min(limit, deadline-time.monotonic())
    if remaining <= 0:
        return {"name": name, "argv": argv, "error": "campaign_deadline", "data": None,
                "exit_code": None, "process_tree_cleanup": "not_started"}
    # Real-hook reports retain per-event spans and diagnostics for four profiles.
    # Keep a finite cap sized for the declared workload, not the 4 MiB microprobe default.
    result = run_subprocess_json(argv, remaining, output_limit=32 * 1024 * 1024)
    result.update(name=name, argv=argv)
    (output / f"{name}.json").write_text(json.dumps(result, indent=2, allow_nan=False), encoding="utf-8")
    return result


def process_ok(result):
    return (result.get("exit_code") == 0 and not result.get("error")
            and result.get("process_tree_cleanup") == "complete" and result.get("output_bounded") is True)


def verdict(ok):
    return "passed" if ok else "failed"


def validate_report(report):
    return (isinstance(report, dict) and report.get("schema") == SCHEMA
            and bool(report.get("attempt_id")) and bool(report.get("assertions"))
            and all(a.get("verdict") in ("passed", "failed", "not_measured") for a in report["assertions"]))


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--attempt", type=int, required=True)
    parser.add_argument("--daemon-bin", required=True)
    parser.add_argument("--hook-bin", required=True)
    parser.add_argument("--production-bench", required=True)
    parser.add_argument("--ipc-bench")
    parser.add_argument("--ipc-load")
    parser.add_argument("--seed", type=int, default=502)
    args = parser.parse_args(argv)
    record = reserve(args.output_dir, args.attempt)
    output = args.output_dir / f"attempt-{args.attempt}.reserved"
    paths = {name: str(Path(value).resolve()) for name, value in vars(args).items()
             if name in ("daemon_bin", "hook_bin", "production_bench", "ipc_bench", "ipc_load") and value}
    before = snapshot(paths)
    started = time.monotonic()
    deadline = started + 900
    probes, assertions = {}, []

    def assertion(identifier, passed, reference, required=True):
        assertions.append({"id": identifier, "verdict": verdict(passed), "required": required,
                           "normative_ref": "production-path-round.md", "implementation_ref": reference})

    assertion("snapshot_available", snapshot_valid(before), "snapshot")
    if snapshot_valid(before):
        probes["production"] = run_probe("production", [paths["production_bench"]], output, deadline, 90)
        assertion("parser_and_production_measurement", process_ok(probes["production"]) and validate_benchmark(probes["production"].get("data")), "examples/production_path.rs")
        probes["architecture"] = run_probe("architecture", [sys.executable, str(HERE/"architecture_suite.py"), "--daemon-bin", paths["daemon_bin"], "--hook-bin", paths["hook_bin"], "--repeats", "1", "--seed", str(args.seed)], output, deadline)
        assertion("architecture_scenarios", process_ok(probes["architecture"]) and validate_architecture(probes["architecture"].get("data")), "architecture_suite.py")
        probes["load"] = run_probe("load", [sys.executable, str(HERE/"load_probe.py"), "--daemon-bin", paths["daemon_bin"], "--hook-bin", paths["hook_bin"], "--levels", "1,4,8,16", "--seed", str(args.seed)], output, deadline)
        load = probes["load"].get("data") or {}
        assertion("real_hook_delivery", process_ok(probes["load"]) and load.get("schema") == "agent-otel-load/v1" and load.get("verdict") == "passed" and len(load.get("profiles", [])) == 4, "load_probe.py")
        probes["hook"] = run_probe("hook", [sys.executable, str(HERE/"hook_timing_campaign.py"), "--daemon-bin", paths["daemon_bin"], "--hook-bin", paths["hook_bin"], "--iterations", "20"], output, deadline)
        assertion("internal_hook_latency", process_ok(probes["hook"]) and (probes["hook"].get("data") or {}).get("verdict") == "passed", "hook_timing_campaign.py")
        if "ipc_bench" in paths:
            probes["ipc"] = run_probe("ipc", [paths["ipc_bench"]], output, deadline, 90)
            data = probes["ipc"].get("data") or {}
            assertion("ipc_latency", process_ok(probes["ipc"]) and validate_ipc_benchmark(data), "examples/performance_ipc.rs")
        else:
            assertions.append({"id": "ipc_latency", "required": True, "verdict": "not_measured"})
        if "ipc_load" in paths:
            probes["ipc_load"] = run_probe("ipc_load", [sys.executable, str(HERE/"production_ipc_probe.py"), "--daemon-bin", paths["daemon_bin"], "--emitter", paths["ipc_load"], "--seed", str(args.seed)], output, deadline)
            assertion("ipc_delivery", process_ok(probes["ipc_load"]) and (probes["ipc_load"].get("data") or {}).get("verdict") == "passed", "production_ipc_probe.py")
        else:
            assertions.append({"id": "ipc_delivery", "required": True, "verdict": "not_measured"})

    after = snapshot(paths)
    intact = snapshot_valid(after) and before == after
    assertion("source_active_binary_integrity", intact, "snapshot")
    size = Path(paths["hook_bin"]).stat().st_size if Path(paths["hook_bin"]).is_file() else None
    assertion("hook_size", size is not None and size < 300000, "agent-otel-client")
    required_pass = all(a["verdict"] == "passed" for a in assertions if a["required"])
    cleanup = all(p.get("process_tree_cleanup") == "complete" for p in probes.values())
    report = {"schema": SCHEMA, **record, "ended_at_ms": int(time.time()*1000), "seed": args.seed,
              "environment": collect_environment(), "snapshots": {"before": before, "after": after},
              "model_roles": dict(MODEL_ROLES),
              "functional_verdict": verdict(required_pass and cleanup),
              "capacity_certification": "not_measured", "assertions": assertions,
              "hook_size_bytes": size, "probes": probes, "duration_seconds": time.monotonic()-started,
              "cleanup": {"processes_drained": cleanup},
              "unmeasured": ["allocator_profile", "queue_latency", "native_linux_macos_performance", "paid_fleet_capacity"],
              "backend_trace": {"verdict": "not_measured", "reason": "separate long_trace_probe and MCP acceptance required"}}
    (output/"report.json").write_text(json.dumps(report, indent=2, allow_nan=False), encoding="utf-8")
    print(json.dumps({"report": str(output/"report.json"), "functional_verdict": report["functional_verdict"], "assertions": assertions}))
    return 0 if required_pass and cleanup and validate_report(report) else 1


if __name__ == "__main__":
    raise SystemExit(main())
