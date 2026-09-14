"""Development-only, three-attempt measurement ledger; refinement occurs between invocations."""
import argparse
import datetime
import json
import os
from pathlib import Path
import sys
import tempfile
import time
import uuid

from architecture_suite import inspect_installed_bridge, source_state_available
from perf_campaign import run_bounded_process
from perf_driver import compute_sha256, inspect_source_state

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
MAX_ATTEMPTS = 3


def reserve_attempt(ledger):
    if len(ledger.get("attempts", [])) >= MAX_ATTEMPTS:
        raise ValueError("three measurement attempts already reserved; no further attempt allowed")
    attempt = {"attempt": len(ledger["attempts"]) + 1, "run_id": uuid.uuid4().hex,
               "started_at_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
               "status": "running", "probes": {}}
    ledger["attempts"].append(attempt)
    return attempt


def store(path, value):
    staged = path.with_suffix(path.suffix + ".tmp")
    staged.write_text(json.dumps(value, indent=2), encoding="utf-8")
    staged.replace(path)


def probe(name, command, timeout, directory, index):
    started = time.monotonic()
    try:
        result = run_bounded_process(command, timeout, 16 * 1024 * 1024, cwd=str(ROOT))
    except Exception as exc:
        result = {"exit_code": None, "error": f"probe_launch_failed: {type(exc).__name__}",
                  "stdout": b"", "stderr": b"", "process_tree_cleanup": "not_started"}
    stdout = result.pop("stdout", b"")
    stderr = result.pop("stderr", b"")
    prefix = directory / f"attempt-{index}-{name}"
    prefix.with_suffix(".stdout.json").write_bytes(stdout)
    prefix.with_suffix(".stderr.txt").write_bytes(stderr)
    report = None
    try:
        report = json.loads(stdout)
    except (ValueError, UnicodeDecodeError):
        pass
    return {**result, "duration_seconds": time.monotonic() - started,
            "report_file": str(prefix.with_suffix(".stdout.json")), "json_valid": isinstance(report, dict),
            "reported_verdict": (report.get("verdict", report.get("overall_verdict"))
                                 if isinstance(report, dict) else None),
            "trace_id": report.get("trace_id") if isinstance(report, dict) else None}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--daemon-bin", type=Path, required=True)
    parser.add_argument("--hook-bin", type=Path, required=True)
    parser.add_argument("--campaign-dir", type=Path)
    parser.add_argument("--seed", type=int, default=42)
    parser.add_argument("--endpoint", default=os.getenv("OTEL_EXPORTER_OTLP_ENDPOINT", "http://127.0.0.1:4318"))
    args = parser.parse_args(argv)
    daemon = args.daemon_bin.resolve(strict=True)
    hook = args.hook_bin.resolve(strict=True)
    directory = (args.campaign_dir.resolve(strict=True) if args.campaign_dir
                 else Path(tempfile.mkdtemp(prefix="aob-load-loop-")))
    if directory == ROOT or ROOT in directory.parents:
        parser.error("campaign data must be outside the source repository")
    if not directory.is_dir():
        parser.error("campaign-dir must be a directory")
    lock = directory / "measurement.lock"
    try:
        descriptor = os.open(lock, os.O_CREAT | os.O_EXCL | os.O_WRONLY)
    except FileExistsError:
        parser.error("campaign already locked; inspect the owning process before recovery")
    os.write(descriptor, json.dumps({"pid": os.getpid(), "started": time.time()}).encode())
    os.close(descriptor)
    ledger_path = directory / "load-ledger.json"
    try:
        ledger = (json.loads(ledger_path.read_text()) if ledger_path.exists() else
                  {"schema": "agent-otel-load-campaign/v1", "campaign_id": uuid.uuid4().hex,
                   "max_attempts": MAX_ATTEMPTS, "seed": args.seed,
                   "coordination": {"author": "Antigravity Gemini 3.8 Flash high",
                                    "integration": "Codex GPT-5.6 Sol medium", "qa": "Codex root",
                                    "paid_models_in_measurement": False}, "attempts": []})
        if ledger.get("schema") != "agent-otel-load-campaign/v1" or ledger.get("seed") != args.seed:
            parser.error("incompatible ledger or changed seed")
        attempt = reserve_attempt(ledger)
        store(ledger_path, ledger)
        source_before = inspect_source_state(str(ROOT))
        active_before = inspect_installed_bridge()
        binaries = [daemon, hook, ROOT / "target/release/examples/performance.exe",
                    ROOT / "target/release/examples/performance_ipc.exe"]
        if os.name != "nt":
            binaries[2:] = [p.with_suffix("") for p in binaries[2:]]
        before = {str(p): compute_sha256(str(p)) for p in binaries}
        attempt["source_before"] = source_before
        attempt["candidate_hashes_before"] = before
        attempt["installed_before"] = active_before
        commands = [
            ("architecture", [sys.executable, str(HERE / "architecture_suite.py"), "--daemon-bin", str(daemon),
                              "--hook-bin", str(hook), "--seed", str(args.seed), "--repeats", "1"], 180),
            ("load", [sys.executable, str(HERE / "load_probe.py"), "--daemon-bin", str(daemon),
                      "--hook-bin", str(hook), "--seed", str(args.seed)], 180),
            ("micro", [sys.executable, str(HERE / "perf_driver.py"), "--example-bin", str(binaries[2]),
                       "--hook-bin", str(hook), "--repeats", "1", "--timeout", "60"], 90),
            ("ipc", [str(binaries[3])], 50),
            ("hook_internal", [sys.executable, str(HERE / "hook_timing_campaign.py"), "--daemon-bin", str(daemon),
                               "--hook-bin", str(hook), "--iterations", "100"], 60),
            ("long_trace", [sys.executable, str(HERE / "long_trace_probe.py"), "--daemon-bin", str(daemon),
                            "--hook-bin", str(hook), "--endpoint", args.endpoint,
                            "--duration", "60", "--events", "30"], 100),
        ]
        for name, command, timeout in commands:
            print(f"attempt {attempt['attempt']}: starting {name}", file=sys.stderr, flush=True)
            attempt["probes"][name] = probe(name, command, timeout, directory, attempt["attempt"])
            store(ledger_path, ledger)
        source_after = inspect_source_state(str(ROOT))
        active_after = inspect_installed_bridge()
        after = {str(p): compute_sha256(str(p)) for p in binaries}
        attempt.update({"source_after": source_after, "installed_after": active_after,
                        "candidate_hashes_after": after,
                        "integrity": {"source_unchanged": source_state_available(source_before) and source_before == source_after,
                                      "candidates_unchanged": all(before.values()) and before == after,
                                      "active_unchanged": bool(active_before.get("active_json_sha256")) and active_before == active_after},
                        "backend_visibility": {"status": "not_checked", "method": "separate SigNoz MCP exact trace query"},
                        "status": "measurement_complete", "finished_at_utc": datetime.datetime.now(datetime.timezone.utc).isoformat()})
        store(ledger_path, ledger)
        print(json.dumps({"campaign_id": ledger["campaign_id"], "ledger": str(ledger_path), "attempt": attempt}, indent=2))
        # Transport/process success is not an overall SLA or backend approval.
        return 0 if all(attempt["integrity"].values()) and all(
            r.get("exit_code") == 0 and r.get("json_valid") for r in attempt["probes"].values()) else 1
    finally:
        lock.unlink(missing_ok=True)


if __name__ == "__main__":
    raise SystemExit(main())
