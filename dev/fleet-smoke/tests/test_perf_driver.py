import importlib.util
import io
import json
import os
import sys
import tempfile
import subprocess
from pathlib import Path
import unittest
from unittest.mock import patch

MODULE_PATH = os.path.join(os.path.dirname(__file__), "..", "perf_driver.py")
spec = importlib.util.spec_from_file_location("perf_driver", os.path.abspath(MODULE_PATH))
perf_driver = importlib.util.module_from_spec(spec)
sys.modules["perf_driver"] = perf_driver
spec.loader.exec_module(perf_driver)


class TestPerformanceDriver(unittest.TestCase):
    def test_untracked_product_source_changes_candidate_provenance(self):
        with tempfile.TemporaryDirectory() as root:
            subprocess.run(["git", "init", "--quiet", root], check=True)
            source = Path(root) / "crates" / "new component.rs"
            source.parent.mkdir()
            source.write_text("first", encoding="utf-8")
            first = perf_driver.inspect_source_state(root)["untracked_source_sha256"]
            source.write_text("second", encoding="utf-8")
            second = perf_driver.inspect_source_state(root)["untracked_source_sha256"]
            self.assertIn("crates/new component.rs", first)
            self.assertNotEqual(first, second)

    def test_evaluate_metric_threshold_exact_boundaries(self):
        res_eq = perf_driver.evaluate_metric_threshold(
            50000.0, 50000.0, "strictly_greater", "parser_sps", "norm", "impl", "spans/s"
        )
        self.assertEqual(res_eq["verdict"], "failed")

        res_below = perf_driver.evaluate_metric_threshold(
            49999.999, 50000.0, "strictly_greater", "parser_sps", "norm", "impl", "spans/s"
        )
        self.assertEqual(res_below["verdict"], "failed")

        res_above = perf_driver.evaluate_metric_threshold(
            50000.001, 50000.0, "strictly_greater", "parser_sps", "norm", "impl", "spans/s"
        )
        self.assertEqual(res_above["verdict"], "passed")

        res_less_eq = perf_driver.evaluate_metric_threshold(
            150.0, 150.0, "strictly_less", "harvest_us", "norm", "impl", "us"
        )
        self.assertEqual(res_less_eq["verdict"], "failed")

        res_less_ok = perf_driver.evaluate_metric_threshold(
            149.999, 150.0, "strictly_less", "harvest_us", "norm", "impl", "us"
        )
        self.assertEqual(res_less_ok["verdict"], "passed")

    def test_invalid_mode_rejected(self):
        res = perf_driver.evaluate_metric_threshold(
            100.0, 150.0, "typo_mode", "test", "norm", "impl", "us"
        )
        self.assertEqual(res["verdict"], "failed")
        self.assertIn("invalid_evaluation_mode", res["reason"])

    def test_invalid_negative_boolean_nonfinite(self):
        for invalid in [True, False, -1.0, float("nan"), float("inf"), float("-inf")]:
            res = perf_driver.evaluate_metric_threshold(
                invalid, 150.0, "strictly_less", "test", "norm", "impl", "us"
            )
            self.assertEqual(res["verdict"], "failed")
            clean = perf_driver.sanitize_for_json(res)
            self.assertFalse(isinstance(clean["value"], float) and (clean["value"] != clean["value"] or clean["value"] > 1e300))

    def test_active_manifest_valid_and_tampered(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            bin_dir = os.path.join(tmpdir, "bin")
            os.makedirs(bin_dir, exist_ok=True)
            hook_file = os.path.join(bin_dir, "agent-hook.exe")
            content = b"fake-binary-content-12345"
            with open(hook_file, "wb") as f:
                f.write(content)
            hook_hash = perf_driver.compute_sha256(hook_file)

            manifest_data = {
                "version": "0.5.1",
                "git_commit": "a27470c1234567",
                "version_id": "0.5.1-a27470c",
                "binaries": [
                    {
                        "filename": "agent-hook.exe",
                        "sha256": hook_hash,
                        "size_bytes": len(content),
                    }
                ],
            }
            manifest_path = os.path.join(tmpdir, "active.json")
            with open(manifest_path, "w", encoding="utf-8") as f:
                json.dump(manifest_data, f)

            res = perf_driver.verify_active_manifest(manifest_path)
            self.assertTrue(res["verified"])
            self.assertEqual(res["version"], "0.5.1")

            with open(hook_file, "wb") as f:
                f.write(b"tampered-content")
            res_tampered = perf_driver.verify_active_manifest(manifest_path)
            self.assertFalse(res_tampered["verified"])

    def test_active_manifest_schema_rejections(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            manifest_path = os.path.join(tmpdir, "active.json")
            for bad in [{}, {"binaries": []}, {"version": "0.5.1", "binaries": []}, []]:
                with open(manifest_path, "w", encoding="utf-8") as f:
                    json.dump(bad, f)
                res = perf_driver.verify_active_manifest(manifest_path)
                self.assertFalse(res["verified"])

    def test_throughput_metric_validation(self):
        valid_metric = {
            "iterations": 50000,
            "elapsed_secs": 0.8,
            "ops_per_sec": 62500.0,
            "latency_stats": {"count": 50000, "p99_us": 18.0},
        }
        res = perf_driver.evaluate_throughput_metric(valid_metric, "p_pure", "norm", "impl")
        self.assertEqual(res["verdict"], "passed")

        mismatched_metric = dict(valid_metric)
        mismatched_metric["latency_stats"] = {"count": 0}
        res_mismatch = perf_driver.evaluate_throughput_metric(mismatched_metric, "p_pure", "norm", "impl")
        self.assertEqual(res_mismatch["verdict"], "failed")

        zero_iter = dict(valid_metric)
        zero_iter["iterations"] = 0
        res_zero = perf_driver.evaluate_throughput_metric(zero_iter, "p_pure", "norm", "impl")
        self.assertEqual(res_zero["verdict"], "failed")

    def test_overall_verdict_rules(self):
        base_report = {
            "fixture_conversation_id": "performance-fixture",
            "parser_pure_json_span": {
                "iterations": 1000,
                "elapsed_secs": 0.01,
                "ops_per_sec": 100000.0,
                "latency_stats": {"count": 1000, "p99_us": 10.0},
            },
            "legacy_frame_0x01_pipeline": {
                "iterations": 1000,
                "elapsed_secs": 0.01,
                "ops_per_sec": 100000.0,
                "latency_stats": {"count": 1000, "p99_us": 10.0},
            },
            "envelope_0x04_pipeline": {
                "iterations": 1000,
                "elapsed_secs": 0.01,
                "ops_per_sec": 100000.0,
                "latency_stats": {"count": 1000, "p99_us": 10.0},
            },
            "context_harvest": [
                {"fixture": "git_repo", "iterations": 100, "stats": {"count": 100, "p99_us": 80.0}},
                {"fixture": "git_worktree", "iterations": 100, "stats": {"count": 100, "p99_us": 90.0}},
                {"fixture": "no_git", "iterations": 100, "stats": {"count": 100, "p99_us": 20.0}},
            ],
            "not_measured": perf_driver.REQUIRED_WINDOWS_OBSERVATIONS,
        }
        hook_meta = {"exists": True, "size_evaluation": {"verdict": "passed", "metric": "hook_binary_size"}}

        eval_res = perf_driver.evaluate_full_run(base_report, hook_meta)
        self.assertEqual(eval_res["overall_verdict"], "not_measured")
        self.assertFalse(eval_res["overall_passed"])
        self.assertFalse(eval_res["has_failures"])
        self.assertTrue(eval_res["has_unmeasured_required"])

        failing_report = dict(base_report)
        failing_report["context_harvest"] = []
        eval_fail = perf_driver.evaluate_full_run(failing_report, hook_meta)
        self.assertEqual(eval_fail["overall_verdict"], "failed")
        self.assertFalse(eval_fail["overall_passed"])
        self.assertTrue(eval_fail["has_failures"])

    @patch("sys.stdout", new_callable=io.StringIO)
    @patch("sys.stderr", new_callable=io.StringIO)
    def test_run_driver_timeout_branch(self, _stderr, stdout):
        with patch("subprocess.run", side_effect=perf_driver.subprocess.TimeoutExpired(cmd="example", timeout=5)):
            with patch.object(perf_driver, "inspect_binary", return_value={"exists": False}):
                with patch.object(perf_driver, "verify_active_manifest", return_value={"verified": False}):
                    exit_code = perf_driver.run_driver(
                        example_bin="mock_example.exe",
                        hook_bin="mock_hook.exe",
                        debug_daemon_bin=None,
                        release_daemon_bin=None,
                        manifest_path=None,
                        repeats=3,
                        timeout_secs=5,
                    )
                    self.assertEqual(exit_code, 1)
                    self.assertTrue(json.loads(stdout.getvalue())["aborted_early"])

    @patch("sys.stdout", new_callable=io.StringIO)
    @patch("sys.stderr", new_callable=io.StringIO)
    def test_run_driver_oserror_branch(self, _stderr, stdout):
        with patch("subprocess.run", side_effect=OSError("Exec format error")):
            with patch.object(perf_driver, "inspect_binary", return_value={"exists": False}):
                with patch.object(perf_driver, "verify_active_manifest", return_value={"verified": False}):
                    exit_code = perf_driver.run_driver(
                        example_bin="mock_example.exe",
                        hook_bin="mock_hook.exe",
                        debug_daemon_bin=None,
                        release_daemon_bin=None,
                        manifest_path=None,
                        repeats=3,
                        timeout_secs=5,
                    )
                    self.assertEqual(exit_code, 1)
                    self.assertEqual(json.loads(stdout.getvalue())["completed_repeats"], 1)


if __name__ == "__main__":
    unittest.main()
