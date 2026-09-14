import importlib.util
import os
import sys
import unittest

MODULE_PATH = os.path.join(os.path.dirname(__file__), "..", "perf_delivery.py")
spec = importlib.util.spec_from_file_location("perf_delivery", os.path.abspath(MODULE_PATH))
perf_delivery = importlib.util.module_from_spec(spec)
sys.modules["perf_delivery"] = perf_delivery
spec.loader.exec_module(perf_delivery)

def encode_varint(val: int) -> bytes:
    res = bytearray()
    while True:
        b = val & 0x7F
        val >>= 7
        if val:
            res.append(b | 0x80)
        else:
            res.append(b)
            break
    return bytes(res)

def make_field(fn: int, wt: int, val: bytes) -> bytes:
    return encode_varint((fn << 3) | wt) + val

def make_span(trace_id_bytes: bytes, span_id_bytes: bytes) -> bytes:
    f1 = make_field(1, 2, encode_varint(len(trace_id_bytes)) + trace_id_bytes)
    f2 = make_field(2, 2, encode_varint(len(span_id_bytes)) + span_id_bytes)
    return f1 + f2

def make_export_request(spans_bytes: list) -> bytes:
    scope_spans_body = bytearray()
    for s in spans_bytes:
        scope_spans_body.extend(make_field(2, 2, encode_varint(len(s)) + s))
    r_spans_body = make_field(2, 2, encode_varint(len(scope_spans_body)) + bytes(scope_spans_body))
    return make_field(1, 2, encode_varint(len(r_spans_body)) + r_spans_body)

