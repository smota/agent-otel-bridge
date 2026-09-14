import importlib.util
import os
import json
import pathlib
import tempfile
import time
import sys
import unittest

MODULE_PATH = os.path.join(os.path.dirname(__file__), "..", "perf_campaign.py")
spec = importlib.util.spec_from_file_location("perf_campaign", os.path.abspath(MODULE_PATH))
perf_campaign = importlib.util.module_from_spec(spec)
sys.modules["perf_campaign"] = perf_campaign
spec.loader.exec_module(perf_campaign)


def valid_ipc():
    return {"overall_passed": True, "results": [
        {"msg_type": kind, "failure_count": 0, "expected_rounds": 1000,
         "success_count": 1000, "latency_stats": {"count": 1000, "p99_us": 100}}
        for kind in ("HookPayload", "HookPayloadWithContext")]}


def valid_delivery():
    run = {"eval": {"expected_ids": ["a"], "received_spans": [("a", "b")]},
           "collector_errors": [], "clients": [{"trace_id": "a", "response_ok": True}]}
    return {"scenarios": {profile: {"verdict": "passed", "runs": [run]}
                          for profile in ("debug", "release")}}


class TestPerfCampaign(unittest.TestCase):
    def test_compose_overall_failed_when_any_failed(self):
        micro = {
            "runs": [{
                "evaluation": {
                    "verdicts": [
                        {"metric": "parser_pure_json_span_throughput", "verdict": "failed", "value": 42000.0},
                        {"metric": "hook_binary_size", "verdict": "passed", "value": 150016.0},
                    ]
                }
            }]
        }
        ipc = {"overall_passed": True, "results": [{"msg_type": "HookPayload", "failure_count": 0}]}
        delivery = {"scenarios": {"debug": {"verdict": "passed"}, "release": {"verdict": "passed"}}}
        res = perf_campaign.compose_campaign_results(micro, ipc, delivery)
        self.assertEqual(res["campaign_verdict"], "failed")
        self.assertFalse(res["overall_passed"])

    def test_compose_overall_not_measured_when_required_unmeasured(self):
        micro = {
            "runs": [{
                "evaluation": {
                    "verdicts": [
                        {"metric": "parser_pure_json_span_throughput", "verdict": "passed", "value": 60000.0},
                        {"metric": "hook_binary_size", "verdict": "passed", "value": 150016.0},
                    ]
                }
            }]
        }
        ipc = {"overall_passed": True, "results": [{"msg_type": "HookPayload", "failure_count": 0}]}
        delivery = {"scenarios": {"debug": {"verdict": "passed"}, "release": {"verdict": "passed"}}}
        ipc = valid_ipc()
        delivery = valid_delivery()
        # hook_internal remains not_measured -> overall must be not_measured
        res = perf_campaign.compose_campaign_results(micro, ipc, delivery)
        self.assertEqual(res["campaign_verdict"], "not_measured")
        self.assertFalse(res["overall_passed"])

    def test_compose_missing_native_marks_not_measured(self):
        micro = {
            "runs": [{
                "evaluation": {
                    "verdicts": [
                        {"metric": "hook_binary_size", "verdict": "passed", "value": 150016.0}
                    ]
                }
            }]
        }
        res = perf_campaign.compose_campaign_results(micro, None, None, skip_native=True)
        self.assertEqual(res["campaign_verdict"], "not_measured")
        metrics = {a["metric"]: a["verdict"] for a in res["effective_assertions"]}
        self.assertEqual(metrics["ipc_roundtrip_p99_us"], "not_measured")
        self.assertEqual(metrics["concurrent_event_delivery_loss"], "not_measured")

    def test_compose_failed_native_delivery(self):
        micro = {
            "runs": [{
                "evaluation": {
                    "verdicts": [
                        {"metric": "hook_binary_size", "verdict": "passed", "value": 150016.0}
                    ]
                }
            }]
        }
        ipc = {"overall_passed": True, "results": [{"msg_type": "HookPayload", "failure_count": 0}]}
        delivery = {"scenarios": {"debug": {"verdict": "failed"}, "release": {"verdict": "passed"}}}
        res = perf_campaign.compose_campaign_results(micro, ipc, delivery)
        self.assertEqual(res["campaign_verdict"], "failed")
        self.assertFalse(res["overall_passed"])

    def test_compose_ipc_failures_in_results(self):
        micro = {
            "runs": [{
                "evaluation": {
                    "verdicts": [
                        {"metric": "hook_binary_size", "verdict": "passed", "value": 150016.0}
                    ]
                }
            }]
        }
        ipc = {"overall_passed": True, "results": [{"msg_type": "HookPayload", "failure_count": 2}]}
        delivery = {"scenarios": {"debug": {"verdict": "passed"}, "release": {"verdict": "passed"}}}
        res = perf_campaign.compose_campaign_results(micro, ipc, delivery)
        self.assertEqual(res["campaign_verdict"], "failed")
        self.assertFalse(res["overall_passed"])

    def test_prior_failed_run_is_not_hidden_by_later_pass(self):
        micro = {"runs": [{"evaluation": {"verdicts": [{"metric": "parser", "verdict": verdict}]}}
                          for verdict in ("failed", "passed")]}
        result = perf_campaign.compose_campaign_results(micro, valid_ipc(), valid_delivery())
        self.assertEqual(result["campaign_verdict"], "failed")

    def test_native_counts_override_declared_success(self):
        ipc = valid_ipc()
        ipc["results"][0]["success_count"] = 0
        self.assertFalse(perf_campaign.valid_ipc_counts(ipc["results"]))
        delivery = valid_delivery()
        delivery["scenarios"]["debug"]["runs"][0]["eval"]["received_spans"] = []
        self.assertEqual(perf_campaign.verified_delivery_verdict(delivery["scenarios"]["debug"]), "failed")

    def test_ipc_results_are_separate_measured_assertions_with_raw_pointers(self):
        composed = perf_campaign.compose_campaign_results({}, valid_ipc(), None, suite="regression")
        ipc = [item for item in composed["effective_assertions"] if item["metric"].startswith("ipc_roundtrip_Hook")]
        self.assertEqual(len(ipc), 2)
        self.assertEqual({item["value"] for item in ipc}, {100})
        self.assertTrue(all(item["threshold"] == 3000.0 and item["count"] == 1000 for item in ipc))
        self.assertTrue(all(item["requirement_id"] == "R08" and item["evidence_refs"] for item in ipc))

    def test_normalization_preserves_requirement_and_evidence(self):
        normalized = perf_campaign.normalize_assertions([{
            "metric": "ipc_roundtrip_HookPayload_p99_us", "verdict": "passed", "value": 100.4,
            "unit": "microseconds", "threshold": 3000.0, "mode": "strictly_less", "count": 1000,
            "requirement_id": "R08", "evidence_refs": ["/raw_reports/ipc/results/0/latency_stats/p99_us"],
        }])[0]
        self.assertEqual(normalized["observed"], 100.4)
        self.assertEqual(normalized["samples"], 1000)
        self.assertEqual(normalized["requirement_id"], "R08")
        self.assertTrue(normalized["evidence_refs"])

    def test_regression_hook_and_delivery_are_diagnostic_not_acceptance(self):
        composed = perf_campaign.compose_campaign_results({}, valid_ipc(), valid_delivery(), suite="regression")
        diagnostics = [item for item in composed["effective_assertions"] if item.get("diagnostic")]
        self.assertEqual({item["metric"] for item in diagnostics},
                         {"hook_internal_execution_duration_us", "concurrent_event_delivery_loss"})
        self.assertTrue(all(item["required"] is False for item in diagnostics))

    def test_preload_delivery_cannot_satisfy_required_acceptance(self):
        composed = perf_campaign.compose_campaign_results(None, None, valid_delivery(), suite="faults",
                                                          delivery_diagnostic=True)
        required = [item for item in composed["effective_assertions"]
                    if item["metric"] == "concurrent_event_delivery_loss" and item.get("required")]
        self.assertEqual(required[0]["verdict"], "not_measured")

    def test_source_change_is_a_required_failed_integrity_assertion(self):
        composed = {"effective_assertions": [], "campaign_verdict": "passed", "overall_passed": True}
        before = {"candidate_git_revision": "a", "untracked_source_sha256": {"x": "1"}}
        after = {"candidate_git_revision": "a", "untracked_source_sha256": {"x": "2"}}
        perf_campaign.append_attempt_integrity_assertions(
            composed, {"verified": True}, {"verified": True}, True, before, after)
        source_assertion = next(item for item in composed["effective_assertions"]
                                if item["metric"] == "candidate_source_unchanged")
        self.assertEqual(source_assertion["requirement_id"], "R10")
        self.assertEqual(source_assertion["verdict"], "failed")
        self.assertEqual(composed["campaign_verdict"], "failed")

    def test_fault_suite_reports_unimplemented_fault_probes_not_measured(self):
        result = perf_campaign.compose_campaign_results(None, None, valid_delivery(), suite="faults")
        statuses = {item["metric"]: item["verdict"] for item in result["effective_assertions"]}
        self.assertEqual(statuses["concurrent_event_delivery_loss"], "passed")
        self.assertEqual(statuses["exporter_retry_faults"], "not_measured")
        self.assertEqual(result["campaign_verdict"], "not_measured")

    def test_hook_timing_requires_successful_transport_samples(self):
        timing = {"iterations": 1, "valid_samples": 1, "all_below_1000us": True,
                  "work_ns": {"p99": 10}, "samples": [{"record": {"send_completed": False}}]}
        item = perf_campaign._hook_timing_assertion(timing)
        self.assertEqual(item["verdict"], "failed")

    def test_schema_rejects_attempt_missing_required_suite(self):
        invalid = {"record_type": "attempt", "schema_version": 1}
        with self.assertRaises(ValueError):
            perf_campaign.validate_report_schema(invalid)

    def test_bounded_runner_stops_on_output_limit_without_unbounded_capture(self):
        command = [sys.executable, "-c", "import sys; sys.stdout.buffer.write(b'x'*1048576); sys.stdout.flush()"]
        started = time.monotonic()
        result = perf_campaign.run_bounded_process(command, 5, output_limit=1024)
        self.assertEqual(result["error"], "output_limit_exceeded")
        self.assertLessEqual(len(result["stdout"]) + len(result["stderr"]), 1024)
        self.assertLess(time.monotonic() - started, 5)

    def test_process_tree_cleanup_closes_pipes_held_by_grandchild(self):
        child_code = ("import subprocess,sys; "
                      "subprocess.Popen([sys.executable,'-c','import time; time.sleep(30)'])")
        started = time.monotonic()
        result = perf_campaign.run_bounded_process([sys.executable, "-c", child_code], 5)
        self.assertEqual(result["process_tree_cleanup"], "complete")
        self.assertLess(time.monotonic() - started, 4)


if __name__ == "__main__":
    unittest.main()
