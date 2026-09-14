"""Compose and execute one bounded candidate performance-suite attempt."""
from __future__ import annotations

import argparse
import ctypes
import datetime
import hashlib
import json
import math
import os
import platform
import signal
import subprocess
import sys
import threading
import time
import uuid
from pathlib import Path
from typing import Any, Dict, List, Optional

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from perf_driver import (collect_environment, compute_sha256, default_active_manifest_path,
                         inspect_source_state, sanitize_for_json, verify_active_manifest)
from perf_delivery import evaluate_delivery

NORMATIVE_REF = "dev/fleet-smoke/performance-coordination.md#required-measurements-and-assertions"
REPORT_SCHEMA = Path(__file__).with_name("performance-report-v1.schema.json")
REPO_ROOT = Path(__file__).resolve().parents[2]
MAX_CHILD_OUTPUT_BYTES = 4 * 1024 * 1024
CHUNK_BYTES = 64 * 1024
SUITES = ("regression", "faults", "performance", "confirmation")


def _windows_affinity_api():
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel32.GetCurrentProcess.restype = ctypes.c_void_p
    kernel32.GetProcessAffinityMask.argtypes = [ctypes.c_void_p, ctypes.POINTER(ctypes.c_size_t),
                                                 ctypes.POINTER(ctypes.c_size_t)]
    kernel32.GetProcessAffinityMask.restype = ctypes.c_int
    kernel32.SetProcessAffinityMask.argtypes = [ctypes.c_void_p, ctypes.c_size_t]
    kernel32.SetProcessAffinityMask.restype = ctypes.c_int
    return kernel32


def _windows_process_masks(handle, kernel32=None):
    api = kernel32 or _windows_affinity_api()
    process_mask, system_mask = ctypes.c_size_t(), ctypes.c_size_t()
    if not api.GetProcessAffinityMask(handle, ctypes.byref(process_mask), ctypes.byref(system_mask)):
        raise OSError(ctypes.get_last_error(), "GetProcessAffinityMask")
    return process_mask.value, system_mask.value


def available_cpu_ids() -> List[int]:
    """Return CPUs allowed to the controller, without widening inherited affinity."""
    if os.name == "nt":
        count = os.cpu_count() or 0
        if count > 64:
            raise ValueError("windows_processor_groups_over_64_unsupported")
        api = _windows_affinity_api()
        process_mask, _ = _windows_process_masks(api.GetCurrentProcess(), api)
        cpus = [cpu for cpu in range(64) if process_mask & (1 << cpu)]
    elif hasattr(os, "sched_getaffinity"):
        cpus = sorted(os.sched_getaffinity(0))
    else:
        count = os.cpu_count() or 0
        cpus = list(range(count))
    if not cpus:
        raise ValueError("logical_cpu_inventory_unavailable")
    return cpus


def apply_process_affinity(proc: subprocess.Popen[bytes], cpus: Optional[List[int]],
                           timing: Optional[str] = None) -> Dict[str, Any]:
    if cpus is None:
        return {"requested": False, "status": "not_requested", "timing": None}
    normalized = sorted(set(cpus))
    if not normalized or any(type(cpu) is not int or cpu < 0 for cpu in normalized):
        return {"requested": True, "status": "failed", "error": "invalid_cpu_set",
                "cpus": cpus, "timing": None}
    if os.name == "nt":
        count = os.cpu_count() or 0
        if count > 64 or any(cpu >= 64 for cpu in normalized):
            return {"requested": True, "status": "unsupported",
                    "error": "windows_processor_groups_over_64_unsupported", "cpus": normalized,
                    "timing": None}
        api = _windows_affinity_api()
        handle = ctypes.c_void_p(int(proc._handle))  # type: ignore[attr-defined]
        try:
            original, system = _windows_process_masks(handle, api)
        except OSError as exc:
            return {"requested": True, "status": "failed", "error": f"GetProcessAffinityMask:{exc.errno}",
                    "cpus": normalized, "timing": None}
        mask = sum(1 << cpu for cpu in normalized)
        if mask & ~original:
            return {"requested": True, "status": "failed", "error": "cpu_outside_inherited_mask",
                    "cpus": normalized, "requested_mask": mask, "original_mask": original,
                    "system_mask": system, "timing": None}
        if not api.SetProcessAffinityMask(handle, ctypes.c_size_t(mask)):
            return {"requested": True, "status": "failed",
                    "error": f"SetProcessAffinityMask:{ctypes.get_last_error()}", "cpus": normalized,
                    "requested_mask": mask, "original_mask": original, "system_mask": system,
                    "timing": None}
        try:
            observed, _ = _windows_process_masks(handle, api)
        except OSError as exc:
            return {"requested": True, "status": "failed",
                    "error": f"GetProcessAffinityMask_after_set:{exc.errno}", "cpus": normalized,
                    "requested_mask": mask, "original_mask": original, "system_mask": system,
                    "timing": None}
        if observed != mask:
            return {"requested": True, "status": "failed", "error": "observed_mask_mismatch",
                    "cpus": normalized, "requested_mask": mask, "observed_mask": observed,
                    "original_mask": original, "system_mask": system, "timing": None}
        return {"requested": True, "status": "applied", "cpus": normalized,
                "requested_mask": mask, "observed_mask": observed, "original_mask": original,
                "system_mask": system, "timing": timing or "unspecified"}
    if not hasattr(os, "sched_getaffinity") or not hasattr(os, "sched_setaffinity"):
        return {"requested": True, "status": "unsupported", "error": "sched_affinity_unavailable",
                "cpus": normalized, "timing": None}
    try:
        original = sorted(os.sched_getaffinity(proc.pid))
        if not set(normalized).issubset(original):
            return {"requested": True, "status": "failed", "error": "cpu_outside_inherited_mask",
                    "cpus": normalized, "original_cpus": original, "timing": None}
        os.sched_setaffinity(proc.pid, normalized)
        observed = sorted(os.sched_getaffinity(proc.pid))
    except OSError as exc:
        return {"requested": True, "status": "failed", "error": f"sched_affinity:{exc.errno}",
                "cpus": normalized, "timing": None}
    if observed != normalized:
        return {"requested": True, "status": "failed", "error": "observed_mask_mismatch",
                "cpus": normalized, "original_cpus": original, "observed_cpus": observed,
                "timing": None}
    return {"requested": True, "status": "applied", "cpus": normalized,
            "original_cpus": original, "observed_cpus": observed,
            "timing": timing or "best_effort_after_spawn"}


