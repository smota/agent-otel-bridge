"""Bounded, resumable performance campaign controller."""
from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import os
import subprocess
import sys
import tempfile
import uuid
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from perf_campaign import SUITES, run_bounded_process, validate_report_schema
from perf_driver import inspect_source_state

MAX_ATTEMPTS = 5
MAX_ATTEMPT_SECONDS = 900
MAX_CAMPAIGN_SECONDS = 4500
MAX_OUTPUT_BYTES = 4 * 1024 * 1024
STATES = ("prepare", "verify", "reserve", "run", "observe", "assess", "cleanup", "decide",
          "complete", "repair", "blocked", "exhausted")
SPEC_VERSION = "performance-implementation-spec-v1"
REQUIRED_SUITES = list(SUITES)
REPO_ROOT = Path(__file__).resolve().parents[2]


def now():
    return dt.datetime.now(dt.timezone.utc).isoformat()


def canonical_hash(value) -> str:
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":")).encode("utf-8")).hexdigest()


def atomic_write(path: Path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, temp_name = tempfile.mkstemp(prefix=".ledger-", suffix=".tmp", dir=str(path.parent))
    try:
        with os.fdopen(fd, "w", encoding="utf-8", newline="\n") as stream:
            json.dump(value, stream, indent=2, sort_keys=True)
            stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temp_name, path)
    finally:
        if os.path.exists(temp_name):
            os.unlink(temp_name)


def transition(ledger, to_state, reason):
    if to_state not in STATES:
        raise ValueError(f"invalid state: {to_state}")
    old = ledger["state"]
    ledger["state"] = to_state
    ledger["state_history"].append({"from": old, "to": to_state, "at_utc": now(), "reason": reason})


def new_ledger(campaign_id=None, campaign_kind="performance", seed=42):
    return {"record_type": "campaign", "schema_version": 1, "spec_version": SPEC_VERSION,
            "campaign_kind": campaign_kind, "campaign_id": campaign_id or str(uuid.uuid4()),
            "max_attempts": MAX_ATTEMPTS, "attempts_consumed": 0, "state": "prepare",
            "state_history": [], "attempts": [],
            "coverage": {"required": list(REQUIRED_SUITES), "obtained": [], "missing": list(REQUIRED_SUITES),
                         "invalidated": []},
            "decision": "start", "terminal_reason": "", "verdict": "not_measured", "seed": seed,
            "elapsed_total_seconds": 0.0, "candidate_fingerprint": None, "candidate_history": []}


def validate_ledger(ledger):
    validate_report_schema(ledger)
    if ledger.get("state") not in STATES:
        raise ValueError("invalid current state")
    attempts = ledger.get("attempts", [])
    if ledger.get("attempts_consumed") != len(attempts):
        raise ValueError("attempt count mismatch")
    if [a.get("attempt_index") for a in attempts] != list(range(1, len(attempts) + 1)):
        raise ValueError("attempt indexes must be contiguous")
    run_ids = [attempt.get("run_id") for attempt in attempts]
    if any(not run_id for run_id in run_ids) or len(run_ids) != len(set(run_ids)):
        raise ValueError("missing or duplicate run_id")
    if ledger.get("elapsed_total_seconds", 0) > MAX_CAMPAIGN_SECONDS:
        raise ValueError("campaign elapsed deadline exceeded")
    coverage = ledger.get("coverage", {})
    if coverage.get("required") != REQUIRED_SUITES:
        raise ValueError("required suite coverage changed")
    obtained = coverage.get("obtained", [])
    if len(obtained) != len(set(obtained)) or any(suite not in REQUIRED_SUITES for suite in obtained):
        raise ValueError("invalid suite coverage")
    if coverage.get("missing") != [suite for suite in REQUIRED_SUITES if suite not in obtained]:
        raise ValueError("suite coverage does not reconcile")
    for attempt in attempts:
        report = attempt.get("raw_report")
        digest = attempt.get("raw_report_sha256")
        if report is not None:
            validate_report_schema(report)
            if digest != canonical_hash(report):
                raise ValueError("evidence hash mismatch")
            if (report.get("campaign_id") != ledger["campaign_id"]
                    or report.get("run_id") != attempt.get("run_id")
                    or report.get("attempt_index") != attempt.get("attempt_index")
                    or report.get("suite") != attempt.get("suite")):
                raise ValueError("attempt evidence identity mismatch")
        elif attempt.get("ended_at_utc") is not None:
            raise ValueError("completed attempt missing hashed raw report")
    return ledger


