#!/usr/bin/env python3
"""Measure agent-hook internal work through its inherited-handle observer.

The observer record is emitted by the exact candidate binary after the measured
region. Missing or malformed records remain explicit and prevent approval.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import pathlib
import struct
import subprocess
import sys
import threading
import time
from dataclasses import dataclass
from typing import Any


RECORD = struct.Struct("<4sHHIIQQQ")
MAGIC = b"AOBT"
VERSION = 1
NORMAL_COMPLETION = 1 << 0
SEND_COMPLETED = 1 << 1


@dataclass(frozen=True)
class TimingRecord:
    pid: int
    flags: int
    response_completed_ns: int
    before_transport_ns: int
    work_completed_ns: int

    @classmethod
    def decode(cls, data: bytes, expected_pid: int) -> "TimingRecord":
        if len(data) != RECORD.size:
            raise ValueError(f"record length {len(data)} != {RECORD.size}")
        magic, version, size, pid, flags, response, transport, work = RECORD.unpack(data)
        if magic != MAGIC or version != VERSION or size != RECORD.size:
            raise ValueError("invalid observer magic, version, or size")
        if pid != expected_pid:
            raise ValueError(f"record pid {pid} != child pid {expected_pid}")
        if flags & ~(NORMAL_COMPLETION | SEND_COMPLETED):
            raise ValueError(f"unknown observer flags 0x{flags:08x}")
        if not flags & NORMAL_COMPLETION:
            raise ValueError("record does not describe normal completion")
        if not response <= transport <= work:
            raise ValueError("observer checkpoints are not monotonic")
        return cls(pid, flags, response, transport, work)


def _read_all(fd: int, destination: list[bytes], errors: list[str]) -> None:
    chunks: list[bytes] = []
    try:
        while True:
            chunk = os.read(fd, RECORD.size + 1)
            if not chunk:
                break
            chunks.append(chunk)
            if sum(map(len, chunks)) > RECORD.size:
                break
        destination.append(b"".join(chunks))
    except OSError as exc:
        errors.append(str(exc))
    finally:
        os.close(fd)


def run_once(args: argparse.Namespace, index: int) -> dict[str, Any]:
    if os.name != "nt":
        raise RuntimeError("the inherited HANDLE observer is Windows-only")
    import msvcrt  # pylint: disable=import-outside-toplevel

    read_fd, write_fd = os.pipe()
    os.set_inheritable(write_fd, True)
    observer_handle = msvcrt.get_osfhandle(write_fd)
    startupinfo = subprocess.STARTUPINFO()
    startupinfo.lpAttributeList = {"handle_list": [observer_handle]}

    captured: list[bytes] = []
    read_errors: list[str] = []
    reader = threading.Thread(
        target=_read_all, args=(read_fd, captured, read_errors), daemon=True
    )
    reader.start()

    environment = os.environ.copy()
    environment["AGENT_OTEL_BENCH_HANDLE"] = str(observer_handle)
    if args.pipe_name:
        environment["AGENT_OTEL_PIPE"] = args.pipe_name
    if args.watchdog_ms is not None:
        environment["AGENT_OTEL_WATCHDOG_MS"] = str(args.watchdog_ms)

    command = [str(args.hook_bin), args.event, "--client", args.client]
    started_ns = time.perf_counter_ns()
    child = subprocess.Popen(
        command,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        env=environment,
        startupinfo=startupinfo,
        close_fds=True,
    )
    os.close(write_fd)
    timed_out = False
    try:
        stdout, stderr = child.communicate(args.payload, timeout=args.timeout_seconds)
    except subprocess.TimeoutExpired:
        timed_out = True
        child.kill()
        stdout, stderr = child.communicate()
    external_ns = time.perf_counter_ns() - started_ns
    reader.join(timeout=args.timeout_seconds)

    error: str | None = None
    record: TimingRecord | None = None
    if reader.is_alive():
        error = "observer reader did not reach EOF"
    elif read_errors:
        error = f"observer read failed: {read_errors[0]}"
    elif not captured or not captured[0]:
        error = "observer record missing"
    else:
        try:
            record = TimingRecord.decode(captured[0], child.pid)
        except ValueError as exc:
            error = str(exc)

    expected_stdout = (
        b'{"decision":"allow"}'
        if args.event == "PreToolUse" and args.client.lower() not in {"codex", "openai", "codex-cli"}
        else b"{}"
    )
    if stdout != expected_stdout:
        error = error or f"unexpected stdout {stdout!r}"
    if timed_out:
        error = error or "external containment timeout"
    if child.returncode != 0:
        error = error or f"exit code {child.returncode}"

    return {
        "index": index,
        "pid": child.pid,
        "exit_code": child.returncode,
        "external_ns": external_ns,
        "stdout_valid": stdout == expected_stdout,
        "stderr": stderr.decode("utf-8", errors="replace"),
        "error": error,
        "record": None
        if record is None
        else {
            "flags": record.flags,
            "send_completed": bool(record.flags & SEND_COMPLETED),
            "response_completed_ns": record.response_completed_ns,
            "before_transport_ns": record.before_transport_ns,
            "work_completed_ns": record.work_completed_ns,
        },
    }


def run_without_observer(args: argparse.Namespace, index: int) -> dict[str, Any]:
    environment = os.environ.copy()
    environment.pop("AGENT_OTEL_BENCH_HANDLE", None)
    if args.pipe_name:
        environment["AGENT_OTEL_PIPE"] = args.pipe_name
    if args.watchdog_ms is not None:
        environment["AGENT_OTEL_WATCHDOG_MS"] = str(args.watchdog_ms)
    command = [str(args.hook_bin), args.event, "--client", args.client]
    started_ns = time.perf_counter_ns()
    try:
        completed = subprocess.run(
            command,
            input=args.payload,
            capture_output=True,
            env=environment,
            timeout=args.timeout_seconds,
            check=False,
        )
        external_ns = time.perf_counter_ns() - started_ns
        expected = (
            b'{"decision":"allow"}'
            if args.event == "PreToolUse"
            and args.client.lower() not in {"codex", "openai", "codex-cli"}
            else b"{}"
        )
        error = None
        if completed.returncode != 0:
            error = f"exit code {completed.returncode}"
        elif completed.stdout != expected:
            error = f"unexpected stdout {completed.stdout!r}"
        return {
            "index": index,
            "external_ns": external_ns,
            "exit_code": completed.returncode,
            "stdout_valid": completed.stdout == expected,
            "error": error,
        }
    except subprocess.TimeoutExpired:
        return {
            "index": index,
            "external_ns": time.perf_counter_ns() - started_ns,
            "exit_code": None,
            "stdout_valid": False,
            "error": "external containment timeout",
        }


def percentile(values: list[int], fraction: float) -> int | None:
    if not values:
        return None
    ordered = sorted(values)
    rank = max(0, math.ceil(fraction * len(ordered)) - 1)
    return ordered[rank]


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--hook-bin", type=pathlib.Path, required=True)
    parser.add_argument("--iterations", type=int, default=100)
    parser.add_argument("--event", default="PostToolUse")
    parser.add_argument("--client", default="codex")
    parser.add_argument("--payload", type=lambda value: value.encode(), default=b"{}")
    parser.add_argument("--pipe-name")
    parser.add_argument("--watchdog-ms", type=int)
    parser.add_argument("--timeout-seconds", type=float, default=3.0)
    parser.add_argument("--skip-disabled-comparison", action="store_true")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    if args.iterations <= 0:
        raise SystemExit("--iterations must be positive")
    if not args.hook_bin.is_file():
        raise SystemExit(f"hook binary not found: {args.hook_bin}")

    samples = [run_once(args, index) for index in range(args.iterations)]
    disabled_samples = (
        []
        if args.skip_disabled_comparison
        else [run_without_observer(args, index) for index in range(args.iterations)]
    )
    work = [
        sample["record"]["work_completed_ns"]
        for sample in samples
        if sample["record"] is not None and sample["error"] is None
    ]
    failures = [sample for sample in samples if sample["error"] is not None]
    disabled_failures = [
        sample for sample in disabled_samples if sample["error"] is not None
    ]
    disabled_external = [sample["external_ns"] for sample in disabled_samples]
    digest = hashlib.sha256(args.hook_bin.read_bytes()).hexdigest()
    report = {
        "schema": "agent-otel-hook-timing/v1",
        "hook_bin": str(args.hook_bin.resolve()),
        "hook_sha256": digest,
        "observer_mode": "enabled",
        "iterations": args.iterations,
        "valid_samples": len(work),
        "missing_or_invalid": len(failures),
        "all_below_1000us": len(work) == args.iterations
        and all(value < 1_000_000 for value in work),
        "work_ns": {
            "p50": percentile(work, 0.50),
            "p99": percentile(work, 0.99),
            "max": max(work) if work else None,
        },
        "disabled_comparison": {
            "iterations": len(disabled_samples),
            "failures": len(disabled_failures),
            "external_ns": {
                "p50": percentile(disabled_external, 0.50),
                "p99": percentile(disabled_external, 0.99),
                "max": max(disabled_external) if disabled_external else None,
            },
            "samples": disabled_samples,
        },
        "samples": samples,
    }
    json.dump(report, sys.stdout, indent=2)
    sys.stdout.write("\n")
    return (
        0
        if report["all_below_1000us"] and not failures and not disabled_failures
        else 1
    )


if __name__ == "__main__":
    raise SystemExit(main())
