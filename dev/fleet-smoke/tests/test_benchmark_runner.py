"""Tests for benchmark runner and CLI state machine."""
import json
import subprocess
import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent


class TestBenchmarkRunner(unittest.TestCase):
    def test_plan_output_valid_json(self):
        cmd = [sys.executable, str(HERE / "benchmark.py"), "plan", "--profile", "quick", "--seed", "502"]
        res = subprocess.run(cmd, capture_output=True, text=True, check=True)
        data = json.loads(res.stdout)
        self.assertEqual(data["profile"], "quick")
        self.assertEqual(data["seed"], 502)
        self.assertTrue(len(data["plan_hash"]) > 0)

    def test_preflight_execution(self):
        cmd = [sys.executable, str(HERE / "benchmark.py"), "preflight", "--profile", "fleet"]
        res = subprocess.run(cmd, capture_output=True, text=True, check=True)
        self.assertIn("Preflight complete", res.stdout)

    def test_run_quick_generates_valid_report(self):
        report_path = HERE.parent.parent / "target" / "test-quick-report.json"
        cmd = [
            sys.executable,
            str(HERE / "benchmark.py"),
            "run",
            "--profile",
            "quick",
            "--retain-report",
            str(report_path),
        ]
        res = subprocess.run(cmd, capture_output=True, text=True, check=True)
        self.assertTrue(report_path.exists())

        with open(report_path, "r", encoding="utf-8") as f:
            data = json.load(f)

        self.assertEqual(data["schema"], "aob-repeatable-benchmark/v1")
        self.assertEqual(data["campaign"]["status"], "completed")
        self.assertTrue(len(data["assertions"]) >= 5)

        # Test verify command
        cmd_ver = [sys.executable, str(HERE / "benchmark.py"), "verify", "--report", str(report_path)]
        res_ver = subprocess.run(cmd_ver, capture_output=True, text=True, check=True)
        self.assertIn("Assertions Summary", res_ver.stdout)

        # Test render command
        cmd_ren = [sys.executable, str(HERE / "benchmark.py"), "render", "--report", str(report_path), "--shareable"]
        res_ren = subprocess.run(cmd_ren, capture_output=True, text=True, check=True)
        self.assertIn("# Resultado do benchmark e insumo para melhoria", res_ren.stdout)


if __name__ == "__main__":
    unittest.main()
