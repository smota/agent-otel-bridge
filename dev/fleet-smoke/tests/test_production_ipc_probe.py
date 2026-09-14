import sys
from pathlib import Path
import unittest
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from production_ipc_probe import reconcile, clean_shutdown


class IpcReconciliationTests(unittest.TestCase):
    def test_graceful_shutdown_null_forced_requires_exit_and_drain_evidence(self):
        cleanup = {"collector_stopped": True, "fixtures_purged": True, "collector_errors": [],
                   "daemon_shutdown": {"forced": None, "measurement_status": "measured", "exit_code": 0,
                                       "output_bounded": True, "output_drain_complete": True}}
        self.assertTrue(clean_shutdown(cleanup))
        cleanup["daemon_shutdown"]["forced"] = "killed"
        self.assertFalse(clean_shutdown(cleanup))
        cleanup["daemon_shutdown"] = {"forced": None}
        self.assertFalse(clean_shutdown(cleanup))

    def fixture(self):
        event = {"index": 0, "trace_id": "1"*32, "parent_span_id": "2"*16, "send_completed": True}
        report = {"schema": "agent-otel-ipc-load/v1", "planned_events": 1, "offered_events": 1, "attempted_events": 1, "send_completed_events": 1, "send_failed_events": 0, "per_event": [event]}
        span = {"trace_id": "1"*32, "parent_span_id": "2"*16, "span_id": "3"*16}
        return report, span

    def test_exact_delivery(self):
        report, span = self.fixture()
        self.assertTrue(reconcile(report, [span])["valid"])

    def test_missing_duplicate_and_wrong_parent_rejected(self):
        report, span = self.fixture()
        self.assertFalse(reconcile(report, [])["valid"])
        self.assertFalse(reconcile(report, [span, span])["valid"])
        span["parent_span_id"] = "4"*16
        self.assertFalse(reconcile(report, [span])["valid"])

    def test_missing_record_never_infers_watchdog(self):
        report, _ = self.fixture()
        result = reconcile(report, [])
        self.assertEqual(result["missing_observations"][0]["classification"], "unknown_before_receiver")
        report["per_event"][0].update(send_completed=False, stage="Connect", os_code=2)
        result = reconcile(report, [])
        self.assertEqual(result["missing_observations"][0]["classification"], "transport_error")

    def test_failed_send_cannot_pass_even_when_received(self):
        report, span = self.fixture()
        report["per_event"][0].update(send_completed=False, stage="Submit", os_code=5)
        self.assertFalse(reconcile(report, [span])["valid"])

    def test_accounting_and_index_must_match(self):
        for key in ("offered_events", "attempted_events", "send_completed_events"):
            report, span = self.fixture()
            report[key] = 2
            self.assertFalse(reconcile(report, [span])["valid"])
        report, span = self.fixture()
        report["per_event"][0]["index"] = 1
        self.assertFalse(reconcile(report, [span])["valid"])

    def test_report_cannot_reduce_requested_workload(self):
        report, span = self.fixture()
        report["configuration"] = {"events": 1, "concurrency": 999}
        self.assertFalse(reconcile(report, [span], {"events": 400, "concurrency": 4})["valid"])