def recover_interrupted(ledger):
    """Consume a reserved attempt left incomplete by a crashed controller."""
    if ledger.get("state") not in {"reserve", "run", "observe", "assess", "cleanup"}:
        return ledger
    if not ledger.get("attempts"):
        raise ValueError("incomplete ledger has no reserved attempt")
    attempt = ledger["attempts"][-1]
    if attempt.get("ended_at_utc") is None:
        attempt["verdict"] = "not_measured"
        attempt["process"] = {"error": "controller_interrupted_after_reservation"}
        attempt["ended_at_utc"] = now()
        # Interrupted reports are still schema-valid, hashed evidence records.
        report = _interrupted_report(ledger, attempt)
        attempt["raw_report"] = report
        attempt["raw_report_sha256"] = canonical_hash(report)
        transition(ledger, "repair", "reserved attempt interrupted and consumed")
        ledger["decision"], ledger["verdict"] = "needs_repair", "not_measured"
        ledger["terminal_reason"] = "controller_interrupted_after_reservation"
    return ledger


def _interrupted_report(ledger, attempt):
    return {"record_type": "attempt", "schema_version": 1, "spec_version": SPEC_VERSION,
            "campaign_id": ledger["campaign_id"], "run_id": attempt["run_id"],
            "attempt_index": attempt["attempt_index"], "mode": "candidate", "suite": attempt["suite"],
            "started_at_utc": attempt["started_at_utc"], "ended_at_utc": attempt["ended_at_utc"],
            "seed": attempt["seed"], "environment": {}, "candidate": attempt["candidate"],
            "active_before": {}, "active_after": {}, "commands": attempt["commands"], "assertions": [],
            "trace_evidence": {"status": "not_checked"}, "cleanup": {"status": "incomplete"},
            "verdict": "not_measured"}


def candidate_snapshot():
    snapshot = inspect_source_state(str(REPO_ROOT))
    return snapshot, canonical_hash(snapshot)


def expected_suite(ledger):
    if ledger.get("decision") == "needs_repair" and ledger.get("attempts"):
        return ledger["attempts"][-1]["suite"]
    missing = ledger["coverage"]["missing"]
    return missing[0] if missing else None


def fixed_command(script, repeats, seed, campaign_id=None, attempt_index=None, suite="performance",
                  skip_native=False, run_id=None, hook_timing_pipe=None, observe_hook=False,
                  preload_stdin=False):
    command = [sys.executable, str(script), "--repeats", str(repeats), "--suite", suite]
    if campaign_id:
        command.extend(["--campaign-id", campaign_id])
    if run_id:
        command.extend(["--run-id", run_id])
    if attempt_index is not None:
        command.extend(["--attempt-index", str(attempt_index)])
    if seed is not None:
        command.extend(["--seed", str(seed)])
    if skip_native:
        command.append("--skip-native")
    if hook_timing_pipe:
        command.extend(["--hook-timing-pipe", hook_timing_pipe])
    if observe_hook:
        command.append("--observe-hook")
    if preload_stdin:
        command.append("--preload-stdin")
    return command


