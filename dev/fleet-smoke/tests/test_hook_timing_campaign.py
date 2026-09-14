import argparse
import importlib.util
import json
import os
import subprocess
import sys
import unittest
from pathlib import Path
from unittest import mock


MODULE_PATH = os.path.join(os.path.dirname(__file__), "..", "hook_timing_campaign.py")
spec = importlib.util.spec_from_file_location(
    "hook_timing_campaign", os.path.abspath(MODULE_PATH)
)
campaign = importlib.util.module_from_spec(spec)
sys.modules["hook_timing_campaign"] = campaign
spec.loader.exec_module(campaign)


def arguments() -> argparse.Namespace:
    return argparse.Namespace(
        daemon_bin=Path("missing-daemon"),
        hook_bin=Path("missing-hook"),
        iterations=100,
        campaign_timeout_seconds=90.0,
    )


def probe_report(work_ns=(999_999,), *, send_completed=True):
    samples = [
        {
            "index": index,
            "error": None,
            "record": {
                "send_completed": send_completed,
                "work_completed_ns": value,
            },
        }
        for index, value in enumerate(work_ns)
    ]
    disabled_samples = [
        {"index": index, "error": None} for index in range(len(samples))
    ]
    return {
        "schema": "agent-otel-hook-timing/v1",
        "hook_bin": str(Path("candidate-hook").resolve()),
        "hook_sha256": "abc123",
        "iterations": len(samples),
        "valid_samples": len(samples),
        "missing_or_invalid": 0,
        "all_below_1000us": all(value < 1_000_000 for value in work_ns),
        "work_ns": {"max": max(work_ns)},
        "samples": samples,
        "disabled_comparison": {
            "iterations": len(samples),
            "failures": 0,
            "samples": disabled_samples,
        },
    }


class HookTimingCampaignTests(unittest.TestCase):
    def test_argument_defaults_require_explicit_candidate_binaries(self):
        parsed = campaign.parse_args(
            ["--daemon-bin", "candidate-daemon", "--hook-bin", "candidate-hook"]
        )
        self.assertEqual(parsed.daemon_bin, Path("candidate-daemon"))
        self.assertEqual(parsed.hook_bin, Path("candidate-hook"))
        self.assertEqual(parsed.iterations, 100)
        self.assertEqual(parsed.campaign_timeout_seconds, 90.0)

    def test_non_windows_is_not_measured_without_spawning_any_child(self):
        with (
            mock.patch.object(campaign.subprocess, "Popen") as popen,
            mock.patch.object(campaign.subprocess, "run") as subprocess_run,
            mock.patch.object(campaign, "run_bounded_process") as bounded,
            mock.patch.object(campaign, "TraceCollector") as collector,
        ):
            report = campaign.run_campaign(arguments(), system_name="Linux")

        self.assertEqual(report["verdict"], "not_measured")
        self.assertEqual(report["reason"], "windows_inherited_handle_unavailable")
        self.assertFalse(report["isolation"]["child_processes_started"])
        self.assertTrue(report["cleanup"]["complete"])
        popen.assert_not_called()
        subprocess_run.assert_not_called()
        bounded.assert_not_called()
        collector.assert_not_called()

    def test_probe_evaluation_requires_schema_exit_and_strict_assertion(self):
        valid = {
            "error": None,
            "exit_code": 0,
            "stdout": json.dumps(probe_report()).encode(),
        }
        verdict, reason, report = campaign.evaluate_probe(
            valid,
            expected_iterations=1,
            expected_hook=Path("candidate-hook"),
            expected_hook_sha256="abc123",
        )
        self.assertEqual(verdict, "passed")
        self.assertIsNone(reason)
        self.assertTrue(report["all_below_1000us"])

        failed = dict(valid)
        failed["exit_code"] = 1
        failed["stdout"] = json.dumps(probe_report((1_000_000,))).encode()
        verdict, reason, _ = campaign.evaluate_probe(failed)
        self.assertEqual(verdict, "failed")
        self.assertEqual(reason, "hook_timing_assertion_failed")

        invalid = dict(valid)
        invalid["stdout"] = b"not-json"
        verdict, reason, report = campaign.evaluate_probe(invalid)
        self.assertEqual(verdict, "not_measured")
        self.assertIn("probe_json_invalid", reason)
        self.assertIsNone(report)

        invalid["stdout"] = b"[]"
        verdict, reason, report = campaign.evaluate_probe(invalid)
        self.assertEqual(verdict, "not_measured")
        self.assertEqual(reason, "probe_report_invalid")
        self.assertIsNone(report)

    def test_missing_observer_send_or_disabled_sample_is_not_measured(self):
        missing = probe_report()
        missing["samples"] = []
        process = {"error": None, "exit_code": 1, "stdout": json.dumps(missing).encode()}
        verdict, reason, _ = campaign.evaluate_probe(process)
        self.assertEqual(verdict, "not_measured")
        self.assertEqual(reason, "probe_observer_samples_incomplete")

        incomplete_send = probe_report(send_completed=False)
        process["stdout"] = json.dumps(incomplete_send).encode()
        verdict, reason, _ = campaign.evaluate_probe(process)
        self.assertEqual(verdict, "not_measured")
        self.assertEqual(reason, "probe_send_observation_incomplete")

        disabled_failure = probe_report()
        disabled_failure["disabled_comparison"]["failures"] = 1
        process["stdout"] = json.dumps(disabled_failure).encode()
        verdict, reason, _ = campaign.evaluate_probe(process)
        self.assertEqual(verdict, "not_measured")
        self.assertEqual(reason, "probe_disabled_comparison_incomplete")

    def test_probe_hash_must_match_and_watchdog_override_is_temporarily_cleared(self):
        process = {
            "error": None,
            "exit_code": 0,
            "stdout": json.dumps(probe_report()).encode(),
        }
        verdict, reason, _ = campaign.evaluate_probe(
            process, expected_hook_sha256="different"
        )
        self.assertEqual(verdict, "not_measured")
        self.assertEqual(reason, "probe_hook_hash_mismatch")

        with mock.patch.dict(os.environ, {"AGENT_OTEL_WATCHDOG_MS": "99"}):
            with campaign.cleared_watchdog_environment():
                self.assertNotIn("AGENT_OTEL_WATCHDOG_MS", os.environ)
            self.assertEqual(os.environ["AGENT_OTEL_WATCHDOG_MS"], "99")

    def test_daemon_cleanup_escalates_from_terminate_to_kill_with_bounds(self):
        process = mock.Mock()
        process.poll.side_effect = [None, 9]
        process.wait.side_effect = [subprocess.TimeoutExpired("daemon", 2), 9]
        process.returncode = 9

        result = campaign.stop_daemon(process)

        process.terminate.assert_called_once_with()
        process.kill.assert_called_once_with()
        self.assertTrue(result["terminated"])
        self.assertTrue(result["killed"])
        self.assertEqual(result["exit_code"], 9)


if __name__ == "__main__":
    unittest.main()