def canonical_hash(value: Any) -> str:
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":")).encode("utf-8")).hexdigest()


class _BoundedCapture:
    def __init__(self, limit: int):
        self.remaining = limit
        self.parts = {"stdout": [], "stderr": []}
        self.exceeded = threading.Event()
        self.lock = threading.Lock()

    def add(self, stream: str, chunk: bytes) -> None:
        with self.lock:
            keep = min(len(chunk), self.remaining)
            if keep:
                self.parts[stream].append(chunk[:keep])
                self.remaining -= keep
            if keep != len(chunk):
                self.exceeded.set()

    def value(self, stream: str) -> bytes:
        with self.lock:
            return b"".join(self.parts[stream])


class _OwnedProcessTree:
    """Own a POSIX process group or a Windows kill-on-close Job Object."""
    def __init__(self, proc: subprocess.Popen[bytes]):
        self.proc = proc
        self.job = None
        self.assignment_error = None
        if os.name == "nt":
            self._create_windows_job()

    def _create_windows_job(self) -> None:
        kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)

        class IoCounters(ctypes.Structure):
            _fields_ = [(name, ctypes.c_ulonglong) for name in (
                "ReadOperationCount", "WriteOperationCount", "OtherOperationCount",
                "ReadTransferCount", "WriteTransferCount", "OtherTransferCount")]

        class BasicLimit(ctypes.Structure):
            _fields_ = [
                ("PerProcessUserTimeLimit", ctypes.c_longlong), ("PerJobUserTimeLimit", ctypes.c_longlong),
                ("LimitFlags", ctypes.c_uint32), ("MinimumWorkingSetSize", ctypes.c_size_t),
                ("MaximumWorkingSetSize", ctypes.c_size_t), ("ActiveProcessLimit", ctypes.c_uint32),
                ("Affinity", ctypes.c_size_t), ("PriorityClass", ctypes.c_uint32),
                ("SchedulingClass", ctypes.c_uint32),
            ]

        class ExtendedLimit(ctypes.Structure):
            _fields_ = [("BasicLimitInformation", BasicLimit), ("IoInfo", IoCounters),
                        ("ProcessMemoryLimit", ctypes.c_size_t), ("JobMemoryLimit", ctypes.c_size_t),
                        ("PeakProcessMemoryUsed", ctypes.c_size_t), ("PeakJobMemoryUsed", ctypes.c_size_t)]

        kernel32.CreateJobObjectW.restype = ctypes.c_void_p
        kernel32.SetInformationJobObject.argtypes = [ctypes.c_void_p, ctypes.c_int, ctypes.c_void_p, ctypes.c_uint32]
        kernel32.AssignProcessToJobObject.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
        kernel32.CloseHandle.argtypes = [ctypes.c_void_p]
        job = kernel32.CreateJobObjectW(None, None)
        if not job:
            self.assignment_error = f"CreateJobObjectW:{ctypes.get_last_error()}"
            return
        limits = ExtendedLimit()
        limits.BasicLimitInformation.LimitFlags = 0x00002000
        if not kernel32.SetInformationJobObject(job, 9, ctypes.byref(limits), ctypes.sizeof(limits)):
            self.assignment_error = f"SetInformationJobObject:{ctypes.get_last_error()}"
            kernel32.CloseHandle(job)
            return
        if not kernel32.AssignProcessToJobObject(job, ctypes.c_void_p(int(self.proc._handle))):  # type: ignore[attr-defined]
            self.assignment_error = f"AssignProcessToJobObject:{ctypes.get_last_error()}"
            kernel32.CloseHandle(job)
            return
        self.job = job

    def terminate(self) -> None:
        if os.name == "nt" and self.job:
            ctypes.WinDLL("kernel32", use_last_error=True).TerminateJobObject(ctypes.c_void_p(self.job), 1)
            return
        if os.name != "nt":
            try:
                os.killpg(self.proc.pid, signal.SIGKILL)
                return
            except OSError:
                pass
        try:
            self.proc.kill()
        except OSError:
            pass

    def resume(self) -> Optional[str]:
        if os.name != "nt":
            return None
        kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)

        class ThreadEntry32(ctypes.Structure):
            _fields_ = [("dwSize", ctypes.c_uint32), ("cntUsage", ctypes.c_uint32),
                        ("th32ThreadID", ctypes.c_uint32), ("th32OwnerProcessID", ctypes.c_uint32),
                        ("tpBasePri", ctypes.c_long), ("tpDeltaPri", ctypes.c_long), ("dwFlags", ctypes.c_uint32)]

        kernel32.CreateToolhelp32Snapshot.restype = ctypes.c_void_p
        kernel32.Thread32First.argtypes = [ctypes.c_void_p, ctypes.POINTER(ThreadEntry32)]
        kernel32.Thread32Next.argtypes = [ctypes.c_void_p, ctypes.POINTER(ThreadEntry32)]
        kernel32.OpenThread.argtypes = [ctypes.c_uint32, ctypes.c_int, ctypes.c_uint32]
        kernel32.OpenThread.restype = ctypes.c_void_p
        kernel32.ResumeThread.argtypes = [ctypes.c_void_p]
        kernel32.ResumeThread.restype = ctypes.c_uint32
        kernel32.CloseHandle.argtypes = [ctypes.c_void_p]
        snapshot = kernel32.CreateToolhelp32Snapshot(0x00000004, 0)  # TH32CS_SNAPTHREAD
        if snapshot == ctypes.c_void_p(-1).value:
            return f"CreateToolhelp32Snapshot:{ctypes.get_last_error()}"
        entry = ThreadEntry32()
        entry.dwSize = ctypes.sizeof(entry)
        resumed = False
        try:
            ok = kernel32.Thread32First(snapshot, ctypes.byref(entry))
            while ok:
                if entry.th32OwnerProcessID == self.proc.pid:
                    thread = kernel32.OpenThread(0x0002, False, entry.th32ThreadID)  # THREAD_SUSPEND_RESUME
                    if thread:
                        resumed = kernel32.ResumeThread(thread) != 0xFFFFFFFF
                        kernel32.CloseHandle(thread)
                        if resumed:
                            break
                ok = kernel32.Thread32Next(snapshot, ctypes.byref(entry))
        finally:
            kernel32.CloseHandle(snapshot)
        return None if resumed else f"ResumeThread:{ctypes.get_last_error()}"

    def close(self) -> None:
        if os.name == "nt" and self.job:
            ctypes.WinDLL("kernel32", use_last_error=True).CloseHandle(ctypes.c_void_p(self.job))
            self.job = None
        elif os.name != "nt":
            try:
                os.killpg(self.proc.pid, signal.SIGKILL)
            except OSError:
                pass