def _record_candidate(ledger, snapshot, fingerprint, index):
    previous = ledger.get("candidate_fingerprint")
    if previous and previous != fingerprint:
        invalidated = list(ledger["coverage"]["obtained"])
        ledger["coverage"]["invalidated"].append(
            {"candidate_fingerprint": previous, "suites": invalidated, "at_attempt": index})
        ledger["coverage"]["obtained"] = []
        ledger["coverage"]["missing"] = list(REQUIRED_SUITES)
        ledger["candidate_history"].append({"from": previous, "to": fingerprint, "attempt_index": index})
        ledger["decision"] = "candidate_changed"
        transition(ledger, "verify", "candidate changed; prior suite coverage invalidated")
    ledger["candidate_fingerprint"] = fingerprint
    return {"revision": snapshot.get("candidate_git_revision"), "dirty": snapshot.get("is_dirty")
            if isinstance(snapshot.get("is_dirty"), bool) else None,
            "diff_digest": snapshot.get("git_diff_head_sha256"), "binaries": [],
            "source": snapshot, "fingerprint": fingerprint}


def run_attempt(ledger, campaign_dir, repeats=3, seed=42, timeout=300, suite=None, skip_native=False,
                ledger_path=None, candidate_fingerprint=None, hook_timing_pipe=None,
                observe_hook=False, preload_stdin=False):
    validate_ledger(ledger)
    if ledger["campaign_kind"] != "performance":
        raise ValueError("fleet campaign execution is not implemented by this non-paid controller")
    if ledger["attempts_consumed"] >= MAX_ATTEMPTS:
        raise ValueError("maximum five attempts already consumed")
    if not 1 <= timeout <= MAX_ATTEMPT_SECONDS:
        raise ValueError("attempt timeout must be between 1 and 900 seconds")
    if ledger.get("elapsed_total_seconds", 0) + timeout > MAX_CAMPAIGN_SECONDS:
        raise ValueError("campaign deadline exceeds 4500 seconds")
    if ledger["state"] in {"prepare", "repair", "blocked"}:
        transition(ledger, "verify", "candidate provenance inspected before reservation")
    snapshot, observed_fingerprint = candidate_snapshot()
    if candidate_fingerprint and candidate_fingerprint != observed_fingerprint:
        raise ValueError("provided candidate fingerprint does not match inspected source")
    index = ledger["attempts_consumed"] + 1
    candidate = _record_candidate(ledger, snapshot, observed_fingerprint, index)
    selected = suite or expected_suite(ledger)
    expected = expected_suite(ledger)
    if selected not in SUITES or selected != expected:
        raise ValueError(f"next required suite is {expected!r}; refusing to skip coverage with {selected!r}")
    run_id = str(uuid.uuid4())
    attempt_seed = seed + 1 if selected == "confirmation" else seed
    command = fixed_command(Path(__file__).with_name("perf_campaign.py"), repeats, attempt_seed,
                            ledger["campaign_id"], index, selected, skip_native, run_id, hook_timing_pipe,
                            observe_hook, preload_stdin)
    transition(ledger, "reserve", f"attempt {index} reserved for {selected}")
    attempt = {"attempt_index": index, "run_id": run_id, "suite": selected, "started_at_utc": now(),
               "seed": attempt_seed, "mode": "candidate", "commands": [{"argv": command}],
               "candidate": candidate, "assertions": [], "verdict": None}
    ledger["attempts"].append(attempt)
    ledger["attempts_consumed"] = index
    transition(ledger, "run", f"attempt {index} process started")
    if ledger_path:
        atomic_write(Path(ledger_path), ledger)

    unexpected = None
    try:
        completed = run_bounded_process(command, timeout, MAX_OUTPUT_BYTES, cwd=str(REPO_ROOT))
        stdout = completed.pop("stdout", b"")
        stderr = completed.pop("stderr", b"")
        attempt["process"] = {**completed, "stderr": stderr.decode("utf-8", errors="replace")[-4000:]}
        transition(ledger, "observe", "attempt process ended within controller containment")
        if completed.get("error"):
            raise ValueError(completed["error"])
        report = json.loads(stdout.decode("utf-8"))
        validate_report_schema(report)
        if (report["campaign_id"], report["run_id"], report["attempt_index"], report.get("suite")) != (
                ledger["campaign_id"], run_id, index, selected):
            raise ValueError("attempt report identity does not match reservation")
        attempt["raw_report"], attempt["raw_report_sha256"] = report, canonical_hash(report)
        report_fingerprint = canonical_hash(report.get("source_provenance", {}))
        report_source_after = report.get("source_after")
        expected_exit = {"passed": 0, "failed": 1, "not_measured": 3}.get(report["verdict"])
        if completed["exit_code"] != expected_exit:
            raise ValueError("attempt exit code contradicts report verdict")
        if report.get("active_before") != report.get("active_after"):
            raise ValueError("active installation changed during candidate attempt")
        attempt["assertions"], attempt["verdict"] = report["assertions"], report["verdict"]
        controller_source_after, controller_fingerprint_after = candidate_snapshot()
        report_end_fingerprint = canonical_hash(report_source_after) if isinstance(report_source_after, dict) else None
        source_unchanged = (report_fingerprint == observed_fingerprint
                            and report_end_fingerprint == observed_fingerprint
                            and controller_fingerprint_after == observed_fingerprint)
        attempt["supplemental_validation"] = {
            "source_after": controller_source_after,
            "source_after_fingerprint": controller_fingerprint_after,
            "report_source_after_fingerprint": report_end_fingerprint,
            "status": "passed" if source_unchanged else "failed",
            "reason": "" if source_unchanged else "candidate_source_changed_during_attempt",
            "evidence_refs": ["/raw_report/source_provenance", "/raw_report/source_after",
                              "/supplemental_validation/source_after"],
        }
        if not source_unchanged:
            attempt["verdict"] = "failed"
            attempt["process"]["report_validation_error"] = "candidate_source_changed_during_attempt"
    except (UnicodeError, json.JSONDecodeError, ValueError) as exc:
        attempt["verdict"] = "not_measured"
        attempt.setdefault("process", {})["report_validation_error"] = str(exc)
        attempt["ended_at_utc"] = now()
        report = _interrupted_report(ledger, attempt)
        attempt["raw_report"], attempt["raw_report_sha256"] = report, canonical_hash(report)
    except Exception as exc:  # persist the consumed reservation before surfacing programming/runtime faults
        unexpected = exc
        attempt["verdict"] = "not_measured"
        attempt["process"] = {"error": f"controller_exception: {exc}"}
        attempt["ended_at_utc"] = now()
        report = _interrupted_report(ledger, attempt)
        attempt["raw_report"], attempt["raw_report_sha256"] = report, canonical_hash(report)
    finally:
        attempt["ended_at_utc"] = attempt.get("ended_at_utc") or now()
        if ledger["state"] == "run":
            transition(ledger, "observe", "attempt ended without a complete report")
        transition(ledger, "assess", "validated report and reconciled reservation identity")
        ledger["elapsed_total_seconds"] += max(0.0, _elapsed_seconds(attempt["started_at_utc"], attempt["ended_at_utc"]))
        transition(ledger, "cleanup", "owned process tree and output pipes finalized")
        transition(ledger, "decide", "evaluate suite verdict and campaign coverage")
        _decide(ledger, attempt)
        if ledger_path:
            atomic_write(Path(ledger_path), ledger)
    if unexpected:
        raise unexpected
    return ledger


