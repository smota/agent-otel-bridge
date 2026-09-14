#!/usr/bin/env python3
"""Run the Windows inherited-HANDLE hook timing probe against an owned daemon.

The campaign requires explicit candidate binaries, a unique private pipe, and a
bounded loopback collector. It never discovers or contacts the active bridge.
"""

from __future__ import annotations

import argparse
import contextlib
import json
import os
import platform
import secrets
import subprocess
import sys
import threading
import time
from pathlib import Path
from typing import Any

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from native_context_probe import pipe_ready
from perf_campaign import MAX_CHILD_OUTPUT_BYTES, run_bounded_process
from perf_delivery import TraceCollector
from perf_driver import (
    collect_environment,
    compute_sha256,
    inspect_source_state,
    sanitize_for_json,
)

SCHEMA = "agent-otel-hook-timing-campaign/v1"
DEFAULT_ITERATIONS = 100
DEFAULT_CAMPAIGN_TIMEOUT_SECONDS = 90.0
DAEMON_READY_SECONDS = 10.0
DAEMON_STOP_SECONDS = 2.0
COLLECTOR_STOP_SECONDS = 2.0
DELIVERY_SETTLE_SECONDS = 2.0


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Measure a candidate Windows hook against an isolated candidate daemon"
    )
    parser.add_argument("--daemon-bin", type=Path, required=True)
    parser.add_argument("--hook-bin", type=Path, required=True)
    parser.add_argument("--iterations", type=int, default=DEFAULT_ITERATIONS)
    parser.add_argument(
        "--campaign-timeout-seconds",
        type=float,
        default=DEFAULT_CAMPAIGN_TIMEOUT_SECONDS,
    )
    return parser.parse_args(argv)


def basic_environment(system_name: str) -> dict[str, Any]:
    """Platform facts which never launch a subprocess."""
    return {
        "os": system_name,
        "os_name": os.name,
        "sys_platform": sys.platform,
        "cpu_count": os.cpu_count(),
        "python_version": platform.python_version(),
    }


def candidate_provenance(path: Path) -> dict[str, Any]:
    return {
        "path": str(path.resolve()),
        "exists": path.is_file(),
        "sha256": compute_sha256(str(path)),
    }


def not_measured_report(args: argparse.Namespace, system_name: str, reason: str) -> dict[str, Any]:
    return {
        "schema": SCHEMA,
        "verdict": "not_measured",
        "reason": reason,
        "measurement": {
            "boundary": "hook_internal_inherited_handle",
            "platform_scope": "windows",
            "iterations": args.iterations,
            "probe": None,
        },
        "environment": basic_environment(system_name),
        "candidate": {
            "daemon": candidate_provenance(args.daemon_bin),
            "hook": candidate_provenance(args.hook_bin),
        },
        "isolation": {
            "active_install_touched": False,
            "child_processes_started": False,
            "pipe": None,
            "collector": None,
        },
        "cleanup": {"required": False, "complete": True},
    }


