#!/usr/bin/env python3
"""Bounded repeated stall campaign over the production IPC and real-hook probes."""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import secrets
import sys
import time

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import architecture_suite as arch
from perf_campaign import apply_process_affinity, available_cpu_ids, run_subprocess_json
from production_ipc_probe import clean_shutdown, reconcile
from production_round import reserve, snapshot, snapshot_valid
from perf_driver import collect_environment

SCHEMA = "agent-otel-stall-round/v1"
DEFAULT_CONCURRENCY = (1, 4, 8, 16)
DEFAULT_REAL_HOOK_RATES = (5, 20, 50)
DEFAULT_REPETITIONS = 10
DEFAULT_DURATION_SECONDS = 15.0
DEFAULT_RATE_EPS = 1000.0
MAX_ATTEMPTS = 3
MAX_REPETITIONS = 20
CHILD_OUTPUT_LIMIT = 32 * 1024 * 1024
RAW_ARTIFACT_LIMIT = 128 * 1024 * 1024


def file_sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def positive_csv(value, name):
    try:
        values = tuple(int(item.strip()) for item in value.split(",") if item.strip())
    except ValueError as exc:
        raise argparse.ArgumentTypeError(f"{name} must be comma-separated integers") from exc
    if not values or any(item <= 0 for item in values) or len(set(values)) != len(values):
        raise argparse.ArgumentTypeError(f"{name} must contain unique positive integers")
    return values


def failure_clusters(events):
    """Return consecutive failed-index runs without inventing receiver-side timing."""
    failures = sorted(
        (event for event in events if isinstance(event, dict) and event.get("send_completed") is False
         and type(event.get("index")) is int),
        key=lambda event: event["index"],
    )
    clusters = []
    for event in failures:
        start = event.get("start_offset_us")
        end = event.get("end_offset_us")
        duration = end - start if type(start) is int and type(end) is int and end >= start else None
        key = (event.get("stage"), event.get("os_code"))
        if clusters and event["index"] == clusters[-1]["end_index"] + 1 and key == clusters[-1]["failure_key"]:
            cluster = clusters[-1]
            cluster["end_index"] = event["index"]
            cluster["count"] += 1
            cluster["raw_failure_durations_us"].append(duration)
            cluster["end_offset_us"] = end
        else:
            clusters.append({
                "start_index": event["index"], "end_index": event["index"], "count": 1,
                "stage": event.get("stage"), "os_code": event.get("os_code"),
                "start_offset_us": start, "end_offset_us": end,
                "raw_failure_durations_us": [duration], "failure_key": key,
            })
    for cluster in clusters:
        cluster.pop("failure_key")
    return clusters


def summarize_ipc(emitter, evidence, elapsed_wall_seconds):
    events = emitter.get("per_event", []) if isinstance(emitter, dict) else []
    offered = emitter.get("offered_events") if isinstance(emitter, dict) else None
    attempted = emitter.get("attempted_events") if isinstance(emitter, dict) else None
    failed = emitter.get("send_failed_events") if isinstance(emitter, dict) else None
    received_rows = evidence.get("span_identities", []) if isinstance(evidence, dict) else []
    received_ids = {row.get("trace_id") for row in received_rows if isinstance(row, dict) and row.get("trace_id")}
    event_rows = events if isinstance(events, list) else []
    offered_ids = {event.get("trace_id") for event in event_rows if isinstance(event, dict) and event.get("trace_id")}
    completed_ids = {event.get("trace_id") for event in event_rows
                     if isinstance(event, dict) and event.get("send_completed") is True and event.get("trace_id")}
    failed_ids = {event.get("trace_id") for event in event_rows
                  if isinstance(event, dict) and event.get("send_completed") is False and event.get("trace_id")}
    delivered_raw = evidence.get("received") if isinstance(evidence, dict) else None
    delivered_expected = len(received_ids & offered_ids)
    delivered_completed = len(received_ids & completed_ids)
    delivered_failed = len(received_ids & failed_ids)
    return {
        "offered_events": offered,
        "attempted_events": attempted,
        "send_failed_events": failed,
        "receiver_received_spans_raw": delivered_raw,
        "receiver_delivered_expected_identities": delivered_expected,
        "receiver_delivered_send_completed_identities": delivered_completed,
        "receiver_delivered_despite_send_failure": delivered_failed,
        "send_failure_fraction_of_attempted": (failed / attempted if type(failed) is int and type(attempted) is int and attempted > 0 else None),
        "receiver_delivery_fraction_of_offered": (delivered_expected / offered if type(offered) is int and offered > 0 else None),
        "receiver_delivery_fraction_of_send_completed": (
            delivered_completed / (attempted - failed)
            if all(type(value) is int for value in (attempted, failed)) and attempted > failed else None
        ),
        "achieved_attempt_rate_eps": emitter.get("achieved_attempt_rate") if isinstance(emitter, dict) else None,
        "generator_limited": emitter.get("generator_limited") if isinstance(emitter, dict) else None,
        "emitter_elapsed_seconds": emitter.get("elapsed_seconds") if isinstance(emitter, dict) else None,
        "wall_elapsed_seconds": elapsed_wall_seconds,
        "failure_clusters": failure_clusters(events if isinstance(events, list) else []),
    }


