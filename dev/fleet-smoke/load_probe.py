#!/usr/bin/env python3
import argparse
import concurrent.futures
import ctypes
import json
import os
import platform
import random
import sys
import threading
import time
from typing import Any, Dict, List, Optional
import uuid

if os.name == "nt":
    from ctypes import wintypes

    class PROCESS_MEMORY_COUNTERS(ctypes.Structure):
        _fields_ = [
            ("cb", wintypes.DWORD),
            ("PageFaultCount", wintypes.DWORD),
            ("PeakWorkingSetSize", ctypes.c_size_t),
            ("WorkingSetSize", ctypes.c_size_t),
            ("QuotaPeakPagedPoolUsage", ctypes.c_size_t),
            ("QuotaPagedPoolUsage", ctypes.c_size_t),
            ("QuotaPeakNonPagedPoolUsage", ctypes.c_size_t),
            ("QuotaNonPagedPoolUsage", ctypes.c_size_t),
            ("PagefileUsage", ctypes.c_size_t),
            ("PeakPagefileUsage", ctypes.c_size_t),
        ]

    try:
        kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
        psapi = ctypes.WinDLL("psapi", use_last_error=True)

        kernel32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
        kernel32.OpenProcess.restype = wintypes.HANDLE
        kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
        kernel32.CloseHandle.restype = wintypes.BOOL
        psapi.GetProcessMemoryInfo.argtypes = [
            wintypes.HANDLE,
            ctypes.POINTER(PROCESS_MEMORY_COUNTERS),
            wintypes.DWORD,
        ]
        psapi.GetProcessMemoryInfo.restype = wintypes.BOOL
    except Exception:
        kernel32 = None
        psapi = None
else:
    kernel32 = None
    psapi = None

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
if SCRIPT_DIR not in sys.path:
    sys.path.insert(0, SCRIPT_DIR)
REPO_ROOT = os.path.abspath(os.path.join(SCRIPT_DIR, "..", ".."))
if REPO_ROOT not in sys.path:
    sys.path.insert(0, REPO_ROOT)

import architecture_suite as arch
from perf_driver import collect_environment, compute_sha256, inspect_source_state, sanitize_for_json

DEFAULT_LEVELS = (1, 4, 8, 16)
DEFAULT_EVENTS = (96, 192, 384, 768)
MAX_EVENTS_PER_PROFILE = 900
WARMUP_EVENTS = 2
_ACTIVE_RSS_SAMPLERS = []
_ACTIVE_RSS_LOCK = threading.Lock()


def get_windows_rss(pid: int) -> Optional[int]:
    if os.name != "nt" or kernel32 is None or psapi is None:
        return None
    PROCESS_QUERY_INFORMATION = 0x0400
    PROCESS_VM_READ = 0x0010
    handle = kernel32.OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, False, pid)
    if not handle:
        return None
    try:
        counters = PROCESS_MEMORY_COUNTERS()
        counters.cb = ctypes.sizeof(PROCESS_MEMORY_COUNTERS)
        if psapi.GetProcessMemoryInfo(handle, ctypes.byref(counters), counters.cb):
            return int(counters.WorkingSetSize)
        return None
    finally:
        kernel32.CloseHandle(handle)


def get_linux_rss(pid: int) -> Optional[int]:
    try:
        with open(f"/proc/{pid}/statm", "r", encoding="utf-8") as f:
            parts = f.read().split()
            page_size = os.sysconf("SC_PAGE_SIZE")
            return int(parts[1]) * page_size
    except Exception:
        return None


def get_process_rss(pid: int) -> Optional[int]:
    if os.name == "nt":
        return get_windows_rss(pid)
    if sys.platform.startswith("linux"):
        return get_linux_rss(pid)
    return None


