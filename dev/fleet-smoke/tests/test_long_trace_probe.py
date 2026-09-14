import importlib.util
import json
import sys
import unittest
from pathlib import Path
from unittest import mock


MODULE = Path(__file__).parents[1] / "long_trace_probe.py"
spec = importlib.util.spec_from_file_location("long_trace_probe", MODULE)
probe = importlib.util.module_from_spec(spec)
sys.modules["long_trace_probe"] = probe
spec.loader.exec_module(probe)


def varint(value):
    encoded = bytearray()
    while True:
        byte = value & 0x7F
        value >>= 7
        encoded.append(byte | (0x80 if value else 0))
        if not value:
            return bytes(encoded)


def field(number, wire_type, value):
    return varint((number << 3) | wire_type) + value


def length_field(number, value):
    return field(number, 2, varint(len(value)) + value)


def native_span(trace_id, span_id, parent_span_id):
    return {
        "trace_id": trace_id,
        "span_id": span_id,
        "parent_span_id": parent_span_id,
        "name": "execute_tool Bash",
        "kind": 1,
        "start_time_unix_nano": 1,
        "end_time_unix_nano": 1,
        "attributes": {},
        "arrival_monotonic": 1.0,
    }


class LongTraceProbeTests(unittest.TestCase):
    def test_malformed_protobuf_request_and_response_are_rejected(self):
        with self.assertRaises(ValueError):
            probe.decode_export_trace_request(b"\x0a\x05\x12")
        outcome = probe.otlp_response_outcome(
            200, b"\x0a\x02\x08", "application/x-protobuf"
        )
        self.assertFalse(outcome["ok"])
        self.assertIn("Malformed OTLP response", outcome["error_message"])

    def test_partial_success_is_a_transport_failure_without_retry_inflation(self):
        partial = field(1, 0, varint(3)) + length_field(2, b"three rejected")
        response = length_field(1, partial)
        outcome = probe.otlp_response_outcome(
            200, response, "application/x-protobuf"
        )
        self.assertFalse(outcome["ok"])
        self.assertEqual(outcome["rejected_spans"], 3)
        self.assertEqual(outcome["error_message"], "three rejected")

    def test_shared_trace_oracle_accepts_thirty_unique_chained_spans(self):
        trace_id = "a" * 32
        root_id = "b" * 16
        spans = []
        parent = root_id
        for index in range(30):
            span_id = f"{index + 1:016x}"
            spans.append(native_span(trace_id, span_id, parent))
            parent = span_id

        evidence = probe.analyze_long_trace(trace_id, root_id, 30, spans)
        self.assertTrue(evidence["valid"])
        self.assertEqual(evidence["logical_count"], 30)
        self.assertEqual([item["span_id"] for item in evidence["ordered"]],
                         [f"{index + 1:016x}" for index in range(30)])

    def test_retransmitted_identical_span_does_not_inflate_logical_count(self):
        trace_id = "a" * 32
        root_id = "b" * 16
        first = native_span(trace_id, "1" * 16, root_id)
        second = native_span(trace_id, "2" * 16, "1" * 16)
        evidence = probe.analyze_long_trace(
            trace_id, root_id, 2, [first, dict(first), second]
        )
        self.assertTrue(evidence["valid"])
        self.assertEqual(evidence["captured_count"], 3)
        self.assertEqual(evidence["logical_count"], 2)
        self.assertEqual(evidence["duplicate_capture_count"], 1)

    def test_wrong_parent_and_foreign_trace_fail_the_oracle(self):
        trace_id = "a" * 32
        root_id = "b" * 16
        spans = [
            native_span(trace_id, "1" * 16, root_id),
            native_span(trace_id, "2" * 16, "f" * 16),
            native_span("c" * 32, "3" * 16, root_id),
        ]
        evidence = probe.analyze_long_trace(trace_id, root_id, 2, spans)
        self.assertFalse(evidence["valid"])
        self.assertEqual(evidence["orphan_ids"], ["2" * 16])
        self.assertEqual(evidence["unexpected_trace_ids"], ["c" * 32])

    def test_pacing_covers_sixty_seconds_without_scheduling_at_root_end(self):
        offsets = probe.pacing_offsets(60.0, 30)
        self.assertEqual(len(offsets), 30)
        self.assertEqual(offsets[:3], [0.0, 2.0, 4.0])
        self.assertEqual(offsets[-1], 58.0)

    def test_root_end_waits_for_small_wall_clock_lag_without_clamping_timestamp(self):
        state = {"monotonic": 0.0}
        root_start_ns = 100_000_000_000

        def monotonic():
            return state["monotonic"]

        def wall_ns():
            lag_ns = 2_000_000 if state["monotonic"] > 0 else 0
            return root_start_ns + int(state["monotonic"] * 1_000_000_000) - lag_ns

        def sleep(seconds):
            state["monotonic"] += seconds

        result = probe.await_root_end(
            root_start_ns, 0.0, 60.0, monotonic, wall_ns, sleep
        )
        self.assertTrue(result["complete"])
        self.assertGreaterEqual(result["wall_elapsed_sec"], 60.0)
        self.assertGreater(result["monotonic_elapsed_sec"], 60.0)
        self.assertEqual(result["end_time_unix_nano"], wall_ns())

    def test_root_end_fails_bounded_when_wall_clock_cannot_catch_up(self):
        state = {"monotonic": 0.0}
        root_start_ns = 100_000_000_000

        def monotonic():
            return state["monotonic"]

        def wall_ns():
            lag_ns = 5_000_000_000 if state["monotonic"] > 0 else 0
            return root_start_ns + int(state["monotonic"] * 1_000_000_000) - lag_ns

        def sleep(seconds):
            state["monotonic"] += seconds

        result = probe.await_root_end(
            root_start_ns, 0.0, 60.0, monotonic, wall_ns, sleep
        )
        self.assertFalse(result["complete"])
        self.assertLess(result["wall_elapsed_sec"], 60.0)
        self.assertLessEqual(result["monotonic_elapsed_sec"], 61.001)

    def test_hook_uses_current_actual_span_as_traceparent_parent(self):
        process = mock.Mock(returncode=0)
        process.stdin = mock.Mock()
        process.wait.return_value = 0
        capture = mock.Mock()
        capture.value.side_effect = lambda stream: b"{}" if stream == "stdout" else b""
        capture.exceeded.is_set.return_value = False
        trace_id = "a" * 32
        parent_id = "b" * 16
        with (mock.patch.object(probe.subprocess, "Popen", return_value=process) as popen,
              mock.patch.object(probe, "start_bounded_capture", return_value=(capture, []))):
            result = probe.execute_hook(
                "hook", "pipe", 7, trace_id, parent_id, ".", deadline=float("inf")
            )
        self.assertTrue(result["response_ok"])
        self.assertEqual(
            popen.call_args.kwargs["env"]["TRACEPARENT"],
            f"00-{trace_id}-{parent_id}-01",
        )
        payload = json.loads(process.stdin.write.call_args.args[0])
        self.assertEqual(payload["conversationId"], trace_id)
        self.assertEqual(payload["stepIdx"], 7)

    def test_daemon_reconciliation_uses_logical_event_counts(self):
        diagnostics = {
            "ingress": {
                "admitted": 30, "invalid": 0, "capacity": 0, "read_failed": 0,
                "read_deadline": 0, "reserved_bytes": 0,
            },
            "pipeline": {
                "transformed": 30, "accepted": 30, "invalid": 0, "span_size": 0,
                "export_capacity": 0, "rejected": 0, "unknown": 0,
                "shutdown_dropped": 0, "queued_bytes": 0, "queued_items": 0,
            },
        }
        self.assertTrue(probe.daemon_reconciliation_ok(diagnostics, 30))
        diagnostics["pipeline"]["rejected"] = 1
        self.assertFalse(probe.daemon_reconciliation_ok(diagnostics, 30))

    def test_only_private_local_otlp_endpoint_is_accepted(self):
        self.assertEqual(
            probe.validate_upstream_endpoint("http://localhost:4318"),
            "http://127.0.0.1:4318",
        )
        for endpoint in (
            "https://127.0.0.1:4318",
            "http://127.0.0.1:4317",
            "http://collector.example:4318",
            "http://127.0.0.1:4318/v1/traces",
        ):
            with self.subTest(endpoint=endpoint), self.assertRaises(ValueError):
                probe.validate_upstream_endpoint(endpoint)

    def test_helper_api_mismatch_after_spawn_still_cleans_all_owned_resources(self):
        process = mock.Mock(returncode=None)
        process.poll.side_effect = lambda: process.returncode
        capture = mock.Mock()
        server = mock.Mock()
        server.server_address = ("127.0.0.1", 43180)

        def stopped(*_args):
            process.returncode = 0
            return {"exit_code": 0}

        with (mock.patch.object(probe, "collect_environment", return_value={"os": "fixture"}),
              mock.patch.object(probe, "inspect_source_state", return_value={}),
              mock.patch.object(probe, "inspect_installed_bridge", return_value={}),
              mock.patch.object(probe, "make_git_workspace", return_value="owned-workspace"),
              mock.patch.object(probe, "cleanup_git_workspace", return_value=True) as cleanup,
              mock.patch.object(probe, "ForwardingCollectorServer", return_value=server),
              mock.patch.object(probe.subprocess, "Popen", return_value=process),
              mock.patch.object(probe, "start_bounded_capture", return_value=(capture, [])),
              mock.patch.object(probe, "pipe_ready", side_effect=TypeError("API mismatch")) as ready,
              mock.patch.object(probe, "stop_daemon_gracefully", side_effect=stopped) as stop):
            with self.assertRaisesRegex(TypeError, "API mismatch"):
                probe.run_probe("daemon", "hook", "http://127.0.0.1:4318", 60, 30)

        self.assertEqual(len(ready.call_args.args), 2)
        self.assertFalse(ready.call_args.kwargs)
        stop.assert_called_once_with(process, mock.ANY, capture, [])
        server.shutdown.assert_called()
        server.server_close.assert_called()
        cleanup.assert_called_with("owned-workspace")

    def test_report_schema_accepts_failed_preflight_and_rejects_bad_ids(self):
        try:
            import jsonschema
        except ImportError:
            self.skipTest("jsonschema is unavailable")
        sha = "d" * 64
        report = {
            "schema": "agent-otel-long-trace/v1",
            "run_id": "a" * 32,
            "trace_id": "b" * 32,
            "root_span_id": "c" * 16,
            "service_name": "agent-otel-long-trace-aaaaaaaa",
            "start_ms": 1,
            "end_ms": 1,
            "duration_sec": 0.0,
            "requested_duration_sec": 60,
            "pacing_interval_sec": 2.0,
            "expected_events": 30,
            "received_events": 0,
            "expected": [],
            "received": [],
            "hook_executions": [],
            "transport": {"error": "pipe unavailable"},
            "backend_visibility": {
                "status": "not_checked",
                "reason": "root MCP read required externally",
            },
            "diagnostics": {"daemon": {}},
            "cleanup": {
                "daemon_stopped": True,
                "workspace_removed": True,
                "collector_stopped": True,
            },
            "integrity": {
                "before": {
                    "candidate_daemon_sha256": sha,
                    "candidate_hook_sha256": sha,
                    "installed_bridge": {},
                    "source_state": {},
                    "environment": {},
                },
                "after": {
                    "candidate_daemon_sha256": sha,
                    "candidate_hook_sha256": sha,
                    "installed_bridge": {},
                    "source_state": {},
                    "environment": {},
                },
            },
            "hard_gates": {"pipe_ready": False},
            "verdict": "failed",
        }
        schema = json.loads(
            (MODULE.parent / "long-trace-report-v1.schema.json").read_text(
                encoding="utf-8"
            )
        )
        validator = jsonschema.Draft202012Validator(
            schema, format_checker=jsonschema.FormatChecker()
        )
        validator.validate(report)
        report["trace_id"] = "not-a-trace"
        with self.assertRaises(jsonschema.ValidationError):
            validator.validate(report)


if __name__ == "__main__":
    unittest.main()
