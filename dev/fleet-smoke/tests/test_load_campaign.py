import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest import mock
import sys

sys.path.insert(0, str(Path(__file__).parents[1]))
import load_campaign


class CampaignTests(unittest.TestCase):
    def test_running_and_failed_attempts_consume_budget_without_replacement(self):
        ledger = {"attempts": []}
        first = load_campaign.reserve_attempt(ledger)
        second = load_campaign.reserve_attempt(ledger)
        second["status"] = "failed"
        third = load_campaign.reserve_attempt(ledger)
        with self.assertRaises(ValueError):
            load_campaign.reserve_attempt(ledger)
        self.assertIs(ledger["attempts"][0], first)
        self.assertEqual([a["attempt"] for a in ledger["attempts"]], [1, 2, 3])
        self.assertEqual(len({a["run_id"] for a in ledger["attempts"]}), 3)

    def test_probe_with_zero_exit_but_invalid_json_cannot_claim_report(self):
        with tempfile.TemporaryDirectory() as directory, mock.patch.object(
            load_campaign, "run_bounded_process", return_value={"exit_code": 0, "stdout": b"broken", "stderr": b""}
        ):
            result = load_campaign.probe("fake", ["fake"], 1, Path(directory), 1)
            self.assertFalse(result["json_valid"])
            self.assertIsNone(result["reported_verdict"])


if __name__ == "__main__":
    unittest.main()