class RssSampler(threading.Thread):
    def __init__(self, pid: int, interval: float = 0.1):
        super().__init__(daemon=True)
        self.pid = pid
        self.interval = interval
        self.stop_event = threading.Event()
        self.samples: List[int] = []

    def start(self) -> None:
        super().start()
        with _ACTIVE_RSS_LOCK:
            _ACTIVE_RSS_SAMPLERS.append(self)

    def run(self) -> None:
        while not self.stop_event.is_set():
            rss = get_process_rss(self.pid)
            if rss is not None:
                self.samples.append(rss)
            self.stop_event.wait(self.interval)

    def stop(self) -> List[int]:
        self.stop_event.set()
        self.join(timeout=1.0)
        if self.is_alive():
            raise RuntimeError("RSS sampler did not stop within one second")
        with _ACTIVE_RSS_LOCK:
            if self in _ACTIVE_RSS_SAMPLERS:
                _ACTIVE_RSS_SAMPLERS.remove(self)
        return self.samples


def percentile(values: List[float], p: float) -> float:
    if not values:
        return 0.0
    s = sorted(values)
    k = (len(s) - 1) * (p / 100.0)
    f = int(k)
    c = f + 1
    if c < len(s):
        return round(s[f] + (k - f) * (s[c] - s[f]), 2)
    return round(s[f], 2)


def calc_stats(values: List[float]) -> Dict[str, Any]:
    if not values:
        return {"p50": "not_measured", "p95": "not_measured", "p99": "not_measured", "max": "not_measured"}
    s = sorted(values)
    return {
        "p50": percentile(s, 50.0),
        "p95": percentile(s, 95.0),
        "p99": percentile(s, 99.0),
        "max": round(s[-1], 2),
    }


def parse_positive_csv(value: str, name: str) -> List[int]:
    try:
        parsed = [int(item.strip()) for item in value.split(",") if item.strip()]
    except ValueError as exc:
        raise ValueError(f"{name} must contain comma-separated integers") from exc
    if not parsed or any(item <= 0 for item in parsed):
        raise ValueError(f"{name} must contain positive integers")
    return parsed


def integrity_available(source: Dict[str, Any], candidates: Dict[str, Any], active: Dict[str, Any]) -> bool:
    return (arch.source_state_available(source)
            and all(arch._valid_sha(value) for value in candidates.values())
            and arch.installed_bridge_available(active))


def load_integrity_ok(load_result: Dict[str, Any], admitted: int) -> bool:
    metrics = load_result.get("metrics", {})
    evidence = load_result.get("trace_evidence", {})
    return (admitted > 0 and load_result.get("verdict") == "passed"
            and metrics.get("delivered") == admitted and evidence.get("valid") is True)


def next_ids(rng: random.Random, used_trace_ids: set) -> tuple[str, str]:
    while True:
        trace_id = f"{rng.getrandbits(128):032x}"
        if int(trace_id, 16) != 0 and trace_id not in used_trace_ids:
            used_trace_ids.add(trace_id)
            break
    while True:
        span_id = f"{rng.getrandbits(64):016x}"
        if int(span_id, 16) != 0:
            return trace_id, span_id