def write_raw(path, result):
    payload = json.dumps(result, indent=2, allow_nan=False).encode("utf-8")
    if len(payload) > RAW_ARTIFACT_LIMIT:
        raise ValueError(f"raw artifact exceeds {RAW_ARTIFACT_LIMIT} byte limit: {len(payload)}")
    path.write_bytes(payload)
    return {"path": str(path), "sha256": file_sha256(path), "bytes": path.stat().st_size}


def bounded_timeout(deadline, per_child_limit, now=None):
    remaining = deadline - (time.monotonic() if now is None else now)
    return min(per_child_limit, remaining) if remaining > 0 else 0.0


def affinity_plan(enabled):
    if not enabled:
        return {"requested": False, "status": "not_requested", "allowed_cpus": None,
                "daemon_cpus": None, "emitter_cpus": None}
    try:
        allowed = available_cpu_ids()
    except ValueError as exc:
        return {"requested": True, "status": "unsupported", "error": str(exc),
                "allowed_cpus": None, "daemon_cpus": None, "emitter_cpus": None}
    if len(allowed) < 2:
        return {"requested": True, "status": "unsupported", "error": "fewer_than_two_allowed_cpus",
                "allowed_cpus": allowed, "daemon_cpus": None, "emitter_cpus": None}
    split = len(allowed) // 2
    return {"requested": True, "status": "planned", "allowed_cpus": allowed,
            "daemon_cpus": allowed[:split], "emitter_cpus": allowed[split:]}


def run_ipc_profile(args, output, repetition, concurrency, deadline, isolation):
    endpoint_name = "aob-round-" + secrets.token_hex(8)
    endpoint = "\\\\.\\pipe\\" + endpoint_name if os.name == "nt" else "/tmp/" + endpoint_name + ".sock"
    events = round(args.duration_seconds * args.rate)
    seed = args.seed + repetition * 100 + concurrency
    argv = [args.emitter, "--endpoint", endpoint, "--events", str(events), "--concurrency",
            str(concurrency), "--rate", str(args.rate), "--seed", str(seed)]
    remaining = bounded_timeout(deadline, args.profile_timeout)
    if remaining <= 0:
        return {"repetition": repetition, "concurrency": concurrency, "verdict": "failed",
                "error": "campaign_deadline", "argv": argv}
    started = time.monotonic()
    process = {}
    daemon_affinity = {"requested": isolation["requested"], "status": "not_applied", "timing": None}
    prior_collector_limit = arch.MAX_COLLECTOR_ITEMS
    arch.MAX_COLLECTOR_ITEMS = events  # Exact declared bound; restored after this owned session.
    try:
        with arch.scenario_session(args.daemon_bin, deadline=deadline, pipe_name=endpoint) as (session, ready):
            if ready:
                daemon_affinity = apply_process_affinity(
                    session.proc, isolation["daemon_cpus"], "after_ready_before_emitter")
                if isolation["requested"] and daemon_affinity["status"] != "applied":
                    ready = False
            if ready:
                process = run_subprocess_json(argv, remaining, output_limit=CHILD_OUTPUT_LIMIT,
                                              cpus=isolation["emitter_cpus"])
                target = (process.get("data") or {}).get("attempted_events", 0)
                drain_until = min(deadline, time.monotonic() + args.receiver_drain_seconds)
                while len(session.col.received_spans) < target and time.monotonic() < drain_until:
                    time.sleep(0.02)
    finally:
        arch.MAX_COLLECTOR_ITEMS = prior_collector_limit
    emitter = process.get("data") or {}
    expected = {"endpoint": endpoint, "events": events, "concurrency": concurrency,
                "target_rate_eps": args.rate, "seed": seed}
    evidence = reconcile(emitter, session.col.received_spans, expected)
    cleanup = dict(session.cleanup_info)
    cleanup["session_trace_evidence"] = evidence
    raw = {"argv": argv, "environment": {"AGENT_OTEL_PIPE_or_SOCKET": endpoint},
           "affinity": {"plan": isolation, "daemon": daemon_affinity,
                        "emitter": process.get("affinity")},
           "process": process, "evidence": evidence, "cleanup": cleanup}
    artifact = write_raw(output / f"ipc-r{repetition:02d}-c{concurrency}.json", raw)
    elapsed = time.monotonic() - started
    ok = (ready and process.get("exit_code") == 0 and not process.get("error")
          and process.get("output_bounded") is True and process.get("process_tree_cleanup") == "complete"
          and evidence.get("valid") is True and clean_shutdown(cleanup))
    return {"repetition": repetition, "concurrency": concurrency,
            "verdict": "passed" if ok else "failed", "error": process.get("error"),
            "affinity": {"daemon": daemon_affinity, "emitter": process.get("affinity")},
            "argv": argv, "summary": summarize_ipc(emitter, evidence, elapsed), "raw_artifact": artifact}


