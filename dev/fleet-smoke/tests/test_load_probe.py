import importlib.util
import json
import os
import random
import sys
import time
import unittest
from pathlib import Path
from unittest import mock

MODULE = Path(__file__).parents[1] / "load_probe.py"
spec = importlib.util.spec_from_file_location("load_probe", MODULE)
probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe)


class LoadProbeTests(unittest.TestCase):
    def test_seeded_ids_are_deterministic_nonzero_and_unique(self):
        first_rng, second_rng = random.Random(42), random.Random(42)
        first_used, second_used = set(), set()
        first = [probe.next_ids(first_rng, first_used) for _ in range(10)]
        second = [probe.next_ids(second_rng, second_used) for _ in range(10)]
        self.assertEqual(first, second)
        self.assertEqual(len({trace for trace, _ in first}), 10)
        self.assertTrue(all(int(trace, 16) and int(span, 16) for trace, span in first))
        salted_a = probe.next_ids(random.Random("42:run-a"), set())
        salted_b = probe.next_ids(random.Random("42:run-b"), set())
        self.assertNotEqual(salted_a, salted_b)

    def test_worker_exception_becomes_a_failed_client_record(self):
        session = mock.Mock()
        session.hook.side_effect = RuntimeError("fixture")
        result = probe.execute_worker(session, "hook", 7, "a" * 32, "b" * 16, "workspace")
        self.assertFalse(result["response_ok"])
        self.assertEqual(result["trace_id"], "a" * 32)
        self.assertEqual(result["span_id"], "b" * 16)
        self.assertEqual(result["error"], "worker_error:RuntimeError")

    def test_percentiles_are_interpolated_and_empty_is_explicit(self):
        self.assertEqual(probe.percentile([4.0, 1.0, 3.0, 2.0], 50), 2.5)
        self.assertEqual(probe.calc_stats([])["p99"], "not_measured")
        stats = probe.calc_stats(list(range(1, 101)))
        self.assertEqual(stats["p95"], 95.05)
        self.assertEqual(stats["p99"], 99.01)

    def test_fixed_profile_defaults_and_bounds(self):
        argv = ["load_probe.py", "--daemon-bin", os.path.abspath("daemon"),
                "--hook-bin", os.path.abspath("hook")]
        with mock.patch.object(sys, "argv", argv):
            args = probe.parse_args()
        self.assertEqual(args.level_values, [1, 4, 8, 16])
        self.assertEqual(args.event_values, [96, 192, 384, 768])
        self.assertEqual([count / args.seconds for count in args.event_values], [24, 48, 96, 192])

        with mock.patch.object(sys, "argv", argv + ["--events", "96,0,384,768"]), self.assertRaises(SystemExit):
            probe.parse_args()
        with mock.patch.object(sys, "argv", argv + ["--events", "96,192,384,901"]), self.assertRaises(SystemExit):
            probe.parse_args()

    def test_rss_sampler_stops_and_joins(self):
        with mock.patch.object(probe, "get_process_rss", return_value=12345):
            sampler = probe.RssSampler(7, interval=0.001)
            sampler.start()
            time.sleep(0.01)
            samples = sampler.stop()
        self.assertFalse(sampler.is_alive())
        self.assertTrue(samples)
        self.assertTrue(all(sample == 12345 for sample in samples))

    def test_probe_finally_joins_an_active_sampler(self):
        with mock.patch.object(probe, "get_process_rss", return_value=12345):
            sampler = probe.RssSampler(7, interval=0.001)
            sampler.start()
            with mock.patch.object(probe, "_run_load_probe", side_effect=RuntimeError("fixture")):
                with self.assertRaises(RuntimeError):
                    probe.run_load_probe()
        self.assertFalse(sampler.is_alive())
        self.assertNotIn(sampler, probe._ACTIVE_RSS_SAMPLERS)

    def test_integrity_requires_available_hashes_and_active_install(self):
        source = {"candidate_git_revision": "a" * 40, "git_diff_head_sha256": "b" * 64}
        candidates = {"daemon_bin_sha256": "c" * 64, "hook_bin_sha256": "d" * 64}
        active = {"active_json_sha256": "e" * 64,
                  "bin_hashes": {"agent-hook.exe": "f" * 64, "agent-otel-bridge.exe": "1" * 64}}
        self.assertTrue(probe.integrity_available(source, candidates, active))
        self.assertFalse(probe.integrity_available({}, candidates, active))
        self.assertFalse(probe.integrity_available(source, {"daemon_bin_sha256": None}, active))

    def test_load_integrity_rejects_zero_loss_and_bad_parent_evidence(self):
        passed = {"verdict": "passed", "metrics": {"delivered": 4},
                  "trace_evidence": {"valid": True}}
        self.assertTrue(probe.load_integrity_ok(passed, 4))
        self.assertFalse(probe.load_integrity_ok(passed, 0))
        self.assertFalse(probe.load_integrity_ok(passed, 3))
        failed_parent = dict(passed, trace_evidence={"valid": False, "wrong_parent_ids": ["a" * 32]})
        self.assertFalse(probe.load_integrity_ok(failed_parent, 4))

    def test_report_fixture_matches_schema_contract(self):
        schema = json.loads((MODULE.parent / "load-report-v1.schema.json").read_text(encoding="utf-8"))
        sha = "a" * 64
        stats = {"p50": 1.0, "p95": 2.0, "p99": 3.0, "max": 4.0}
        profile = {
            "concurrency": 1, "target_rate_eps": 24.0, "scheduled_interval_ms": 41.667,
            "duration_seconds": 4.0, "configured_seconds": 4.0, "max_events": 900,
            "counts": {"configured": 96, "offered": 96, "admitted": 96, "completed": 96,
                       "delivered": 96, "schedule_missed": 0, "admission_skipped": 0,
                       "worker_errors": 0,
                       "generator_limited": False},
            "rates": {"offered_rate_eps": 24.0, "admitted_rate_eps": 24.0,
                      "completed_rate_eps": 24.0, "delivered_rate_eps": 24.0},
            "percentiles": {"external_hook_durations_ms": stats, "e2e_latencies_ms": stats,
                            "scheduler_lateness_ms": stats},
            "rss_bytes": {"baseline": 100, "peak": 200, "delta": 100},
            "daemon_diagnostics": {}, "verdict": "passed", "assertions": [],
            "trace_evidence": {"valid": True}, "cleanup": {"collector_stopped": True}
        }
        report = {
            "schema": "agent-otel-load/v1", "run_id": "00000000-0000-4000-8000-000000000001",
            "seed": 42, "environment": {}, "source_state": {"before": {}, "after": {}},
            "candidate_hashes": {"before": {}, "after": {}},
            "installed_bridge": {"before": {}, "after": {}},
            "integrity_gates": {"source_intact": True, "candidate_binaries_intact": True,
                                "installed_bridge_intact": True},
            "campaign_duration_seconds": 4.2, "profiles": [profile], "verdict": "passed",
            "limitations": ["fixture"]
        }
        self.assertEqual(set(schema["required"]), set(report))
        try:
            import jsonschema
        except ImportError:
            return
        jsonschema.Draft202012Validator(schema).validate(report)


if __name__ == "__main__":
    unittest.main()
