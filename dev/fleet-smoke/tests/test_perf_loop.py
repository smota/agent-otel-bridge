import importlib.util
import json
import pathlib
import sys
import tempfile
import unittest
from unittest import mock

ROOT = pathlib.Path(__file__).parents[1]
spec = importlib.util.spec_from_file_location("perf_loop", ROOT / "perf_loop.py")
perf_loop = importlib.util.module_from_spec(spec)
sys.modules["perf_loop"] = perf_loop
spec.loader.exec_module(perf_loop)


SOURCE = {"candidate_git_revision": "a" * 40, "is_dirty": True,
          "git_diff_head_sha256": "b" * 64, "dev_fleet_smoke_source_sha256": {}}


def assertion(status="passed"):
    return {"id": "a1", "requirement_id": "R09", "implementation_refs": [], "test_id": "t",
            "boundary": "b", "unit": "count", "operator": "equal", "threshold": 0,
            "samples": 1, "observed": 0, "status": status, "reason": "", "evidence_refs": []}


def report(campaign_id, run_id, index, suite, verdict="passed", source=SOURCE):
    return {"record_type": "attempt", "schema_version": 1,
            "spec_version": perf_loop.SPEC_VERSION, "campaign_id": campaign_id, "run_id": run_id,
            "attempt_index": index, "mode": "candidate", "suite": suite,
            "started_at_utc": perf_loop.now(), "ended_at_utc": perf_loop.now(), "seed": 42,
            "environment": {}, "candidate": {"revision": "a" * 40, "dirty": True,
            "diff_digest": "b" * 64, "binaries": []}, "active_before": {}, "active_after": {},
            "commands": [], "assertions": [assertion(verdict)], "trace_evidence": {"status": "not_checked"},
            "cleanup": {"status": "complete"}, "verdict": verdict,
            "source_provenance": source, "source_after": source}


