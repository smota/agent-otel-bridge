import importlib.util
import json
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

MODULE = Path(__file__).parents[1] / "architecture_suite.py"
spec = importlib.util.spec_from_file_location("architecture_suite", MODULE)
suite = importlib.util.module_from_spec(spec)
spec.loader.exec_module(suite)


def client(trace_id="1" * 32, span_id="2" * 16, response_ok=True):
    return {"trace_id": trace_id, "span_id": span_id, "response_ok": response_ok,
            "send_start": 1.0, "hook_end": 1.001, "duration_ms": 1.0}


def span(trace_id="1" * 32, span_id="3" * 16, parent_span_id="2" * 16):
    return {"trace_id": trace_id, "span_id": span_id, "parent_span_id": parent_span_id,
            "name": "execute_tool Bash", "attributes": {"workspace.path": "C:/fixture"},
            "arrival_time": 1.002}


class ArchitectureSuiteTests(unittest.TestCase):
    def test_extract_span_attributes_rejects_empty_and_malformed_payloads(self):
        for payload in (b"", bytes([0xFF, 0xFF]), b"\x0a\x00"):
            with self.subTest(payload=payload), self.assertRaises(ValueError):
                suite.extract_spans_with_attrs(payload)

    def test_evidence_oracle_rejects_missing_duplicate_unexpected_wrong_parent_and_response_failure(self):
        offered = [client("a" * 32, "1" * 16), client("b" * 32, "2" * 16, False)]
        received = [span("a" * 32, "3" * 16, "f" * 16), span("a" * 32, "4" * 16, "1" * 16),
                    span("c" * 32, "5" * 16, "2" * 16)]
        result = suite.analyze_evidence(offered, received)
        self.assertFalse(result["valid"])
        self.assertEqual(result["missing_ids"], ["b" * 32])
        self.assertEqual(result["duplicate_ids"], ["a" * 32])
        self.assertEqual(result["unexpected_ids"], ["c" * 32])
        self.assertEqual(result["wrong_parent_ids"], ["a" * 32])
        self.assertEqual(result["response_failure_ids"], ["b" * 32])

    def test_evidence_oracle_accepts_one_valid_response_and_child(self):
        self.assertTrue(suite.analyze_evidence([client()], [span()])["valid"])

    def test_empty_stdout_is_not_a_valid_hook_response_and_payload_is_exact(self):
        proc = mock.Mock(returncode=0)
        proc.communicate.return_value = (b"", b"")
        with mock.patch.object(suite.subprocess, "Popen", return_value=proc) as popen:
            result = suite.execute_hook("hook", "pipe", 7, "1" * 32, "2" * 16, ".")
        self.assertFalse(result["response_ok"])
        self.assertEqual(popen.call_args.args[0], ["hook", "--client", "codex", "PostToolUse"])
        payload = json.loads(proc.communicate.call_args.kwargs["input"])
        self.assertEqual(set(payload), {"conversationId", "toolCall", "stepIdx", "workspace_path"})
        self.assertEqual(payload["stepIdx"], 7)

    def test_missing_workspace_path_cannot_match(self):
        with tempfile.TemporaryDirectory() as workspace:
            self.assertFalse(suite.workspace_matches(None, workspace))
            self.assertFalse(suite.workspace_matches("missing", workspace))
            self.assertTrue(suite.workspace_matches(workspace, workspace))

    def test_cleanup_or_diagnostics_failure_demotes_a_pass(self):
        cleanup = suite.CleanupState()
        cleanup.update({"fixtures_purged": False, "collector_stopped": True, "collector_errors": [],
                        "daemon_shutdown": {"measurement_status": "measured", "diagnostics": {
                            "ingress": {"admitted": 1}, "pipeline": {"transformed": 1, "accepted": 1}}}})
        result = suite.make_scenario_result("concurrent_delivery", "passed", [], [client()], [span()], cleanup)
        fake_session = SimpleNamespace(cleanup_info=cleanup, col=SimpleNamespace(received_spans=[span()]))
        suite._finalize_registered_results(fake_session)
        self.assertEqual(result["verdict"], "failed")
        self.assertEqual(result["assertions"][-1]["id"], "final_evidence_and_cleanup")

    def test_capture_is_bounded(self):
        capture = suite._BoundedCapture(4)
        capture.add("stderr", b"123456")
        self.assertEqual(capture.value("stderr"), b"1234")
        self.assertTrue(capture.exceeded.is_set())

    def test_generated_report_shape_validates_against_schema(self):
        try:
            import jsonschema
        except ImportError:
            self.skipTest("jsonschema is not available")
        sha = "a" * 64
        scenario = suite.make_scenario_result("concurrent_delivery", "failed", [], [], [], {"exception": "fixture"})
        scenario["round"] = 0
        report = {"schema": "agent-otel-new-architecture/v1", "timestamp": 1, "seed": 42, "repeats": 1,
                  "environment": {}, "candidate_hashes": {"daemon": {"path": "daemon", "sha256": sha},
                  "hook": {"path": "hook", "sha256": sha}}, "source_state": {"before": {}, "after": {}},
                  "installed_bridge": {"before": {"active_json_sha256": sha, "bin_hashes": {}},
                                       "after": {"active_json_sha256": sha, "bin_hashes": {}}},
                  "integrity_gates": {"source_intact": True, "candidate_binaries_intact": True,
                                      "installed_bridge_intact": True}, "campaign_duration_seconds": 0.1,
                  "overall_verdict": "failed", "scenarios": [scenario]}
        schema = json.loads((MODULE.parent / "architecture-report-v1.schema.json").read_text(encoding="utf-8"))
        jsonschema.Draft202012Validator(schema).validate(report)


if __name__ == "__main__":
    unittest.main()