def evaluate_probe(
    process: dict[str, Any],
    *,
    expected_iterations: int | None = None,
    expected_hook: Path | None = None,
    expected_hook_sha256: str | None = None,
) -> tuple[str, str | None, dict[str, Any] | None]:
    if process.get("error"):
        return "not_measured", str(process["error"]), None
    raw = process.get("stdout", b"")
    if not isinstance(raw, bytes):
        return "not_measured", "probe_stdout_invalid", None
    try:
        report = json.loads(raw.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        return "not_measured", f"probe_json_invalid: {exc}", None
    if not isinstance(report, dict):
        return "not_measured", "probe_report_invalid", None
    if report.get("schema") != "agent-otel-hook-timing/v1":
        return "not_measured", "probe_schema_invalid", report
    if expected_iterations is not None and report.get("iterations") != expected_iterations:
        return "not_measured", "probe_iterations_mismatch", report
    if expected_hook is not None and report.get("hook_bin") != str(expected_hook.resolve()):
        return "not_measured", "probe_hook_mismatch", report
    if expected_hook_sha256 is not None and report.get("hook_sha256") != expected_hook_sha256:
        return "not_measured", "probe_hook_hash_mismatch", report

    iterations = report.get("iterations")
    samples = report.get("samples")
    if (
        type(iterations) is not int
        or not isinstance(samples, list)
        or len(samples) != iterations
        or report.get("valid_samples") != iterations
        or report.get("missing_or_invalid") != 0
    ):
        return "not_measured", "probe_observer_samples_incomplete", report

    work_ns: list[int] = []
    for sample in samples:
        if not isinstance(sample, dict) or sample.get("error") is not None:
            return "not_measured", "probe_observer_samples_incomplete", report
        record = sample.get("record")
        if not isinstance(record, dict) or record.get("send_completed") is not True:
            return "not_measured", "probe_send_observation_incomplete", report
        value = record.get("work_completed_ns")
        if type(value) is not int or value < 0:
            return "not_measured", "probe_work_duration_invalid", report
        work_ns.append(value)

    disabled = report.get("disabled_comparison")
    disabled_samples = disabled.get("samples") if isinstance(disabled, dict) else None
    if (
        not isinstance(disabled, dict)
        or disabled.get("iterations") != iterations
        or disabled.get("failures") != 0
        or not isinstance(disabled_samples, list)
        or len(disabled_samples) != iterations
        or any(
            not isinstance(sample, dict) or sample.get("error") is not None
            for sample in disabled_samples
        )
    ):
        return "not_measured", "probe_disabled_comparison_incomplete", report

    if any(value >= 1_000_000 for value in work_ns):
        return "failed", "hook_timing_assertion_failed", report
    if process.get("exit_code") == 0 and report.get("all_below_1000us") is True:
        return "passed", None, report
    return "not_measured", "probe_result_inconsistent", report


@contextlib.contextmanager
def cleared_watchdog_environment():
    inherited = os.environ.pop("AGENT_OTEL_WATCHDOG_MS", None)
    try:
        yield
    finally:
        if inherited is not None:
            os.environ["AGENT_OTEL_WATCHDOG_MS"] = inherited


def collector_snapshot(collector: TraceCollector) -> tuple[list[tuple[str, str]], list[str]]:
    with collector.lock:
        return list(collector.received_spans), list(collector.collector_errors)


def stop_daemon(process: subprocess.Popen[bytes] | None) -> dict[str, Any]:
    if process is None:
        return {"started": False, "terminated": True, "killed": False, "exit_code": None}
    killed = False
    try:
        if process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=DAEMON_STOP_SECONDS)
            except subprocess.TimeoutExpired:
                killed = True
                process.kill()
                process.wait(timeout=DAEMON_STOP_SECONDS)
        return {
            "started": True,
            "terminated": process.poll() is not None,
            "killed": killed,
            "exit_code": process.returncode,
        }
    except (OSError, subprocess.TimeoutExpired) as exc:
        return {
            "started": True,
            "terminated": process.poll() is not None,
            "killed": killed,
            "exit_code": process.returncode,
            "error": str(exc),
        }


def stop_collector(
    collector: TraceCollector | None, thread: threading.Thread | None
) -> dict[str, Any]:
    if collector is None:
        return {"started": False, "stopped": True}
    error = None
    try:
        collector.shutdown()
        collector.server_close()
    except OSError as exc:
        error = str(exc)
    if thread is not None:
        thread.join(timeout=COLLECTOR_STOP_SECONDS)
    stopped = thread is None or not thread.is_alive()
    result: dict[str, Any] = {"started": True, "stopped": stopped}
    if error:
        result["error"] = error
    return result