class TestPerfLoop(unittest.TestCase):
    def test_ledger_has_one_shared_five_attempt_limit_and_required_suite_order(self):
        ledger = perf_loop.new_ledger("campaign-1")
        self.assertEqual(ledger["max_attempts"], 5)
        self.assertEqual(ledger["coverage"]["required"], list(perf_loop.SUITES))
        self.assertEqual(perf_loop.expected_suite(ledger), "regression")

    def test_atomic_write_and_schema_valid_resume(self):
        ledger = perf_loop.new_ledger("campaign-1")
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / "ledger.json"
            perf_loop.atomic_write(path, ledger)
            loaded = perf_loop.validate_ledger(json.loads(path.read_text(encoding="utf-8")))
        self.assertEqual(loaded["campaign_id"], "campaign-1")

    def test_schema_or_hash_tampering_rejects_resume(self):
        ledger = perf_loop.new_ledger("campaign-1")
        raw = report("campaign-1", "r1", 1, "regression")
        attempt = {"attempt_index": 1, "run_id": "r1", "suite": "regression",
                   "started_at_utc": perf_loop.now(), "ended_at_utc": perf_loop.now(), "seed": 42,
                   "mode": "candidate", "commands": [], "candidate": raw["candidate"], "assertions": [],
                   "verdict": "passed", "raw_report": raw, "raw_report_sha256": "0" * 64}
        ledger["attempts"], ledger["attempts_consumed"] = [attempt], 1
        with self.assertRaisesRegex(ValueError, "hash"):
            perf_loop.validate_ledger(ledger)

    def test_fixed_command_passes_reserved_run_identity(self):
        command = perf_loop.fixed_command(ROOT / "perf_campaign.py", 3, 42, "c", 2, "faults", True, "r")
        self.assertEqual(command[command.index("--suite") + 1], "faults")
        self.assertEqual(command[command.index("--run-id") + 1], "r")
        self.assertIn("--skip-native", command)

    def test_fixed_command_forwards_delivery_diagnostics(self):
        command = perf_loop.fixed_command(ROOT / "perf_campaign.py", 3, 42, "c", 2, "regression",
                                          run_id="r", observe_hook=True, preload_stdin=True)
        self.assertIn("--observe-hook", command)
        self.assertIn("--preload-stdin", command)

    def test_refuses_skipping_required_suite(self):
        ledger = perf_loop.new_ledger("campaign-1")
        with tempfile.TemporaryDirectory() as directory, self.assertRaisesRegex(ValueError, "refusing to skip"):
            perf_loop.run_attempt(ledger, pathlib.Path(directory), suite="performance")

    @mock.patch.object(perf_loop, "candidate_snapshot", return_value=(SOURCE, perf_loop.canonical_hash(SOURCE)))
    @mock.patch.object(perf_loop, "run_bounded_process", side_effect=RuntimeError("simulated crash"))
    def test_crash_consumes_persisted_reservation_with_hashed_partial_report(self, _run, _snapshot):
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / "ledger.json"
            ledger = perf_loop.new_ledger("campaign-1")
            with self.assertRaises(RuntimeError):
                perf_loop.run_attempt(ledger, pathlib.Path(directory), ledger_path=path)
            saved = perf_loop.validate_ledger(json.loads(path.read_text(encoding="utf-8")))
        self.assertEqual(saved["attempts_consumed"], 1)
        self.assertEqual(saved["state"], "blocked")
        self.assertEqual(saved["attempts"][0]["verdict"], "not_measured")

    def test_recover_interrupted_consumes_without_new_attempt(self):
        ledger = perf_loop.new_ledger("campaign-1")
        candidate = {"revision": None, "dirty": None, "diff_digest": None, "binaries": []}
        ledger["attempts"] = [{"attempt_index": 1, "run_id": "r1", "suite": "regression",
                               "started_at_utc": perf_loop.now(), "seed": 42, "mode": "candidate",
                               "commands": [], "candidate": candidate, "assertions": [], "verdict": None}]
        ledger["attempts_consumed"], ledger["state"] = 1, "run"
        recovered = perf_loop.recover_interrupted(ledger)
        self.assertEqual(recovered["attempts_consumed"], 1)
        self.assertEqual(recovered["state"], "repair")
        perf_loop.validate_ledger(recovered)

    @mock.patch.object(perf_loop, "candidate_snapshot", return_value=(SOURCE, perf_loop.canonical_hash(SOURCE)))
    @mock.patch.object(perf_loop, "run_bounded_process")
    def test_passed_suite_advances_coverage_but_not_campaign_pass(self, run, _snapshot):
        def invoke(command, *_args, **_kwargs):
            run_id = command[command.index("--run-id") + 1]
            raw = report("campaign-1", run_id, 1, "regression")
            return {"exit_code": 0, "error": None, "process_tree_cleanup": "complete",
                    "stdout": json.dumps(raw).encode(), "stderr": b""}
        run.side_effect = invoke
        with tempfile.TemporaryDirectory() as directory:
            ledger = perf_loop.run_attempt(perf_loop.new_ledger("campaign-1"), pathlib.Path(directory))
        self.assertEqual(ledger["decision"], "next")
        self.assertEqual(ledger["coverage"]["obtained"], ["regression"])
        self.assertNotEqual(ledger["verdict"], "passed")

    @mock.patch.object(perf_loop, "run_bounded_process")
    def test_post_child_source_change_rejects_but_preserves_raw_report(self, run):
        changed = dict(SOURCE)
        changed["git_diff_head_sha256"] = "c" * 64
        start_hash, changed_hash = perf_loop.canonical_hash(SOURCE), perf_loop.canonical_hash(changed)

        def invoke(command, *_args, **_kwargs):
            run_id = command[command.index("--run-id") + 1]
            raw = report("campaign-1", run_id, 1, "regression")
            return {"exit_code": 0, "error": None, "process_tree_cleanup": "complete",
                    "stdout": json.dumps(raw).encode(), "stderr": b""}

        run.side_effect = invoke
        with mock.patch.object(perf_loop, "candidate_snapshot",
                               side_effect=[(SOURCE, start_hash), (changed, changed_hash)]):
            with tempfile.TemporaryDirectory() as directory:
                ledger = perf_loop.run_attempt(perf_loop.new_ledger("campaign-1"), pathlib.Path(directory))
        attempt = ledger["attempts"][0]
        self.assertEqual(attempt["verdict"], "failed")
        self.assertEqual(attempt["supplemental_validation"]["status"], "failed")
        self.assertEqual(attempt["raw_report"]["verdict"], "passed")
        self.assertEqual(attempt["raw_report_sha256"], perf_loop.canonical_hash(attempt["raw_report"]))

    def test_candidate_change_invalidates_prior_suite_coverage(self):
        ledger = perf_loop.new_ledger("campaign-1")
        ledger["candidate_fingerprint"] = "old"
        ledger["coverage"]["obtained"] = ["regression"]
        ledger["coverage"]["missing"] = ["faults", "performance", "confirmation"]
        ledger["state"] = "prepare"
        perf_loop._record_candidate(ledger, SOURCE, "new", 2)
        self.assertEqual(ledger["coverage"]["obtained"], [])
        self.assertEqual(ledger["coverage"]["missing"], list(perf_loop.SUITES))

    @mock.patch.object(perf_loop, "run_attempt")
    def test_explicit_resume_reenters_needs_repair_once(self, run_attempt):
        ledger = perf_loop.new_ledger("campaign-1")
        raw = report("campaign-1", "r1", 1, "regression", "failed")
        attempt = {"attempt_index": 1, "run_id": "r1", "suite": "regression",
                   "started_at_utc": raw["started_at_utc"], "ended_at_utc": raw["ended_at_utc"], "seed": 42,
                   "mode": "candidate", "commands": [], "candidate": raw["candidate"], "assertions": raw["assertions"],
                   "verdict": "failed", "raw_report": raw, "raw_report_sha256": perf_loop.canonical_hash(raw)}
        ledger["attempts"], ledger["attempts_consumed"] = [attempt], 1
        ledger["state"], ledger["decision"], ledger["verdict"] = "repair", "needs_repair", "failed"
        run_attempt.side_effect = lambda current, *_args, **_kwargs: current.update(
            {"decision": "blocked", "state": "blocked", "verdict": "not_measured"}) or current
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / "ledger.json"
            perf_loop.atomic_write(path, ledger)
            with mock.patch("builtins.print"):
                exit_code = perf_loop.main(["--resume-ledger", str(path)])
        self.assertEqual(exit_code, 3)
        run_attempt.assert_called_once()


if __name__ == "__main__":
    unittest.main()