def _read_pipe(pipe, stream: str, capture: _BoundedCapture) -> None:
    try:
        while True:
            chunk = pipe.read(CHUNK_BYTES)
            if not chunk:
                return
            capture.add(stream, chunk)
    except (OSError, ValueError):
        return


def run_bounded_process(cmd: List[str], timeout_sec: float, output_limit: int = MAX_CHILD_OUTPUT_BYTES,
                        cwd: Optional[str] = None, cpus: Optional[List[int]] = None) -> Dict[str, Any]:
    """Run fixed argv with bounded capture, monotonic deadline and tree cleanup."""
    if timeout_sec <= 0 or output_limit <= 0:
        raise ValueError("timeout and output_limit must be positive")
    creationflags = ((getattr(subprocess, "CREATE_NEW_PROCESS_GROUP", 0) | 0x00000004)
                     if os.name == "nt" else 0)  # CREATE_SUSPENDED closes the pre-assignment race.
    try:
        proc = subprocess.Popen(cmd, cwd=cwd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, shell=False,
                                text=False, start_new_session=(os.name != "nt"), creationflags=creationflags)
    except OSError as exc:
        return {"exit_code": -1, "error": f"os_error: {exc}", "stdout": b"", "stderr": b"",
                "affinity": {"requested": cpus is not None, "status": "not_started", "timing": None}}
    owner = _OwnedProcessTree(proc)
    containment_ready = os.name != "nt" or owner.assignment_error is None
    affinity = apply_process_affinity(proc, cpus, "before_resume" if os.name == "nt" else "best_effort_after_spawn")
    capture = _BoundedCapture(output_limit)
    readers = [threading.Thread(target=_read_pipe, args=(proc.stdout, "stdout", capture), daemon=True),
               threading.Thread(target=_read_pipe, args=(proc.stderr, "stderr", capture), daemon=True)]
    for reader in readers:
        reader.start()
    error = None
    deadline = time.monotonic() + timeout_sec
    try:
        if affinity["status"] in ("failed", "unsupported"):
            error = "process_affinity_unavailable"
            owner.terminate()
        elif os.name == "nt" and owner.assignment_error:
            error = "process_containment_unavailable"
            owner.terminate()
        elif os.name == "nt":
            resume_error = owner.resume()
            if resume_error:
                owner.assignment_error = resume_error
                error = "process_resume_failed"
                owner.terminate()
        while proc.poll() is None:
            if capture.exceeded.is_set():
                error = "output_limit_exceeded"
                owner.terminate()
                break
            if time.monotonic() >= deadline:
                error = "subprocess_timeout_expired"
                owner.terminate()
                break
            time.sleep(min(0.02, max(0.0, deadline - time.monotonic())))
        if capture.exceeded.is_set():
            error = error or "output_limit_exceeded"
        if error:
            try:
                proc.wait(timeout=2.0)
            except subprocess.TimeoutExpired:
                owner.terminate()
        owner.close()
        try:
            proc.wait(timeout=2.0)
        except subprocess.TimeoutExpired:
            owner.terminate()
        for reader in readers:
            reader.join(timeout=1.0)
        if any(reader.is_alive() for reader in readers):
            error = error or "output_drain_deadline"
        for reader, pipe in zip(readers, (proc.stdout, proc.stderr)):
            if not reader.is_alive():
                try:
                    pipe.close()
                except (OSError, ValueError):
                    pass
    finally:
        owner.close()
    cleanup_complete = proc.returncode is not None and not any(reader.is_alive() for reader in readers) and containment_ready
    if not cleanup_complete:
        error = error or "process_tree_cleanup_incomplete"
    return {"exit_code": proc.returncode if proc.returncode is not None else -1, "error": error,
            "timeout_seconds": timeout_sec if error == "subprocess_timeout_expired" else None,
            "output_bounded": not capture.exceeded.is_set(),
            "containment": "windows_job" if os.name == "nt" and owner.assignment_error is None else
                           "posix_process_group" if os.name != "nt" else "windows_process_group_fallback",
            "containment_error": owner.assignment_error,
            "affinity": affinity,
            "process_tree_cleanup": "complete" if cleanup_complete else "incomplete",
            "stdout": capture.value("stdout"), "stderr": capture.value("stderr")}