def execute_worker(sess: Any, hook_bin: str, idx: int, trace_id: str, span_id: str,
                   workspace: str) -> Dict[str, Any]:
    started = time.monotonic()
    client = {"index": idx, "trace_id": trace_id, "span_id": span_id,
              "send_start": started, "hook_end": started, "duration_ms": 0.0,
              "response_ok": False, "error": "worker_did_not_complete"}
    try:
        result = sess.hook(hook_bin, idx, trace_id, span_id, workspace)
        ended = time.monotonic()
        client.update(dict(result) if isinstance(result, dict) else {})
        client.update({
            "trace_id": trace_id, "span_id": span_id,
            "send_start": result.get("send_start", started),
            "hook_end": result.get("hook_end", ended),
            "duration_ms": result.get("duration_ms", round((ended - started) * 1000.0, 2)),
            "response_ok": bool(result.get("response_ok", result.get("returncode") == 0)),
        })
    except Exception as exc:
        client.update({"hook_end": time.monotonic(), "error": f"worker_error:{type(exc).__name__}"})
    return client


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Standalone Agent OTEL Load Probe")
    parser.add_argument("--daemon-bin", required=True, help="Absolute path to daemon executable")
    parser.add_argument("--hook-bin", required=True, help="Absolute path to hook executable")
    parser.add_argument("--seed", type=int, default=42, help="Random seed")
    parser.add_argument("--levels", type=str, default=",".join(map(str, DEFAULT_LEVELS)), help="Comma-separated concurrency levels")
    parser.add_argument("--events", type=str, default=",".join(map(str, DEFAULT_EVENTS)), help="Comma-separated event counts matching --levels")
    parser.add_argument("--seconds", type=float, default=4.0, help="Duration per level in seconds [1..10]")
    parser.add_argument("--max-events", type=int, default=MAX_EVENTS_PER_PROFILE, help="Safety cap per level; maximum 900")
    args = parser.parse_args()

    if not os.path.isabs(args.daemon_bin):
        sys.stderr.write(f"Error: --daemon-bin must be an absolute path: {args.daemon_bin}\n")
        sys.exit(2)
    if not os.path.isabs(args.hook_bin):
        sys.stderr.write(f"Error: --hook-bin must be an absolute path: {args.hook_bin}\n")
        sys.exit(2)
    if not (1.0 <= args.seconds <= 10.0):
        sys.stderr.write(f"Error: --seconds must be within bounds 1..10: {args.seconds}\n")
        sys.exit(2)
    try:
        levels = parse_positive_csv(args.levels, "--levels")
        events = parse_positive_csv(args.events, "--events")
    except ValueError as exc:
        sys.stderr.write(f"Error: {exc}\n")
        sys.exit(2)
    if len(levels) != len(events) or len(set(levels)) != len(levels) or any(level > 16 for level in levels):
        sys.stderr.write("Error: --levels must be unique values in 1..16 and match --events\n")
        sys.exit(2)
    if not (1 <= args.max_events <= MAX_EVENTS_PER_PROFILE) or any(event > args.max_events for event in events):
        sys.stderr.write("Error: event counts must be positive, at most --max-events, and --max-events must be <= 900\n")
        sys.exit(2)
    args.level_values = levels
    args.event_values = events
    return args