class TestPerfDelivery(unittest.TestCase):
    def test_protobuf_trace_extraction(self):
        t1 = bytes.fromhex("11223344556677889900aabbccddeeff")
        s1 = bytes.fromhex("aabbccddeeff0011")
        payload = make_export_request([make_span(t1, s1)])
        parsed = perf_delivery.extract_trace_spans(payload)
        self.assertEqual(len(parsed), 1)
        self.assertEqual(parsed[0][0], t1.hex())
        self.assertEqual(parsed[0][1], s1.hex())

    def test_malformed_protobuf_rejection(self):
        with self.assertRaises(ValueError):
            perf_delivery.extract_trace_spans(b"\x0a\x0f\x12\x0d\x12\xff")

    def test_evaluate_delivery_perfect_pass(self):
        exp = ["a1", "b2", "c3"]
        rcv = [("a1", "s1"), ("b2", "s2"), ("c3", "s3")]
        res = perf_delivery.evaluate_delivery(exp, rcv)
        self.assertEqual(res["verdict"], "passed")
        self.assertEqual(len(res["missing_ids"]), 0)
        self.assertEqual(len(res["duplicate_ids"]), 0)

    def test_evaluate_delivery_detects_missing(self):
        exp = ["a1", "b2", "c3"]
        rcv = [("a1", "s1"), ("c3", "s3")]
        res = perf_delivery.evaluate_delivery(exp, rcv)
        self.assertEqual(res["verdict"], "failed")
        self.assertIn("b2", res["missing_ids"])

    def test_evaluate_delivery_detects_duplicates(self):
        exp = ["a1", "b2"]
        rcv = [("a1", "s1"), ("b2", "s2"), ("b2", "s2_dup")]
        res = perf_delivery.evaluate_delivery(exp, rcv)
        self.assertEqual(res["verdict"], "failed")
        self.assertIn("b2", res["duplicate_ids"])

    def test_evaluate_delivery_rejects_empty_expected(self):
        res = perf_delivery.evaluate_delivery([], [("a1", "s1")])
        self.assertEqual(res["verdict"], "failed")
        self.assertEqual(res["reason"], "empty_expected_ids")

    def test_daemon_diagnostics_are_selected_from_mixed_stderr(self):
        stderr = 'startup\n{"kind":"other"}\n{"kind":"bridge_diagnostics","pipeline":{"accepted":21,"shutdown_dropped":3}}\n'
        result = perf_delivery.parse_daemon_diagnostics(stderr)
        self.assertEqual(result["pipeline"]["accepted"], 21)
        self.assertEqual(result["pipeline"]["shutdown_dropped"], 3)

    def test_observer_classifies_completed_send(self):
        raw = perf_delivery.HOOK_OBSERVER_RECORD.pack(
            b"AOBT", 2, perf_delivery.HOOK_OBSERVER_RECORD.size, 1234, 3, 0, 0, 10, 20, 30
        )
        result = perf_delivery.classify_observer(raw, 1234, True, 0)
        self.assertEqual(result["status"], "send_completed")
        self.assertTrue(result["send_completed"])
        self.assertEqual(result["record_bytes"], 48)
        self.assertIsNone(result["transport_stage"])
        self.assertEqual(result["os_code"], 0)

    def test_missing_observer_record_is_unknown(self):
        result = perf_delivery.classify_observer(b"", 1234, True, 0)
        self.assertEqual(result["status"], "unknown")
        self.assertIsNone(result["send_completed"])
        self.assertFalse(result["watchdog_inferred"])
        self.assertIsNone(result["inference"])

    def test_observer_classifies_explicit_transport_error(self):
        raw = perf_delivery.HOOK_OBSERVER_RECORD.pack(
            b"AOBT", 2, perf_delivery.HOOK_OBSERVER_RECORD.size, 1234, 1, 1, 2, 10, 20, 30
        )
        result = perf_delivery.classify_observer(raw, 1234, True, 0)
        self.assertEqual(result["status"], "send_error")
        self.assertFalse(result["send_completed"])
        self.assertEqual(result["transport_stage"], "connect")
        self.assertEqual(result["os_code"], 2)

    def test_v1_observer_record_remains_decodable(self):
        raw = perf_delivery.RECORD_V1.pack(
            b"AOBT", 1, perf_delivery.RECORD_V1.size, 1234, 3, 10, 20, 30
        )
        result = perf_delivery.classify_observer(raw, 1234, True, 0)
        self.assertEqual(result["status"], "send_completed")
        self.assertEqual(result["version"], 1)
        self.assertIsNone(result["transport_stage"])
        self.assertIsNone(result["os_code"])

        incomplete = perf_delivery.RECORD_V1.pack(
            b"AOBT", 1, perf_delivery.RECORD_V1.size, 1234, 1, 10, 20, 30
        )
        result = perf_delivery.classify_observer(incomplete, 1234, True, 0)
        self.assertEqual(result["status"], "send_not_completed_legacy")
        self.assertFalse(result["send_completed"])
        self.assertIsNone(result["transport_stage"])

    def test_truncated_unknown_version_and_pid_mismatch_are_invalid(self):
        valid = perf_delivery.HOOK_OBSERVER_RECORD.pack(
            b"AOBT", 2, perf_delivery.HOOK_OBSERVER_RECORD.size, 1234, 3, 0, 0, 10, 20, 30
        )
        cases = [
            valid[:-1],
            bytearray(valid),
            perf_delivery.HOOK_OBSERVER_RECORD.pack(
                b"AOBT", 2, perf_delivery.HOOK_OBSERVER_RECORD.size, 9999, 3, 0, 0, 10, 20, 30
            ),
        ]
        cases[1][4:6] = (99).to_bytes(2, "little")
        for raw in cases:
            result = perf_delivery.classify_observer(bytes(raw), 1234, True, 0)
            self.assertEqual(result["status"], "invalid_record")
            self.assertIsNone(result["send_completed"])

    def test_missing_trace_is_correlated_with_its_observer(self):
        clients = [{"trace_id": "lost", "event_idx": 2, "response_ok": True,
                    "observer": {"status": "unknown", "send_completed": None,
                                 "transport_stage": None, "os_code": None,
                                 "watchdog_inferred": False}}]
        result = perf_delivery.correlate_missing_clients(["lost"], clients)
        self.assertEqual(result[0]["event_idx"], 2)
        self.assertFalse(result[0]["watchdog_inferred"])
        self.assertIsNone(result[0]["send_completed"])

if __name__ == "__main__":
    unittest.main()