def run_real_hook_profiles(args, output, deadline):
    if not args.hook_bin:
        return {"status": "not_measured", "reason": "--hook-bin not supplied", "profiles": []}
    profiles = []
    for repetition in range(1, args.real_hook_repetitions + 1):
        for rate in args.real_hook_rates:
            events = round(rate * args.real_hook_seconds)
            argv = [sys.executable, str(HERE / "load_probe.py"), "--daemon-bin", args.daemon_bin,
                    "--hook-bin", args.hook_bin, "--levels", "1", "--events", str(events),
                    "--seconds", str(args.real_hook_seconds), "--max-events", str(events),
                    "--seed", str(args.seed + repetition * 100 + rate)]
            remaining = bounded_timeout(deadline, args.real_hook_timeout)
            if remaining <= 0:
                profiles.append({"repetition": repetition, "target_rate_eps": rate, "verdict": "failed",
                                 "error": "campaign_deadline", "argv": argv})
                return {"status": "measured", "boundary": "external real-hook process execution through owned candidate daemon and collector",
                        "profiles": profiles}
            result = run_subprocess_json(argv, remaining, output_limit=CHILD_OUTPUT_LIMIT)
            artifact = write_raw(output / f"real-hook-r{repetition:02d}-{rate}eps.json", {"argv": argv, "process": result})
            data = result.get("data") or {}
            source_profile = (data.get("profiles") or [{}])[0]
            profiles.append({"repetition": repetition, "target_rate_eps": rate,
                             "verdict": "passed" if result.get("exit_code") == 0 and data.get("verdict") == "passed" else "failed",
                             "error": result.get("error"), "argv": argv, "observed": {
                                 "counts": source_profile.get("counts"), "rates": source_profile.get("rates"),
                                 "external_hook_durations_ms": source_profile.get("percentiles", {}).get("external_hook_durations_ms")},
                             "raw_artifact": artifact})
    return {"status": "measured", "boundary": "external real-hook process execution through owned candidate daemon and collector",
            "profiles": profiles}


def validate_args(args):
    if args.isolate_cpus and args.hook_bin:
        raise ValueError("--isolate-cpus supports IPC profiles only; real-hook affinity is not measured")
    if not 1 <= args.repetitions <= MAX_REPETITIONS:
        raise ValueError(f"repetitions must be in 1..{MAX_REPETITIONS}")
    if not (0 < args.duration_seconds <= 60 and math.isfinite(args.duration_seconds)):
        raise ValueError("duration-seconds must be finite and in (0, 60]")
    if not (1 <= args.rate <= 50000 and math.isfinite(args.rate)):
        raise ValueError("rate must be finite and in [1, 50000]")
    if any(level > 32 for level in args.concurrency):
        raise ValueError("concurrency must be <=32")
    events = round(args.duration_seconds * args.rate)
    if not 1 <= events <= args.emitter_max_events:
        raise ValueError(f"sustained workload requires {events} events but emitter limit is {args.emitter_max_events}")
    if args.hook_bin and any(round(rate * args.real_hook_seconds) > 900 for rate in args.real_hook_rates):
        raise ValueError("real-hook profile exceeds load_probe's 900-event safety cap")
    if not 1 <= args.real_hook_repetitions <= MAX_REPETITIONS:
        raise ValueError(f"real-hook-repetitions must be in 1..{MAX_REPETITIONS}")
    for name in ("profile_timeout", "campaign_timeout", "receiver_drain_seconds", "real_hook_timeout"):
        value = getattr(args, name)
        if not (value > 0 and math.isfinite(value)):
            raise ValueError(f"{name.replace('_', '-')} must be finite and positive")