def _run_load_probe() -> None:
    args = parse_args()
    overall_start = time.monotonic()
    arch.CAMPAIGN_DEADLINE = overall_start + 175.0
    arch.SCENARIO_TIMEOUT_SECONDS = 20.0

    run_id = str(uuid.uuid4())
    rng = random.Random(f"{args.seed}:{run_id}")
    used_trace_ids = set()
    levels = args.level_values
    event_counts = args.event_values

    candidate_hashes_before = {
        "daemon_bin_sha256": compute_sha256(args.daemon_bin),
        "hook_bin_sha256": compute_sha256(args.hook_bin),
    }
    source_state_before = inspect_source_state()
    installed_bridge_before = arch.inspect_installed_bridge()

    profiles = []

    for concurrency, configured_events in zip(levels, event_counts):
        if time.monotonic() >= arch.CAMPAIGN_DEADLINE or (time.monotonic() - overall_start) > 170.0:
            break

        scenario_deadline = min(arch.CAMPAIGN_DEADLINE, time.monotonic() + arch.SCENARIO_TIMEOUT_SECONDS)
        total_planned = configured_events
        target_rate = total_planned / args.seconds
        dt = 1.0 / target_rate

        warmup_clients: List[Dict[str, Any]] = []
        load_clients: List[Dict[str, Any]] = []
        clients_lock = threading.Lock()
        latenesses_ms: List[float] = []
        schedule_missed = 0
        admission_skipped = 0
        worker_errors = 0
        scheduled_count = 0
        actual_offer_times: List[float] = []
        admitted_plan: List[Dict[str, Any]] = []

        with arch.scenario_session(args.daemon_bin, deadline=scenario_deadline) as (sess, ready):
            if not ready or sess.proc is None:
                exc_res = arch.exception_result(f"load_c{concurrency}", RuntimeError("Daemon failed to start or pipe not ready"))
                profiles.append({"concurrency": concurrency, "verdict": "failed", "assertions": exc_res.get("assertions", [])})
                continue

            ws = sess.new_fixture(f"bench_c{concurrency}")
            rss_sampler = RssSampler(sess.proc.pid, interval=0.1)
            rss_sampler.start()

            received_warmup_ids = set()
            for w_idx in range(WARMUP_EVENTS):
                if time.monotonic() >= sess.deadline:
                    break
                w_tid, w_sid = next_ids(rng, used_trace_ids)
                t0 = time.monotonic()
                h_res = sess.hook(args.hook_bin, w_idx, w_tid, w_sid, ws)
                t1 = time.monotonic()
                c = dict(h_res) if isinstance(h_res, dict) else {}
                c.update({
                    "trace_id": w_tid, "span_id": w_sid,
                    "send_start": h_res.get("send_start", t0), "hook_end": h_res.get("hook_end", t1),
                    "duration_ms": h_res.get("duration_ms", round((t1 - t0) * 1000.0, 2)),
                    "response_ok": bool(h_res.get("response_ok", h_res.get("returncode") == 0 if isinstance(h_res, dict) else False)),
                })
                warmup_clients.append(c)
                receipt_deadline = min(sess.deadline, time.monotonic() + 2.0)
                while time.monotonic() < receipt_deadline:
                    if any(span.get("trace_id") == w_tid for span in sess.col.received_spans):
                        received_warmup_ids.add(w_tid)
                        break
                    time.sleep(0.02)

            warmup_ids = {client["trace_id"] for client in warmup_clients}
            warmup_deadline = min(sess.deadline, time.monotonic() + 3.0)
            while time.monotonic() < warmup_deadline:
                received_warmup_ids = {span["trace_id"] for span in sess.col.received_spans
                                       if span.get("trace_id") in warmup_ids}
                if received_warmup_ids == warmup_ids:
                    break
                time.sleep(0.02)
            last_warmup_spans = [span for span in sess.col.received_spans
                                 if warmup_clients and span.get("trace_id") == warmup_clients[-1]["trace_id"]]
            last_warmup_attrs = last_warmup_spans[-1].get("attributes", {}) if last_warmup_spans else {}
            warmup_state = arch.get_attr(last_warmup_attrs, "agent.context.state", "state")
            warmup_branch = arch.get_attr(last_warmup_attrs, "vcs.branch.name", "branch")
            warmup_verified = (len(received_warmup_ids) == WARMUP_EVENTS
                               and len(last_warmup_spans) == 1
                               and warmup_state == "fresh" and warmup_branch == f"bench_c{concurrency}")

            sem = threading.Semaphore(concurrency)
            pool = concurrent.futures.ThreadPoolExecutor(max_workers=concurrency)
            t_load_start = time.monotonic()

            for i in range(total_planned):
                t_sched = t_load_start + i * dt
                now = time.monotonic()
                if now + dt >= sess.deadline:
                    break

                sleep_time = t_sched - now
                if sleep_time > 0:
                    time.sleep(sleep_time)
                    now = time.monotonic()

                lateness = max(0.0, (now - t_sched) * 1000.0)
                latenesses_ms.append(lateness)
                scheduled_count += 1
                actual_offer_times.append(now)
                if lateness > max(50.0, dt * 2000.0):
                    schedule_missed += 1

                if not sem.acquire(blocking=False):
                    admission_skipped += 1
                    continue

                t_id, s_id = next_ids(rng, used_trace_ids)
                admitted_plan.append({"index": 1000 + i, "trace_id": t_id, "span_id": s_id})

                def run_hook_worker(idx: int, trace_id: str, span_id: str) -> None:
                    fallback_start = time.monotonic()
                    client = {"index": idx, "trace_id": trace_id, "span_id": span_id,
                              "send_start": fallback_start, "hook_end": fallback_start,
                              "duration_ms": 0.0, "response_ok": False,
                              "error": "worker_did_not_complete"}
                    try:
                        client = execute_worker(sess, args.hook_bin, idx, trace_id, span_id, ws)
                    except BaseException as exc:
                        client.update({"hook_end": time.monotonic(),
                                       "error": f"worker_error:{type(exc).__name__}"})
                    finally:
                        with clients_lock:
                            load_clients.append(client)
                        sem.release()

                try:
                    pool.submit(run_hook_worker, 1000 + i, t_id, s_id)
                except Exception as exc:
                    with clients_lock:
                        load_clients.append({"index": 1000 + i, "trace_id": t_id, "span_id": s_id,
                                             "send_start": now, "hook_end": time.monotonic(),
                                             "duration_ms": 0.0, "response_ok": False,
                                             "error": f"submit_error:{type(exc).__name__}"})
                    sem.release()

            pool.shutdown(wait=True)
            worker_errors = sum(str(client.get("error", "")).startswith(("worker_error:", "submit_error:"))
                                for client in load_clients)
            t_load_end = time.monotonic()

            source_current = inspect_source_state()
            candidates_current = {
                "daemon_bin_sha256": compute_sha256(args.daemon_bin),
                "hook_bin_sha256": compute_sha256(args.hook_bin),
            }
            active_current = arch.inspect_installed_bridge()

            warmup_assertions = [
                {"id": "warmup_responses", "observed": sum(bool(c.get("response_ok")) for c in warmup_clients), "expected": WARMUP_EVENTS,
                 "status": "passed" if len(warmup_clients) == WARMUP_EVENTS and all(c.get("response_ok") for c in warmup_clients) else "failed"},
                {"id": "warmup_context_fresh", "observed": {"state": warmup_state, "branch": warmup_branch,
                                                               "received": len(received_warmup_ids)},
                 "expected": {"state": "fresh", "branch": f"bench_c{concurrency}", "received": WARMUP_EVENTS},
                 "status": "passed" if warmup_verified else "failed"},
            ]
            arch.make_scenario_result(
                f"load_c{concurrency}_warmup",
                "passed" if all(a["status"] == "passed" for a in warmup_assertions) else "failed",
                warmup_assertions, warmup_clients, [], sess.cleanup_info
            )

            load_assertions = [
                {"id": "unchanged_source", "observed": arch.source_state_available(source_current) and source_current == source_state_before, "expected": True,
                 "status": "passed" if arch.source_state_available(source_current) and source_current == source_state_before else "failed"},
                {"id": "unchanged_candidates", "observed": all(arch._valid_sha(value) for value in candidates_current.values()) and candidates_current == candidate_hashes_before, "expected": True,
                 "status": "passed" if all(arch._valid_sha(value) for value in candidates_current.values()) and candidates_current == candidate_hashes_before else "failed"},
                {"id": "unchanged_active", "observed": arch.installed_bridge_available(active_current) and active_current == installed_bridge_before, "expected": True,
                 "status": "passed" if arch.installed_bridge_available(active_current) and active_current == installed_bridge_before else "failed"},
                {"id": "admitted_events_positive", "observed": len(load_clients) > 0, "expected": True,
                 "status": "passed" if len(load_clients) > 0 else "failed"},
                {"id": "valid_hook_responses", "observed": all(c.get("response_ok") for c in load_clients), "expected": True,
                 "status": "passed" if all(c.get("response_ok") for c in load_clients) else "failed"},
                {"id": "worker_results_complete", "observed": len(load_clients), "expected": len(admitted_plan),
                 "status": "passed" if len(load_clients) == len(admitted_plan) else "failed"},
            ]
            init_verdict = "passed" if all(a["status"] == "passed" for a in load_assertions) else "failed"
            load_res = arch.make_scenario_result(
                f"load_c{concurrency}", init_verdict, load_assertions, load_clients, [], sess.cleanup_info
            )

        rss_samples = rss_sampler.stop()
        baseline_rss = rss_samples[0] if rss_samples else "not_measured"
        peak_rss = max(rss_samples) if rss_samples else "not_measured"
        delta_rss = (peak_rss - baseline_rss) if (isinstance(peak_rss, int) and isinstance(baseline_rss, int)) else "not_measured"

        shut = sess.cleanup_info.get("daemon_shutdown", {})
        diag = shut.get("diagnostics") or {}
        pipeline = diag.get("pipeline", {}) if isinstance(diag, dict) else {}
        ingress = diag.get("ingress", {}) if isinstance(diag, dict) else {}

        daemon_diag = {
            "source": "final_bridge_diagnostics_after_graceful_drain",
            "ingress_admitted": ingress.get("admitted", "not_measured"),
            "ingress_reserved_bytes": ingress.get("reserved_bytes", "not_measured"),
            "peak_ingress_bytes": ingress.get("peak_reserved_bytes", "not_measured"),
            "active_connections": ingress.get("active_connections", "not_measured"),
            "peak_connections": ingress.get("peak_connections", "not_measured"),
            "pipeline_transformed": pipeline.get("transformed", "not_measured"),
            "export_accepted": pipeline.get("accepted", "not_measured"),
            "peak_queued_bytes": pipeline.get("peak_queued_bytes", "not_measured"),
            "queued_bytes": pipeline.get("queued_bytes", "not_measured"),
            "queued_items": pipeline.get("queued_items", "not_measured"),
            "dropped": pipeline.get("shutdown_dropped", pipeline.get("dropped", 0)),
            "unknown": pipeline.get("unknown", 0),
            "queue_latency": "not_measured",
            "peak_queued_items": "not_measured",
            "memory_allocator": "not_measured",
        }

        durations = [c["duration_ms"] for c in load_clients]
        e2e_lats = load_res.get("metrics", {}).get("e2e_latencies_ms", [])
        if not isinstance(e2e_lats, list):
            e2e_lats = []

        offered_cnt = scheduled_count
        admitted_cnt = len(admitted_plan)
        completed_cnt = sum(1 for c in load_clients if c.get("response_ok"))
        delivered_cnt = load_res.get("metrics", {}).get("delivered", 0)

        starts = [c["send_start"] for c in load_clients]
        ends = [c["hook_end"] for c in load_clients]
        first_start = min(starts) if starts else t_load_start
        last_start = max(starts) if starts else first_start
        last_end = max(ends) if ends else last_start
        admitted_rate = round(admitted_cnt / (last_start - first_start), 2) if (last_start > first_start) else "not_measured"
        completed_rate = round(completed_cnt / (last_end - first_start), 2) if (last_end > first_start) else "not_measured"

        lat_p95 = percentile(latenesses_ms, 95.0)
        generator_limited = bool(admission_skipped > 0 or worker_errors > 0 or schedule_missed > 0
                                 or lat_p95 > max(50.0, dt * 2000.0))

        final_verdict = "passed" if load_integrity_ok(load_res, admitted_cnt) else "failed"

        actual_offer_duration = ((actual_offer_times[-1] - actual_offer_times[0] + dt)
                                 if actual_offer_times else 0.0)

        profile = {
            "concurrency": concurrency,
            "target_rate_eps": target_rate,
            "scheduled_interval_ms": round(dt * 1000.0, 3),
            "duration_seconds": round(t_load_end - t_load_start, 2),
            "configured_seconds": args.seconds,
            "max_events": args.max_events,
            "counts": {
                "offered": offered_cnt,
                "configured": total_planned,
                "admitted": admitted_cnt,
                "completed": completed_cnt,
                "delivered": delivered_cnt,
                "schedule_missed": schedule_missed,
                "admission_skipped": admission_skipped,
                "worker_errors": worker_errors,
                "generator_limited": generator_limited,
            },
            "rates": {
                "offered_rate_eps": round(offered_cnt / actual_offer_duration, 2) if actual_offer_duration > 0 else "not_measured",
                "admitted_rate_eps": admitted_rate,
                "completed_rate_eps": completed_rate,
                "delivered_rate_eps": load_res.get("metrics", {}).get("delivered_rate_eps", "not_measured"),
            },
            "percentiles": {
                "external_hook_durations_ms": calc_stats(durations),
                "e2e_latencies_ms": calc_stats(e2e_lats),
                "scheduler_lateness_ms": calc_stats(latenesses_ms),
            },
            "rss_bytes": {
                "baseline": baseline_rss,
                "peak": peak_rss,
                "delta": delta_rss,
            },
            "daemon_diagnostics": daemon_diag,
            "verdict": final_verdict,
            "assertions": load_res.get("assertions", []),
            "trace_evidence": load_res.get("trace_evidence", {}),
            "cleanup": load_res.get("cleanup", {}),
        }
        profiles.append(profile)

    source_state_after = inspect_source_state()
    candidate_hashes_after = {
        "daemon_bin_sha256": compute_sha256(args.daemon_bin),
        "hook_bin_sha256": compute_sha256(args.hook_bin),
    }
    installed_bridge_after = arch.inspect_installed_bridge()

    all_passed = len(profiles) == len(levels) and all(p.get("verdict") == "passed" for p in profiles)
    source_ok = (arch.source_state_available(source_state_before)
                 and arch.source_state_available(source_state_after)
                 and source_state_before == source_state_after)
    candidates_ok = (all(arch._valid_sha(value) for value in candidate_hashes_before.values())
                     and all(arch._valid_sha(value) for value in candidate_hashes_after.values())
                     and candidate_hashes_before == candidate_hashes_after)
    active_ok = (arch.installed_bridge_available(installed_bridge_before)
                 and arch.installed_bridge_available(installed_bridge_after)
                 and installed_bridge_before == installed_bridge_after)
    overall_verdict = "passed" if (all_passed and source_ok and candidates_ok and active_ok) else "failed"

    report = {
        "schema": "agent-otel-load/v1",
        "run_id": run_id,
        "seed": args.seed,
        "environment": collect_environment(),
        "source_state": {"before": source_state_before, "after": source_state_after},
        "candidate_hashes": {"before": candidate_hashes_before, "after": candidate_hashes_after},
        "installed_bridge": {"before": installed_bridge_before, "after": installed_bridge_after},
        "integrity_gates": {"source_intact": source_ok, "candidate_binaries_intact": candidates_ok,
                            "installed_bridge_intact": active_ok},
        "campaign_duration_seconds": round(time.monotonic() - overall_start, 2),
        "profiles": profiles,
        "verdict": overall_verdict,
        "limitations": [
            "Synthetic load generated via CLI hook processes; microbench profiles (IPC-only, parser, harvester) are measured separately",
            "Queue latency, peak queued items, and memory allocator metrics are not exposed by daemon diagnostics and reported as not_measured",
            "External hook durations measure end-to-end hook process execution, not internal hook IPC duration",
        ],
    }

    print(json.dumps(sanitize_for_json(report), indent=2))
    sys.exit(0 if overall_verdict == "passed" else 1)


def run_load_probe() -> None:
    try:
        _run_load_probe()
    finally:
        with _ACTIVE_RSS_LOCK:
            active = list(_ACTIVE_RSS_SAMPLERS)
        for sampler in active:
            sampler.stop()


if __name__ == "__main__":
    run_load_probe()