def run_subprocess_json(cmd: List[str], timeout_sec: int, output_limit: int = MAX_CHILD_OUTPUT_BYTES,
                        cpus: Optional[List[int]] = None) -> Dict[str, Any]:
    completed = run_bounded_process(cmd, timeout_sec, output_limit=output_limit, cpus=cpus)
    stdout_raw = completed.pop("stdout", b"").decode("utf-8", errors="replace").strip()
    completed["stderr"] = completed.pop("stderr", b"").decode("utf-8", errors="replace").strip()[-4000:]
    if completed.get("error"):
        completed["data"] = None
        return completed
    try:
        completed["data"] = json.loads(stdout_raw) if stdout_raw else None
    except json.JSONDecodeError as exc:
        completed["error"] = f"json_decode_error: {exc}"
        completed["data"] = None
    return completed


def valid_ipc_counts(results):
    if not isinstance(results, list) or len(results) != 2:
        return False
    if {r.get("msg_type") for r in results} != {"HookPayload", "HookPayloadWithContext"}:
        return False
    for result in results:
        expected = result.get("expected_rounds")
        stats = result.get("latency_stats") or {}
        p99 = stats.get("p99_us")
        if (type(expected) is not int or expected <= 0 or result.get("success_count") != expected
                or stats.get("count") != expected or result.get("failure_count") != 0
                or type(p99) not in (int, float) or not math.isfinite(p99) or not 0 <= p99 < 3000):
            return False
    return True


def verified_delivery_verdict(scenario):
    if not scenario or not scenario.get("runs"):
        return "not_measured"
    outcomes = []
    for run in scenario["runs"]:
        evidence = run.get("eval", {})
        expected, received = evidence.get("expected_ids"), evidence.get("received_spans")
        if expected is None or received is None:
            outcomes.append("not_measured" if run.get("verdict") == "not_measured" else "failed")
            continue
        clients = run.get("clients", [])
        valid_clients = (len(clients) == len(expected) and {c.get("trace_id") for c in clients} == set(expected)
                         and all(c.get("response_ok") for c in clients))
        outcomes.append("failed" if run.get("collector_errors") or not valid_clients else
                        evaluate_delivery(expected, received)["verdict"])
    if "failed" in outcomes:
        return "failed"
    return "passed" if outcomes and all(v == "passed" for v in outcomes) else "not_measured"


def _not_measured(metric: str, reason: str) -> Dict[str, Any]:
    return {"metric": metric, "verdict": "not_measured", "reason": reason, "normative_ref": NORMATIVE_REF}


def _requirement_for(metric: str) -> str:
    if metric == "hook_internal_execution_duration_us":
        return "R11"
    if metric == "concurrent_event_delivery_loss":
        return "R09"
    if metric in {"otlp_response_regression_contracts", "exporter_retry_faults"}:
        return "R06"
    if metric in {"cancellation_and_capacity_faults", "shutdown_deadline_faults"}:
        return "R03"
    if metric in {"active_installation_unchanged", "owned_resource_cleanup"}:
        return "R10"
    return "R08"


def _annotate(assertion: Dict[str, Any], evidence_refs: List[str], *, required: bool = True) -> Dict[str, Any]:
    result = dict(assertion)
    result["requirement_id"] = result.get("requirement_id", _requirement_for(result.get("metric", "")))
    result["evidence_refs"] = evidence_refs
    result["required"] = required
    return result


def _micro_evidence(item: Dict[str, Any], run: Dict[str, Any], run_index: int, verdict_index: int) -> Dict[str, Any]:
    metric = item.get("metric", "")
    raw = run.get("raw_report", {})
    pointer = f"/raw_reports/micro/runs/{run_index}/evaluation/verdicts/{verdict_index}"
    enriched = dict(item)
    raw_key = {"parser_pure_json_span_throughput": "parser_pure_json_span",
               "legacy_frame_0x01_throughput": "legacy_frame_0x01_pipeline",
               "envelope_0x04_throughput": "envelope_0x04_pipeline"}.get(metric)
    if raw_key:
        measurement = raw.get(raw_key, {})
        enriched["count"] = measurement.get("iterations")
        pointer = f"/raw_reports/micro/runs/{run_index}/raw_report/{raw_key}"
    elif metric.startswith("context_harvest_"):
        fixture = metric.removeprefix("context_harvest_").removesuffix("_p99_us")
        for index, measurement in enumerate(raw.get("context_harvest", [])):
            if measurement.get("fixture") == fixture:
                enriched["count"] = measurement.get("iterations")
                pointer = f"/raw_reports/micro/runs/{run_index}/raw_report/context_harvest/{index}/stats/p99_us"
                break
    elif metric == "hook_binary_size":
        enriched["count"] = 1
        pointer = "/candidate/binaries"
    return _annotate(enriched, [pointer])


def _delivery_assertion(delivery_data: Optional[Dict[str, Any]], skip_native: bool,
                        *, required: bool = True) -> Dict[str, Any]:
    if skip_native:
        result = _not_measured("concurrent_event_delivery_loss", "explicitly_skipped_native")
    elif delivery_data:
        scenarios = delivery_data.get("scenarios", {})
        debug, release = verified_delivery_verdict(scenarios.get("debug")), verified_delivery_verdict(scenarios.get("release"))
        verdict = "failed" if "failed" in (debug, release) else "passed" if debug == release == "passed" else "not_measured"
        result = {"metric": "concurrent_event_delivery_loss", "verdict": verdict,
                  "debug_verdict": debug, "release_verdict": release, "unit": "missing_events",
                  "mode": "equal", "threshold": 0, "value": 0 if verdict == "passed" else None,
                  "normative_ref": NORMATIVE_REF, "impl_ref": "dev/fleet-smoke/perf_delivery.py"}
    else:
        result = _not_measured("concurrent_event_delivery_loss", "delivery_data_missing")
    return _annotate(result, ["/raw_reports/delivery/scenarios/debug", "/raw_reports/delivery/scenarios/release"],
                     required=required)