def _decide(ledger, attempt):
    verdict, suite = attempt["verdict"], attempt["suite"]
    if verdict == "passed":
        if suite not in ledger["coverage"]["obtained"]:
            ledger["coverage"]["obtained"].append(suite)
        ledger["coverage"]["missing"] = [s for s in REQUIRED_SUITES if s not in ledger["coverage"]["obtained"]]
        if not ledger["coverage"]["missing"]:
            transition(ledger, "complete", "all required suites passed for one candidate fingerprint")
            ledger["decision"], ledger["verdict"], ledger["terminal_reason"] = "complete", "passed", "coverage_complete"
        elif ledger["attempts_consumed"] >= MAX_ATTEMPTS:
            transition(ledger, "exhausted", "attempt limit reached with suite coverage missing")
            ledger["decision"], ledger["verdict"], ledger["terminal_reason"] = "exhausted", "not_measured", "coverage_missing"
        else:
            transition(ledger, "prepare", "continue with next required suite")
            ledger["decision"], ledger["verdict"], ledger["terminal_reason"] = "next", "not_measured", "coverage_remaining"
    elif verdict == "failed":
        transition(ledger, "repair", "suite assertion failed")
        ledger["decision"], ledger["verdict"], ledger["terminal_reason"] = "needs_repair", "failed", "suite_failed"
    else:
        transition(ledger, "blocked", "required suite evidence not measured")
        ledger["decision"], ledger["verdict"], ledger["terminal_reason"] = "blocked", "not_measured", "required_evidence_missing"


