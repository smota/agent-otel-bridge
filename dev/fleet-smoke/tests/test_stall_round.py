import tempfile
from pathlib import Path
import sys
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import stall_round
import load_probe
from stall_round import bounded_timeout, failure_clusters, parse_args, reserve, summarize_ipc, write_raw


class StallRoundTests(unittest.TestCase):
    def test_summary_uses_explicit_denominators(self):
        events = [{"trace_id": str(index), "send_completed": index < 6} for index in range(8)]
        emitter = {"offered_events": 10, "attempted_events": 8, "send_failed_events": 2,
                   "achieved_attempt_rate": 80.0, "generator_limited": True,
                   "elapsed_seconds": 0.1, "per_event": events}
        received = [{"trace_id": str(index)} for index in (0, 1, 2, 3, 6)]
        result = summarize_ipc(emitter, {"received": 6, "span_identities": received + [received[0]]}, 0.2)
        self.assertEqual(result["send_failure_fraction_of_attempted"], 2 / 8)
        self.assertEqual(result["receiver_delivery_fraction_of_offered"], 5 / 10)
        self.assertEqual(result["receiver_delivery_fraction_of_send_completed"], 4 / 6)
        self.assertEqual(result["receiver_received_spans_raw"], 6)
        self.assertEqual(result["receiver_delivered_despite_send_failure"], 1)

    def test_failure_clusters_require_consecutive_index_and_same_cause(self):
        events = [
            {"index": 1, "send_completed": False, "stage": "Submit", "os_code": 5,
             "start_offset_us": 10, "end_offset_us": 14},
            {"index": 2, "send_completed": False, "stage": "Submit", "os_code": 5,
             "start_offset_us": 15, "end_offset_us": 22},
            {"index": 3, "send_completed": True},
            {"index": 4, "send_completed": False, "stage": "Deadline", "os_code": 1460,
             "start_offset_us": 30, "end_offset_us": 39},
        ]
        clusters = failure_clusters(events)
        self.assertEqual([(c["start_index"], c["end_index"]) for c in clusters], [(1, 2), (4, 4)])
        self.assertEqual(clusters[0]["raw_failure_durations_us"], [4, 7])

    def test_only_three_sequential_attempts_can_be_reserved(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            with self.assertRaises(ValueError):
                reserve(root, 2)
            for attempt in (1, 2, 3):
                self.assertEqual(reserve(root, attempt)["attempt"], attempt)
            with self.assertRaises(ValueError):
                reserve(root, 4)
            with self.assertRaises(ValueError):
                reserve(root, 3)

    def test_timeout_and_emitter_capacity_are_validated_before_reservation(self):
        common = ["--output-dir", "unused", "--attempt", "1", "--daemon-bin", "daemon",
                  "--emitter", "emitter", "--repetitions", "1"]
        with self.assertRaises(ValueError):
            parse_args(common + ["--campaign-timeout", "0"])
        with self.assertRaises(ValueError):
            parse_args(common + ["--emitter-max-events", "10000"])

    def test_child_timeout_is_capped_by_profile_and_campaign_deadline(self):
        self.assertEqual(bounded_timeout(deadline=200.0, per_child_limit=180.0, now=100.0), 100.0)
        self.assertEqual(bounded_timeout(deadline=500.0, per_child_limit=180.0, now=100.0), 180.0)
        self.assertEqual(bounded_timeout(deadline=100.0, per_child_limit=180.0, now=100.0), 0.0)

    def test_raw_artifact_limit_is_enforced(self):
        with tempfile.TemporaryDirectory() as temp:
            original = stall_round.RAW_ARTIFACT_LIMIT
            stall_round.RAW_ARTIFACT_LIMIT = 8
            try:
                with self.assertRaises(ValueError):
                    write_raw(Path(temp) / "raw.json", {"long": "payload"})
            finally:
                stall_round.RAW_ARTIFACT_LIMIT = original

    def test_real_hook_probe_accepts_fifteen_second_profile(self):
        argv = ["load_probe.py", "--daemon-bin", str(Path("daemon").resolve()),
                "--hook-bin", str(Path("hook").resolve()), "--levels", "1",
                "--events", "750", "--seconds", "15", "--max-events", "750"]
        with mock.patch.object(sys, "argv", argv):
            self.assertEqual(load_probe.parse_args().seconds, 15.0)

    def test_failed_preflight_does_not_consume_attempt(self):
        with tempfile.TemporaryDirectory() as temp:
            output = Path(temp) / "campaign"
            argv = ["--output-dir", str(output), "--attempt", "1", "--daemon-bin", "missing-daemon",
                    "--emitter", "missing-emitter", "--repetitions", "1"]
            with mock.patch.object(stall_round, "snapshot", return_value={}), self.assertRaises(ValueError):
                stall_round.main(argv)
            self.assertFalse((output / "attempt-1.reserved").exists())


if __name__ == "__main__":
    unittest.main()