def _hook_timing_assertion(data: Optional[Dict[str, Any]]) -> Dict[str, Any]:
    if not data:
        return _annotate(_not_measured("hook_internal_execution_duration_us", "hook_timing_probe_missing_or_not_executed"),
                         ["/raw_reports/hook_timing"])
    if data.get("schema") == "agent-otel-hook-timing-campaign/v1":
        wrapper_verdict = data.get("verdict")
        if data.get("cleanup", {}).get("complete") is not True:
            return _annotate({**_not_measured("hook_internal_execution_duration_us", "owned_hook_campaign_cleanup_incomplete"),
                              "verdict": "failed"}, ["/raw_reports/hook_timing"])
        if wrapper_verdict != "passed":
            return _annotate({**_not_measured("hook_internal_execution_duration_us",
                                              data.get("reason") or "owned_hook_campaign_not_measured"),
                              "verdict": "failed" if wrapper_verdict == "failed" else "not_measured"},
                             ["/raw_reports/hook_timing"])
        data = data.get("measurement", {}).get("probe")
        if not isinstance(data, dict):
            return _annotate(_not_measured("hook_internal_execution_duration_us", "owned_hook_campaign_probe_missing"),
                             ["/raw_reports/hook_timing"])
    iterations, valid = data.get("iterations"), data.get("valid_samples")
    p99_ns = (data.get("work_ns") or {}).get("p99")
    numeric = type(p99_ns) in (int, float) and not isinstance(p99_ns, bool) and math.isfinite(p99_ns)
    samples = data.get("samples") or []
    sends_completed = (len(samples) == iterations and all(
        isinstance(sample.get("record"), dict) and sample["record"].get("send_completed") is True
        for sample in samples
    )) if type(iterations) is int else False
    complete = type(iterations) is int and iterations > 0 and valid == iterations and numeric and sends_completed
    verdict = "passed" if complete and p99_ns < 1_000_000 and data.get("all_below_1000us") is True else "failed"
    return _annotate({"metric": "hook_internal_execution_duration_us", "verdict": verdict,
                      "value": p99_ns / 1000.0 if numeric else None, "unit": "microseconds", "threshold": 1000.0,
                      "mode": "strictly_less", "count": valid,
                      "reason": "" if verdict == "passed" else "invalid_incomplete_unsent_or_over_deadline_hook_samples",
                      "normative_ref": NORMATIVE_REF, "impl_ref": "crates/agent-otel-client/src/hook_observer.rs"},
                     ["/raw_reports/hook_timing"])


def compose_campaign_results(micro_data: Optional[Dict[str, Any]], ipc_data: Optional[Dict[str, Any]],
                             delivery_data: Optional[Dict[str, Any]], skip_native: bool = False,
                             suite: str = "performance", hook_timing_data: Optional[Dict[str, Any]] = None,
                             delivery_diagnostic: bool = False) -> Dict[str, Any]:
    if suite not in SUITES:
        raise ValueError(f"unknown suite: {suite}")
    effective: List[Dict[str, Any]] = []
    if suite in {"regression", "performance", "confirmation"}:
        if micro_data and isinstance(micro_data.get("runs"), list) and micro_data["runs"]:
            evaluated = [(run_index, verdict_index, run, verdict)
                         for run_index, run in enumerate(micro_data["runs"])
                         for verdict_index, verdict in enumerate(run.get("evaluation", {}).get("verdicts", []))]
            if not evaluated or any("error" in run for run in micro_data["runs"]):
                effective.append(_annotate(
                    {"metric": "micro_execution", "verdict": "failed", "reason": "missing_or_failed_run_evidence"},
                    ["/raw_reports/micro/runs"]))
            for run_index, verdict_index, run, item in evaluated:
                if item.get("metric") not in {"ipc_roundtrip_p99_us", "concurrent_event_delivery_loss", "hook_internal_execution_duration_us"}:
                    effective.append(_micro_evidence(item, run, run_index, verdict_index))
        else:
            effective.append(_annotate(_not_measured("microbenchmarks", "microbenchmarks_not_executed_or_missing"),
                                       ["/raw_reports/micro"]))
        if skip_native:
            effective.append(_annotate(_not_measured("ipc_roundtrip_p99_us", "explicitly_skipped_native"),
                                       ["/raw_reports/ipc"]))
        elif ipc_data:
            valid = valid_ipc_counts(ipc_data.get("results", []))
            for result_index, result in enumerate(ipc_data.get("results", [])):
                stats = result.get("latency_stats") or {}
                expected = result.get("expected_rounds")
                result_valid = (valid and ipc_data.get("overall_passed") is True
                                and result.get("success_count") == expected and result.get("failure_count") == 0)
                effective.append(_annotate({
                    "metric": f"ipc_roundtrip_{result.get('msg_type', 'unknown')}_p99_us",
                    "verdict": "passed" if result_valid else "failed", "value": stats.get("p99_us"),
                    "unit": "microseconds", "threshold": 3000.0, "mode": "strictly_less",
                    "count": stats.get("count"), "reason": "" if result_valid else "invalid_ipc_counts_or_threshold",
                    "normative_ref": NORMATIVE_REF,
                    "impl_ref": ipc_data.get("impl_ref", "agent_otel_ipc::client::try_send")},
                    [f"/raw_reports/ipc/results/{result_index}/latency_stats/p99_us",
                     f"/raw_reports/ipc/results/{result_index}/success_count"]))
            if not ipc_data.get("results"):
                effective.append(_annotate({"metric": "ipc_roundtrip_p99_us", "verdict": "failed",
                                            "reason": "ipc_results_missing"}, ["/raw_reports/ipc/results"]))
        else:
            effective.append(_annotate(_not_measured("ipc_roundtrip_p99_us", "ipc_data_missing"),
                                       ["/raw_reports/ipc"]))
    if suite in {"faults", "performance", "confirmation"}:
        delivery_assertion = _delivery_assertion(delivery_data, skip_native, required=not delivery_diagnostic)
        if delivery_diagnostic:
            delivery_assertion["diagnostic"] = True
            effective.append(delivery_assertion)
            effective.append(_annotate(_not_measured("concurrent_event_delivery_loss",
                                                      "preload_stdin_diagnostic_cannot_satisfy_acceptance"),
                                       ["/raw_reports/delivery"], required=True))
        else:
            effective.append(delivery_assertion)
    if suite in {"performance", "confirmation"}:
        effective.append(_hook_timing_assertion(hook_timing_data))
    elif suite == "regression":
        hook_diagnostic = _hook_timing_assertion(hook_timing_data)
        hook_diagnostic["required"] = False
        hook_diagnostic["diagnostic"] = True
        effective.append(hook_diagnostic)
        delivery_diagnostic = _delivery_assertion(delivery_data, skip_native, required=False)
        delivery_diagnostic["diagnostic"] = True
        effective.append(delivery_diagnostic)
    if suite == "regression":
        effective.append(_annotate(_not_measured("otlp_response_regression_contracts", "probe_not_implemented"),
                                   ["dev/fleet-smoke/performance-implementation-spec.md#5-batch-exportacao-e-encerramento"]))
    elif suite == "faults":
        for metric in ("cancellation_and_capacity_faults", "exporter_retry_faults", "shutdown_deadline_faults"):
            effective.append(_annotate(_not_measured(metric, "probe_not_implemented"),
                                       ["dev/fleet-smoke/performance-implementation-spec.md#9-matriz-de-aceitacao"]))
    required = [item for item in effective if item.get("required", True)]
    failed = any(a.get("verdict") == "failed" for a in required)
    passed = bool(required) and all(a.get("verdict") == "passed" for a in required)
    final = "failed" if failed else "passed" if passed else "not_measured"
    return {"suite": suite, "campaign_verdict": final, "overall_passed": final == "passed", "effective_assertions": effective}