def parse_args(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", required=True, type=Path)
    parser.add_argument("--attempt", required=True, type=int)
    parser.add_argument("--daemon-bin", required=True)
    parser.add_argument("--emitter", required=True)
    parser.add_argument("--hook-bin")
    parser.add_argument("--daemon-label", default="candidate")
    parser.add_argument("--isolate-cpus", action="store_true")
    parser.add_argument("--repetitions", type=int, default=DEFAULT_REPETITIONS)
    parser.add_argument("--concurrency", default=",".join(map(str, DEFAULT_CONCURRENCY)))
    parser.add_argument("--duration-seconds", type=float, default=DEFAULT_DURATION_SECONDS)
    parser.add_argument("--rate", type=float, default=DEFAULT_RATE_EPS)
    parser.add_argument("--seed", type=int, default=502)
    parser.add_argument("--emitter-max-events", type=int, default=15000)
    parser.add_argument("--profile-timeout", type=float, default=180.0)
    parser.add_argument("--campaign-timeout", type=float, default=1800.0)
    parser.add_argument("--receiver-drain-seconds", type=float, default=5.0)
    parser.add_argument("--real-hook-rates", default=",".join(map(str, DEFAULT_REAL_HOOK_RATES)))
    parser.add_argument("--real-hook-seconds", type=float, default=15.0)
    parser.add_argument("--real-hook-repetitions", type=int, default=DEFAULT_REPETITIONS)
    parser.add_argument("--real-hook-timeout", type=float, default=180.0)
    args = parser.parse_args(argv)
    args.concurrency = positive_csv(args.concurrency, "--concurrency")
    args.real_hook_rates = positive_csv(args.real_hook_rates, "--real-hook-rates")
    validate_args(args)
    return args


def main(argv=None):
    args = parse_args(argv)
    paths = {"daemon": str(Path(args.daemon_bin).resolve()), "emitter": str(Path(args.emitter).resolve())}
    if args.hook_bin:
        paths["hook"] = str(Path(args.hook_bin).resolve())
    args.daemon_bin, args.emitter = paths["daemon"], paths["emitter"]
    args.hook_bin = paths.get("hook")
    before = snapshot(paths)
    isolation = affinity_plan(args.isolate_cpus)
    missing = [name for name, path in paths.items() if not Path(path).is_file()]
    if missing or not snapshot_valid(before) or isolation["status"] == "unsupported":
        raise ValueError(f"preflight failed before reservation; missing={missing}, snapshot_valid={snapshot_valid(before)}, affinity={isolation}")
    record = reserve(args.output_dir, args.attempt)
    output = args.output_dir / f"attempt-{args.attempt}.reserved"
    started = time.monotonic()
    deadline = time.monotonic() + args.campaign_timeout
    ipc = []
    for repetition in range(1, args.repetitions + 1):
        for concurrency in args.concurrency:
            ipc.append(run_ipc_profile(args, output, repetition, concurrency, deadline, isolation))
    real_hook = run_real_hook_profiles(args, output, deadline)
    after = snapshot(paths)
    integrity = snapshot_valid(before) and snapshot_valid(after) and before == after
    passed = integrity and all(item.get("verdict") == "passed" for item in ipc)
    if real_hook["status"] == "measured":
        passed = passed and all(item.get("verdict") == "passed" for item in real_hook["profiles"])
    report = {"schema": SCHEMA, **record, "ended_at_ms": int(time.time() * 1000),
              "duration_seconds": time.monotonic() - started, "environment": collect_environment(),
              "configuration": {
                  "repetitions": args.repetitions, "concurrency": args.concurrency,
                  "duration_seconds": args.duration_seconds, "target_rate_eps": args.rate,
                  "events_per_profile": round(args.duration_seconds * args.rate), "max_attempts": MAX_ATTEMPTS,
                  "real_hook_repetitions": args.real_hook_repetitions,
                  "real_hook_rates_eps": args.real_hook_rates,
                  "real_hook_duration_seconds": args.real_hook_seconds},
              "commands": {"controller": [sys.executable, str(Path(__file__).resolve()), *(argv or sys.argv[1:])]},
              "binary_paths": paths, "snapshots": {"before": before, "after": after},
              "daemon_label": args.daemon_label,
              "cpu_isolation": isolation,
              "integrity_intact": integrity, "ipc_profiles": ipc, "real_hook": real_hook,
              "verdict": "passed" if passed else "failed",
              "boundary": "candidate IPC offers through owned candidate daemon to owned OTLP receiver; active installation is read-only"}
    report_path = output / "report.json"
    report_path.write_text(json.dumps(report, indent=2, allow_nan=False), encoding="utf-8")
    print(json.dumps({"schema": SCHEMA, "report": str(report_path), "attempt": args.attempt,
                      "verdict": report["verdict"], "integrity_intact": integrity,
                      "ipc_profile_count": len(ipc), "ipc_failed_count": sum(p["verdict"] != "passed" for p in ipc),
                      "real_hook_status": real_hook["status"]}, allow_nan=False))
    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
