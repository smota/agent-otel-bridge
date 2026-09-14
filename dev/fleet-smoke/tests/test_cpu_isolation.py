"""Owned-child affinity must be verified without changing the test coordinator."""
import os
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from perf_campaign import available_cpu_ids, run_subprocess_json
from stall_round import affinity_plan, parse_args


class CpuIsolationTests(unittest.TestCase):
    @unittest.skipUnless(os.name == "nt" or hasattr(os, "sched_setaffinity"), "affinity unavailable")
    def test_owned_child_mask_is_verified_and_coordinator_unchanged(self):
        original = available_cpu_ids()
        selected = original[:1]
        result = run_subprocess_json([sys.executable, "-c", "print('{}')"], 5, cpus=selected)
        self.assertIsNone(result["error"], result)
        self.assertEqual(result["affinity"]["status"], "applied")
        if os.name == "nt":
            self.assertEqual(result["affinity"]["observed_mask"], 1 << selected[0])
            self.assertEqual(result["affinity"]["timing"], "before_resume")
        else:
            self.assertEqual(result["affinity"]["observed_cpus"], selected)
        self.assertEqual(result["process_tree_cleanup"], "complete")
        self.assertEqual(available_cpu_ids(), original)

    @unittest.skipUnless(os.name == "nt", "Windows suspended-child guarantee")
    def test_invalid_mask_never_executes_child_and_is_cleaned_up(self):
        with tempfile.TemporaryDirectory() as directory:
            marker = str(Path(directory) / "must-not-exist")
            result = run_subprocess_json(
                [sys.executable, "-c", "import pathlib,sys; pathlib.Path(sys.argv[1]).touch()", marker],
                5, cpus=[-1],
            )
            self.assertEqual(result["error"], "process_affinity_unavailable")
            self.assertEqual(result["process_tree_cleanup"], "complete")
            self.assertFalse(Path(marker).exists())

    def test_partition_is_disjoint_and_uses_only_allowed_cpus(self):
        plan = affinity_plan(True)
        if plan["status"] == "unsupported":
            self.skipTest(plan["error"])
        self.assertFalse(set(plan["daemon_cpus"]) & set(plan["emitter_cpus"]))
        self.assertEqual(sorted(plan["daemon_cpus"] + plan["emitter_cpus"]), available_cpu_ids())

    def test_real_hook_is_not_silently_claimed_isolated(self):
        with self.assertRaises(ValueError):
            parse_args(["--output-dir", "unused", "--attempt", "2", "--daemon-bin", "daemon",
                        "--emitter", "emitter", "--hook-bin", "hook", "--isolate-cpus"])


if __name__ == "__main__":
    unittest.main()