def run_campaign(
    args: argparse.Namespace, system_name: str | None = None
) -> dict[str, Any]:
    system_name = system_name or platform.system()
    if system_name != "Windows":
        return not_measured_report(args, system_name, "windows_inherited_handle_unavailable")
    if not 1 <= args.iterations <= 1_000:
        return not_measured_report(args, system_name, "iterations_out_of_range_1_1000")
    if not 5.0 <= args.campaign_timeout_seconds <= 300.0:
        return not_measured_report(args, system_name, "campaign_timeout_out_of_range_5_300")
    if not args.daemon_bin.is_file() or not args.hook_bin.is_file():
        return not_measured_report(args, system_name, "candidate_binary_missing")

    started = time.monotonic()
    deadline = started + args.campaign_timeout_seconds
    pipe_name = rf"\\.\pipe\agent-otel-hook-timing-{os.getpid()}-{secrets.token_hex(6)}"
    collector: TraceCollector | None = None
    collector_thread: threading.Thread | None = None
    daemon: subprocess.Popen[bytes] | None = None
    probe_process: dict[str, Any] | None = None
    probe_report: dict[str, Any] | None = None
    reason: str | None = None
    verdict = "not_measured"

    try:
        collector = TraceCollector(("127.0.0.1", 0))
        collector_thread = threading.Thread(
            target=collector.serve_forever,
            name="hook-timing-collector",
            daemon=True,
        )
        collector_thread.start()
        collector_endpoint = f"http://127.0.0.1:{collector.server_address[1]}"

        daemon_environment = os.environ.copy()
        daemon_environment.pop("TRACEPARENT", None)
        daemon_environment.pop("TRACESTATE", None)
        daemon_environment.pop("AGENT_OTEL_WATCHDOG_MS", None)
        daemon_environment.update(
            {
                "AGENT_OTEL_PIPE": pipe_name,
                "AGY_OTEL_PIPE": pipe_name,
                "AGENT_OTEL_SOCKET": pipe_name,
                "OTEL_EXPORTER_OTLP_ENDPOINT": collector_endpoint,
                "OTEL_SERVICE_NAME": "agent-otel-hook-timing-candidate",
                "AGENT_OTEL_IDLE_TIMEOUT_SECS": "30",
            }
        )
        daemon = subprocess.Popen(
            [str(args.daemon_bin.resolve()), "daemon"],
            env=daemon_environment,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            shell=False,
            creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
        )
        ready_deadline = min(deadline, time.monotonic() + DAEMON_READY_SECONDS)
        if not pipe_ready(pipe_name, ready_deadline):
            reason = "candidate_pipe_readiness_timeout"
        elif daemon.poll() is not None:
            reason = f"candidate_daemon_exited:{daemon.returncode}"
        else:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                reason = "campaign_deadline_before_probe"
            else:
                command = [
                    sys.executable,
                    str(Path(__file__).with_name("hook_timing_probe.py")),
                    "--hook-bin",
                    str(args.hook_bin.resolve()),
                    "--pipe-name",
                    pipe_name,
                    "--iterations",
                    str(args.iterations),
                ]
                with cleared_watchdog_environment():
                    probe_process = run_bounded_process(
                        command,
                        timeout_sec=remaining,
                        output_limit=MAX_CHILD_OUTPUT_BYTES,
                    )
                verdict, reason, probe_report = evaluate_probe(
                    probe_process,
                    expected_iterations=args.iterations,
                    expected_hook=args.hook_bin,
                    expected_hook_sha256=compute_sha256(str(args.hook_bin)),
                )

                settle_deadline = min(deadline, time.monotonic() + DELIVERY_SETTLE_SECONDS)
                received_spans, collector_errors = collector_snapshot(collector)
                while not received_spans and not collector_errors and time.monotonic() < settle_deadline:
                    time.sleep(0.02)
                    received_spans, collector_errors = collector_snapshot(collector)
    except OSError as exc:
        verdict = "not_measured"
        reason = f"campaign_os_error: {exc}"
    finally:
        daemon_cleanup = stop_daemon(daemon)
        collector_cleanup = stop_collector(collector, collector_thread)

    cleanup_complete = bool(
        daemon_cleanup.get("terminated") and collector_cleanup.get("stopped")
    )
    if not cleanup_complete:
        verdict = "failed"
        reason = reason or "owned_resource_cleanup_failed"

    process_summary = None
    if probe_process is not None:
        process_summary = {
            key: value
            for key, value in probe_process.items()
            if key not in {"stdout", "stderr"}
        }
        process_summary["stderr"] = probe_process.get("stderr", b"").decode(
            "utf-8", errors="replace"
        )[-4_000:]

    received_spans, collector_errors = (
        collector_snapshot(collector) if collector is not None else ([], [])
    )
    delivery_observation = {
        "scope": "observational_only",
        "status": (
            "invalid"
            if collector_errors
            else "observed"
            if received_spans
            else "not_observed"
        ),
        "received_span_count": len(received_spans),
        "claims_all_iterations_delivered": False,
        "errors": collector_errors,
    }
    assertion = {
        "id": "hook_internal_under_1000us",
        "boundary": "candidate_hook_internal_inherited_handle",
        "operator": "strictly_less",
        "threshold": 1_000_000,
        "unit": "ns",
        "observed_max": (probe_report or {}).get("work_ns", {}).get("max"),
        "status": verdict,
        "reason": reason,
    }
    return sanitize_for_json(
        {
            "schema": SCHEMA,
            "verdict": verdict,
            "reason": reason,
            "elapsed_seconds": time.monotonic() - started,
            "environment": collect_environment(),
            "source": inspect_source_state(),
            "candidate": {
                "daemon": candidate_provenance(args.daemon_bin),
                "hook": candidate_provenance(args.hook_bin),
            },
            "measurement": {
                "boundary": "hook_internal_inherited_handle",
                "platform_scope": "windows",
                "iterations": args.iterations,
                "assertion": assertion,
                "probe_process": process_summary,
                "probe": probe_report,
                "delivery_observation": delivery_observation,
            },
            "isolation": {
                "active_install_touched": False,
                "child_processes_started": True,
                "pipe": {"name": pipe_name, "owned": True},
                    "collector": {
                    "endpoint": (
                        f"http://127.0.0.1:{collector.server_address[1]}"
                        if collector is not None
                        else None
                    ),
                    "received_span_count": len(received_spans),
                    "errors": collector_errors,
                    "delivery_scope": "observational_only",
                    "claims_all_iterations_delivered": False,
                    "bounded_body_bytes": 2 * 1024 * 1024,
                    "bounded_span_count": 1_000,
                },
            },
            "cleanup": {
                "required": True,
                "complete": cleanup_complete,
                "daemon": daemon_cleanup,
                "collector": collector_cleanup,
            },
        }
    )


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    report = run_campaign(args)
    json.dump(report, sys.stdout, indent=2)
    sys.stdout.write("\n")
    verdict = report.get("verdict")
    return 0 if verdict == "passed" else 1 if verdict == "failed" else 3


if __name__ == "__main__":
    raise SystemExit(main())
