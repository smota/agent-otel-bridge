"""Reconcile deterministic IPC offers against an owned real daemon/OTLP receiver."""
import argparse
import collections
import json
import os
from pathlib import Path
import secrets
import time

import architecture_suite as arch
from perf_campaign import run_subprocess_json


def clean_shutdown(cleanup):
    shutdown = cleanup.get("daemon_shutdown", {})
    return (cleanup.get("collector_stopped") is True and cleanup.get("fixtures_purged") is True
            and not cleanup.get("collector_errors") and shutdown.get("forced") in (None, False)
            and shutdown.get("measurement_status") == "measured" and shutdown.get("exit_code") == 0
            and shutdown.get("output_bounded") is True and shutdown.get("output_drain_complete") is True)


def reconcile(emitter, spans, configuration=None):
    if not isinstance(emitter, dict) or emitter.get("schema") != "agent-otel-ipc-load/v1":
        return {"valid": False, "reason": "invalid_emitter_report"}
    events = emitter.get("per_event", [])
    if configuration is not None and (not isinstance(emitter.get("configuration"), dict)
            or any(emitter["configuration"].get(k) != v for k, v in configuration.items())
            or not isinstance(events, list) or len(events) != configuration["events"]):
        return {"valid": False, "reason": "offered_workload_mismatch"}
    if not isinstance(events, list) or not all(isinstance(e, dict) for e in events):
        return {"valid": False, "reason": "invalid_event_records"}
    if not all(isinstance(e.get("trace_id"), str) and type(e.get("index")) is int for e in events):
        return {"valid": False, "reason": "invalid_event_identity"}
    expected = {event.get("trace_id"): event for event in events}
    complete = (bool(events) and len(expected) == len(events)
                and all(type(emitter.get(k)) is int and emitter[k] == len(events) for k in
                        ("planned_events", "offered_events", "attempted_events", "send_completed_events"))
                and type(emitter.get("send_failed_events")) is int and emitter["send_failed_events"] == 0
                and sorted(e.get("index", -1) for e in events) == list(range(len(events)))
                and all(e.get("send_completed") is True and e.get("stage") is None and e.get("os_code") is None
                        and arch._valid_hex(e.get("parent_span_id"), 16) and int(e["parent_span_id"], 16) != 0 for e in events)
                and all(arch._valid_hex(t, 32) and int(t, 16) != 0 for t in expected))
    by_trace = collections.defaultdict(list)
    for span in spans:
        by_trace[span.get("trace_id")].append(span)
    missing = sorted(set(expected)-set(by_trace))
    unexpected = sorted(str(t) for t in set(by_trace)-set(expected))
    duplicate = sorted(str(t) for t, rows in by_trace.items() if len(rows) != 1)
    bad_parents = [t for t in set(expected) & set(by_trace)
                   if any(s.get("parent_span_id") != expected[t].get("parent_span_id") for s in by_trace[t])]
    invalid_spans = [s for s in spans if not arch._valid_hex(s.get("span_id"), 16) or int(s["span_id"], 16) == 0]
    observation = [{"trace_id": t, "stage": expected[t].get("stage"), "os_code": expected[t].get("os_code"),
                    "classification": "transport_error" if expected[t].get("send_completed") is False and expected[t].get("stage") else "unknown_before_receiver"}
                   for t in missing]
    return {"valid": complete and not (missing or unexpected or duplicate or bad_parents or invalid_spans),
            "expected": len(expected), "received": len(spans), "missing_trace_ids": missing,
            "unexpected_trace_ids": unexpected, "duplicate_trace_ids": duplicate,
            "bad_parent_trace_ids": sorted(bad_parents), "invalid_span_ids": len(invalid_spans),
            "missing_observations": observation,
            "span_identities": [{k: s.get(k) for k in ("trace_id", "span_id", "parent_span_id")} for s in spans]}


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--daemon-bin", required=True)
    p.add_argument("--emitter", required=True)
    p.add_argument("--seed", type=int, default=502)
    args = p.parse_args()
    from production_round import snapshot, snapshot_valid
    paths = {"daemon": args.daemon_bin, "emitter": args.emitter}
    before = snapshot(paths)
    results = []
    deadline = time.monotonic()+170
    for concurrency in (1, 4, 8, 16):
        name = "aob-round-"+secrets.token_hex(8)
        endpoint = "\\\\.\\pipe\\"+name if os.name == "nt" else "/tmp/"+name+".sock"
        process = {}
        with arch.scenario_session(args.daemon_bin, deadline=deadline, pipe_name=endpoint) as (session, ready):
            if ready:
                process = run_subprocess_json([args.emitter, "--endpoint", endpoint, "--events", "400", "--concurrency", str(concurrency), "--rate", "1000", "--seed", str(args.seed+concurrency)], min(30, max(0.1, deadline-time.monotonic())))
                target = (process.get("data") or {}).get("attempted_events", 0)
                until = min(deadline, time.monotonic()+2)
                while len(session.col.received_spans) < target and time.monotonic() < until:
                    time.sleep(0.02)
        # Capture after graceful drain, not just after the producer exited.
        emitter = process.get("data") or {}
        evidence = reconcile(emitter, session.col.received_spans,
                             {"endpoint": endpoint, "events": 400, "concurrency": concurrency,
                              "target_rate_eps": 1000.0, "seed": args.seed+concurrency})
        cleanup = dict(session.cleanup_info)
        # ScenarioSession's hook ledger is empty for an external native emitter;
        # replace that helper's unrelated ledger with the independently reconciled offers.
        cleanup["session_trace_evidence"] = evidence
        clean = clean_shutdown(cleanup)
        ok = (ready and process.get("exit_code") == 0 and not process.get("error")
              and process.get("output_bounded") is True
              and process.get("process_tree_cleanup") == "complete" and evidence["valid"] and clean)
        results.append({"concurrency": concurrency, "verdict": "passed" if ok else "failed",
                        "generator_limited": emitter.get("generator_limited", True),
                        "emitter": emitter, "evidence": evidence, "cleanup": cleanup,
                        "process": {k: v for k, v in process.items() if k != "data"}})
    after = snapshot(paths)
    intact = snapshot_valid(before) and snapshot_valid(after) and before == after
    report = {"schema": "agent-otel-production-ipc/v1", "profiles": results,
              "integrity": {"before": before, "after": after, "intact": intact},
              "verdict": "passed" if intact and all(r["verdict"] == "passed" for r in results) else "failed",
              "capacity": "not_established" if any(r["generator_limited"] for r in results) else "observed_at_declared_rate",
              "schedule_interpretation": "generator_limited means the offered schedule was not sustained; OS scheduling, worker occupancy and receiver backpressure are not causally distinguished by this flag",
              "boundary": "explicit IPC offers through candidate daemon to owned OTLP collector; not real-harness process creation"}
    print(json.dumps(report, allow_nan=False))
    return 0 if report["verdict"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