def normalize_assertions(assertions):
    normalized = []
    for index, assertion in enumerate(assertions):
        status = assertion.get("status", assertion.get("verdict", "not_measured"))
        metric = assertion.get("metric", f"assertion_{index + 1}")
        observed, reason = assertion.get("value"), assertion.get("reason", "")
        if observed is None and not reason:
            reason = "observation_not_reported"
        normalized.append({"id": f"assertion-{index + 1}",
                           "requirement_id": assertion.get("requirement_id", _requirement_for(metric)),
                           "implementation_refs": [assertion.get("impl_ref")] if assertion.get("impl_ref") else [],
                           "test_id": metric, "boundary": metric, "unit": assertion.get("unit", "unknown"),
                           "operator": assertion.get("mode", "unknown"), "threshold": assertion.get("threshold"),
                           "samples": assertion.get("stats_count", assertion.get("count")), "observed": observed,
                           "status": status, "reason": reason,
                           "evidence_refs": assertion.get("evidence_refs", []),
                           "required": assertion.get("required", True),
                           "diagnostic": assertion.get("diagnostic", False)})
    return normalized


def append_attempt_integrity_assertions(composed: Dict[str, Any], active_before: Dict[str, Any],
                                        active_after: Dict[str, Any], cleanup_complete: bool,
                                        source_before: Dict[str, Any], source_after: Dict[str, Any]) -> None:
    before_available = active_before.get("verified") is True
    after_available = active_after.get("verified") is True
    if before_available and after_available:
        active_status = "passed" if active_before == active_after else "failed"
        active_reason = "" if active_status == "passed" else "active_installation_snapshot_changed"
    else:
        active_status, active_reason = "not_measured", "active_installation_snapshot_unavailable"
    composed["effective_assertions"].append(_annotate({
        "metric": "active_installation_unchanged", "verdict": active_status, "unit": "boolean",
        "mode": "equal", "threshold": True, "value": True if active_status == "passed" else None,
        "reason": active_reason}, ["/active_before", "/active_after"]))
    composed["effective_assertions"].append(_annotate({
        "metric": "owned_resource_cleanup", "verdict": "passed" if cleanup_complete else "failed",
        "unit": "boolean", "mode": "equal", "threshold": True, "value": cleanup_complete,
        "reason": "" if cleanup_complete else "one_or_more_owned_process_trees_not_cleaned"},
        ["/cleanup", "/execution_results"]))
    source_unchanged = canonical_hash(source_before) == canonical_hash(source_after)
    composed["effective_assertions"].append(_annotate({
        "metric": "candidate_source_unchanged", "requirement_id": "R10",
        "verdict": "passed" if source_unchanged else "failed", "unit": "boolean",
        "mode": "equal", "threshold": True, "value": source_unchanged,
        "reason": "" if source_unchanged else "candidate_source_changed_during_attempt"},
        ["/source_provenance", "/source_after"]))
    required = [item for item in composed["effective_assertions"] if item.get("required", True)]
    failed = any(item.get("verdict") == "failed" for item in required)
    passed = bool(required) and all(item.get("verdict") == "passed" for item in required)
    verdict = "failed" if failed else "passed" if passed else "not_measured"
    composed["campaign_verdict"], composed["overall_passed"] = verdict, verdict == "passed"


def _type_matches(value: Any, expected: Any) -> bool:
    names = expected if isinstance(expected, list) else [expected]
    checks = {"object": lambda v: isinstance(v, dict), "array": lambda v: isinstance(v, list),
              "string": lambda v: isinstance(v, str), "integer": lambda v: isinstance(v, int) and not isinstance(v, bool),
              "number": lambda v: isinstance(v, (int, float)) and not isinstance(v, bool),
              "boolean": lambda v: isinstance(v, bool), "null": lambda v: v is None}
    return any(checks[name](value) for name in names)