def _elapsed_seconds(started, ended):
    try:
        return (dt.datetime.fromisoformat(ended) - dt.datetime.fromisoformat(started)).total_seconds()
    except (TypeError, ValueError):
        return 0.0


def main(argv=None):
    parser = argparse.ArgumentParser(description="Bounded performance campaign controller")
    parser.add_argument("--campaign-kind", choices=("performance", "fleet"), default="performance")
    parser.add_argument("--campaign-id")
    parser.add_argument("--seed", type=int, default=42)
    parser.add_argument("--repeats", type=int, default=3)
    parser.add_argument("--suite", choices=SUITES, help="must equal the next required suite")
    parser.add_argument("--skip-native", action="store_true")
    parser.add_argument("--candidate-fingerprint")
    parser.add_argument("--hook-timing-pipe", help="explicit campaign-owned live candidate pipe")
    parser.add_argument("--observe-hook", action="store_true")
    parser.add_argument("--preload-stdin", action="store_true")
    parser.add_argument("--campaign-dir")
    parser.add_argument("--resume-ledger")
    parser.add_argument("--timeout", type=int, default=300)
    args = parser.parse_args(argv)
    if not 1 <= args.repeats <= 5 or not 1 <= args.timeout <= MAX_ATTEMPT_SECONDS or args.seed < 0:
        return 2
    if args.resume_ledger:
        ledger_path = Path(args.resume_ledger)
        try:
            ledger = recover_interrupted(validate_ledger(json.loads(ledger_path.read_text(encoding="utf-8"))))
            atomic_write(ledger_path, ledger)
        except (OSError, json.JSONDecodeError, ValueError) as exc:
            print(json.dumps({"error": f"invalid_resume_ledger: {exc}"}))
            return 2
        if args.campaign_id and args.campaign_id != ledger["campaign_id"]:
            print(json.dumps({"error": "campaign_id_mismatch"}))
            return 2
        campaign_dir = ledger_path.parent
    else:
        campaign_dir = Path(args.campaign_dir) if args.campaign_dir else Path(tempfile.mkdtemp(prefix="agent-otel-perf-"))
        campaign_dir.mkdir(parents=True, exist_ok=True)
        ledger_path = campaign_dir / "ledger.json"
        ledger = new_ledger(args.campaign_id, args.campaign_kind, args.seed)
    try:
        validate_ledger(ledger)
        first = True
        authorized_resume = bool(args.resume_ledger and ledger["decision"] in {"needs_repair", "blocked"})
        while ledger["decision"] in {"start", "next"} or authorized_resume:
            authorized_resume = False
            run_attempt(ledger, campaign_dir, args.repeats, args.seed, args.timeout,
                        args.suite if first else None, args.skip_native, ledger_path,
                        args.candidate_fingerprint, args.hook_timing_pipe, args.observe_hook,
                        args.preload_stdin)
            first = False
            if ledger["decision"] != "next":
                break
        atomic_write(ledger_path, ledger)
    except (OSError, ValueError) as exc:
        print(json.dumps({"error": str(exc)}))
        return 4
    print(json.dumps(ledger, indent=2, sort_keys=True))
    if ledger["decision"] == "complete":
        return 0
    if ledger["decision"] == "needs_repair":
        return 4
    return 3


if __name__ == "__main__":
    raise SystemExit(main())