def validate_report_schema(document: Any, schema_path: Path = REPORT_SCHEMA) -> Any:
    """Validate the JSON Schema subset used by the campaign without a new dependency."""
    schema = json.loads(schema_path.read_text(encoding="utf-8"))

    def walk(value: Any, rule: Dict[str, Any], location: str) -> None:
        if "$ref" in rule:
            target = schema
            for part in rule["$ref"].removeprefix("#/").split("/"):
                target = target[part]
            walk(value, target, location)
            return
        if "oneOf" in rule:
            matches = 0
            for option in rule["oneOf"]:
                try:
                    walk(value, option, location)
                    matches += 1
                except ValueError:
                    pass
            if matches != 1:
                raise ValueError(f"{location}: expected exactly one schema variant")
            return
        if "type" in rule and not _type_matches(value, rule["type"]):
            raise ValueError(f"{location}: invalid type")
        if "const" in rule and value != rule["const"]:
            raise ValueError(f"{location}: expected constant {rule['const']!r}")
        if "enum" in rule and value not in rule["enum"]:
            raise ValueError(f"{location}: value outside enum")
        if isinstance(value, str) and len(value) < rule.get("minLength", 0):
            raise ValueError(f"{location}: string too short")
        if isinstance(value, (int, float)) and not isinstance(value, bool):
            if "minimum" in rule and value < rule["minimum"]:
                raise ValueError(f"{location}: below minimum")
            if "maximum" in rule and value > rule["maximum"]:
                raise ValueError(f"{location}: above maximum")
        if isinstance(value, dict):
            missing = set(rule.get("required", [])) - set(value)
            if missing:
                raise ValueError(f"{location}: missing {sorted(missing)}")
            properties = rule.get("properties", {})
            for key, child in value.items():
                if key in properties:
                    walk(child, properties[key], f"{location}.{key}")
                elif rule.get("additionalProperties") is False:
                    raise ValueError(f"{location}: unexpected property {key}")
        if isinstance(value, list) and "items" in rule:
            for index, child in enumerate(value):
                walk(child, rule["items"], f"{location}[{index}]")
    walk(document, schema, "report")
    return document


def _execute_probe(name: str, argv: List[str], timeout: int, commands: List[Dict[str, Any]],
                   execution: Dict[str, Any], raw: Dict[str, Any]) -> None:
    commands.append({"probe": name, "argv": argv, "environment": "inherited_sanitized_not_recorded"})
    result = run_bounded_process(argv, timeout, cwd=str(REPO_ROOT))
    stdout_raw = result.pop("stdout", b"").decode("utf-8", errors="replace").strip()
    result["stderr"] = result.pop("stderr", b"").decode("utf-8", errors="replace").strip()[-4000:]
    if result.get("error"):
        result["data"] = None
    else:
        try:
            result["data"] = json.loads(stdout_raw) if stdout_raw else None
        except json.JSONDecodeError as exc:
            result["error"], result["data"] = f"json_decode_error: {exc}", None
    execution[name], raw[name] = result, result.get("data")


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description="Unified performance campaign runner and report composer")
    exe = ".exe" if platform.system() == "Windows" else ""
    parser.add_argument("--micro-driver", default=os.path.join("dev", "fleet-smoke", "perf_driver.py"))
    parser.add_argument("--example-bin", default=os.path.join("target", "release", "examples", f"performance{exe}"))
    parser.add_argument("--ipc-bin", default=os.path.join("target", "release", "examples", f"performance_ipc{exe}"))
    parser.add_argument("--delivery-probe", default=os.path.join("dev", "fleet-smoke", "perf_delivery.py"))
    parser.add_argument("--hook-timing-probe", default=os.path.join("dev", "fleet-smoke", "hook_timing_probe.py"))
    parser.add_argument("--hook-timing-campaign", default=os.path.join("dev", "fleet-smoke", "hook_timing_campaign.py"))
    parser.add_argument("--hook-timing-pipe", help="Explicit campaign-owned live candidate pipe")
    parser.add_argument("--hook-bin", default=os.path.join("target", "release", f"agent-hook{exe}"))
    parser.add_argument("--debug-daemon-bin", default=os.path.join("target", "debug", f"agent-otel-bridge{exe}"))
    parser.add_argument("--release-daemon-bin", default=os.path.join("target", "release", f"agent-otel-bridge{exe}"))
    parser.add_argument("--manifest")
    parser.add_argument("--repeats", type=int, default=3)
    parser.add_argument("--suite", choices=SUITES, default="performance")
    parser.add_argument("--campaign-id")
    parser.add_argument("--run-id")
    parser.add_argument("--attempt-index", type=int)
    parser.add_argument("--seed", type=int)
    parser.add_argument("--skip-native", action="store_true")
    parser.add_argument("--observe-hook", action="store_true", help="Forward hook observation diagnostics to delivery probe")
    parser.add_argument("--preload-stdin", action="store_true", help="Diagnostic delivery variant; cannot satisfy delivery acceptance")
    parser.add_argument("--compose-only")
    args = parser.parse_args(argv)
    if args.attempt_index is not None and not 1 <= args.attempt_index <= 5:
        parser.error("attempt-index must be between 1 and 5")
    if args.seed is not None and args.seed < 0:
        parser.error("seed must be non-negative")
    if not 1 <= args.repeats <= 5:
        parser.error("repeats must be between 1 and 5")
    if args.attempt_index not in (None, 1) and not (args.campaign_id and args.run_id):
        parser.error("attempt-index greater than one requires controller campaign-id and run-id")
    if args.compose_only:
        captured = json.loads(Path(args.compose_only).read_text(encoding="utf-8"))
        composed = compose_campaign_results(captured.get("micro"), captured.get("ipc"), captured.get("delivery"),
                                            args.skip_native, args.suite, captured.get("hook_timing"), args.preload_stdin)
        output = {"mode": "compose_only", "input_file": args.compose_only, "raw_reports": captured,
                  "environment": (captured.get("micro") or {}).get("environment"),
                  "source_provenance": (captured.get("micro") or {}).get("source_provenance"),
                  "composed_evaluation": composed}
        sys.stdout.write(json.dumps(sanitize_for_json(output), indent=2) + "\n")
        return 0 if composed["overall_passed"] else 1 if composed["campaign_verdict"] == "failed" else 3

    environment, source = collect_environment(), inspect_source_state(str(REPO_ROOT))
    active_path = args.manifest or default_active_manifest_path()
    active_before = verify_active_manifest(active_path) if active_path else {"verified": False, "reason": "active_manifest_path_unavailable"}
    started_at = datetime.datetime.now(datetime.timezone.utc).isoformat()
    commands: List[Dict[str, Any]] = []
    raw: Dict[str, Any] = {}
    execution: Dict[str, Any] = {}
    if args.suite in {"regression", "performance", "confirmation"}:
        micro = [sys.executable, args.micro_driver, "--example-bin", args.example_bin, "--hook-bin", args.hook_bin,
                 "--repeats", str(args.repeats), "--timeout", "60"]
        if args.manifest:
            micro.extend(["--manifest", args.manifest])
        _execute_probe("micro", micro, 300, commands, execution, raw)
        if not args.skip_native and os.path.isfile(args.ipc_bin):
            _execute_probe("ipc", [args.ipc_bin], 50, commands, execution, raw)
    if args.suite in {"regression", "faults", "performance", "confirmation"} and not args.skip_native:
        if os.path.isfile(args.delivery_probe) and os.path.isfile(args.hook_bin):
            delivery = [sys.executable, args.delivery_probe, "--hook", args.hook_bin, "--repeats", str(args.repeats),
                        "--events", "24", "--concurrency", "4"]
            if os.path.isfile(args.debug_daemon_bin):
                delivery.extend(["--debug-daemon", args.debug_daemon_bin])
            if os.path.isfile(args.release_daemon_bin):
                delivery.extend(["--release-daemon", args.release_daemon_bin])
            if args.observe_hook:
                delivery.append("--observe-hook")
            if args.preload_stdin:
                delivery.append("--preload-stdin")
            _execute_probe("delivery", delivery, 180, commands, execution, raw)
    if args.suite in {"regression", "performance", "confirmation"} and not args.skip_native:
        if (os.name == "nt" and args.hook_timing_pipe and os.path.isfile(args.hook_timing_probe)
                and os.path.isfile(args.hook_bin)):
            timing = [sys.executable, args.hook_timing_probe, "--hook-bin", args.hook_bin,
                      "--iterations", str(max(100, args.repeats * 100)), "--pipe-name", args.hook_timing_pipe]
            _execute_probe("hook_timing", timing, 300, commands, execution, raw)
        elif (os.name == "nt" and os.path.isfile(args.hook_timing_campaign)
              and os.path.isfile(args.release_daemon_bin) and os.path.isfile(args.hook_bin)):
            timing = [sys.executable, args.hook_timing_campaign, "--daemon-bin", args.release_daemon_bin,
                      "--hook-bin", args.hook_bin, "--iterations", str(max(100, args.repeats * 100))]
            _execute_probe("hook_timing", timing, 300, commands, execution, raw)

    composed = compose_campaign_results(raw.get("micro"), raw.get("ipc"), raw.get("delivery"), args.skip_native,
                                        args.suite, raw.get("hook_timing"), args.preload_stdin)
    active_after = verify_active_manifest(active_path) if active_path else {"verified": False, "reason": "active_manifest_path_unavailable"}
    cleanup_complete = all(item.get("process_tree_cleanup") == "complete" for item in execution.values())
    source_after = inspect_source_state(str(REPO_ROOT))
    append_attempt_integrity_assertions(composed, active_before, active_after, cleanup_complete, source, source_after)
    binaries = [{"path": path, "sha256": compute_sha256(path),
                 "size_bytes": os.path.getsize(path) if os.path.isfile(path) else None}
                for path in (args.example_bin, args.ipc_bin, args.hook_bin,
                              args.debug_daemon_bin, args.release_daemon_bin)]
    report = {"record_type": "attempt", "schema_version": 1, "spec_version": "performance-implementation-spec-v1",
              "campaign_id": args.campaign_id or str(uuid.uuid4()), "run_id": args.run_id or str(uuid.uuid4()),
              "attempt_index": args.attempt_index or 1, "mode": "standalone" if not args.campaign_id else "candidate",
              "suite": args.suite, "started_at_utc": started_at,
              "ended_at_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(), "seed": args.seed or 0,
              "environment": environment,
              "candidate": {"revision": source.get("candidate_git_revision"), "dirty": source.get("is_dirty"),
                            "diff_digest": source.get("git_diff_head_sha256"), "binaries": binaries,
                            "untracked": source.get("untracked_source_sha256",
                                                    source.get("dev_fleet_smoke_source_sha256", {})),
                            "toolchain": environment.get("rustc_version"),
                            "target": environment.get("architecture"), "profile": "candidate_release"},
              "active_before": active_before, "active_after": active_after,
              "commands": commands, "assertions": normalize_assertions(composed["effective_assertions"]),
              "trace_evidence": {"status": "not_checked", "origin": "native"},
              "cleanup": {"status": "complete" if cleanup_complete else "incomplete",
                          "owned_process_trees": len(commands)}, "raw_reports": raw,
              "execution_results": execution, "source_provenance": source, "source_after": source_after,
              "composed_evaluation": composed,
              "verdict": composed["campaign_verdict"]}
    try:
        validate_report_schema(report)
    except (OSError, json.JSONDecodeError, ValueError) as exc:
        sys.stderr.write(f"report schema validation failed: {exc}\n")
        return 2
    sys.stdout.write(json.dumps(sanitize_for_json(report), indent=2) + "\n")
    return 0 if report["verdict"] == "passed" else 1 if report["verdict"] == "failed" else 3


if __name__ == "__main__":
    raise SystemExit(main())
