import copy
import importlib.util
import json
import unittest
from pathlib import Path

MODULE = Path(__file__).parents[1] / "backend_trace_validation.py"
spec = importlib.util.spec_from_file_location("backend_trace_validation", MODULE)
validator = importlib.util.module_from_spec(spec)
spec.loader.exec_module(validator)


def fixture():
    trace, root = "a" * 32, "b" * 16
    child1, child2 = "c" * 16, "d" * 16
    report = {"trace_id": trace, "root_span_id": root, "requested_duration_sec": 60,
              "duration_sec": 60.0000004,
              "expected": [{"span_id": child1, "parent_span_id": root},
                           {"span_id": child2, "parent_span_id": child1}]}
    rows = [{"trace_id": trace, "span_id": root, "parent_span_id": "", "duration_nano": 60_000_000_400},
            {"trace_id": trace, "span_id": child1, "parent_span_id": root, "duration_nano": 0},
            {"trace_id": trace, "span_id": child2, "parent_span_id": child1, "duration_nano": 0}]
    direct = {"status": "success", "data": {"webUrl": "https://example/trace/a",
              "data": {"results": [{"nextCursor": "", "rows": [{"data": row} for row in rows]}]}}}
    return report, direct


class BackendTraceValidationTests(unittest.TestCase):
    def test_valid_direct_and_content_wrapped_payloads(self):
        report, backend = fixture()
        direct = validator.validate(report, backend)
        wrapped = validator.validate(report, {"content": [{"type": "text", "text": json.dumps(backend)}]})
        self.assertEqual(direct["verdict"], "passed")
        self.assertEqual(wrapped["verdict"], "passed")
        self.assertEqual(direct["webUrl"], "https://example/trace/a")

    def test_wrong_parent_fails(self):
        report, backend = fixture()
        backend["data"]["data"]["results"][0]["rows"][2]["data"]["parent_span_id"] = "e" * 16
        result = validator.validate(report, backend)
        self.assertEqual(result["verdict"], "failed")
        self.assertFalse(result["gates"]["parent_map_exact"])

    def test_missing_root_and_partial_result_fail(self):
        report, backend = fixture()
        backend["data"]["data"]["results"][0]["rows"].pop(0)
        result = validator.validate(report, backend)
        self.assertEqual(result["verdict"], "failed")
        self.assertFalse(result["gates"]["single_root"])
        self.assertFalse(result["gates"]["exact_span_ids"])

    def test_short_root_fails_without_rounding_threshold(self):
        report, backend = fixture()
        report["duration_sec"] = 59.9999996
        backend["data"]["data"]["results"][0]["rows"][0]["data"]["duration_nano"] = 59_999_999_600
        result = validator.validate(report, backend)
        self.assertTrue(result["gates"]["root_duration_matches_local"])
        self.assertFalse(result["gates"]["root_duration_threshold"])
        self.assertEqual(result["verdict"], "failed")

    def test_pagination_fails_even_with_complete_first_page(self):
        report, backend = fixture()
        backend["data"]["data"]["results"][0]["nextCursor"] = "page-2"
        result = validator.validate(report, backend)
        self.assertFalse(result["gates"]["pagination_complete"])
        self.assertEqual(result["verdict"], "failed")

    def test_normalized_saved_wrapper_uses_raw_status_and_web_url(self):
        report, backend = fixture()
        rows = [item["data"] for item in backend["data"]["data"]["results"][0]["rows"]]
        result = validator.validate(report, {"method": "SigNoz MCP", "rows": rows, "raw": backend})
        self.assertEqual(result["verdict"], "passed")

    def test_http_style_rows_cannot_substitute_for_mcp_evidence(self):
        report, backend = fixture()
        rows = [item["data"] for item in backend["data"]["data"]["results"][0]["rows"]]
        with self.assertRaisesRegex(ValueError, "not a SigNoz MCP result envelope"):
            validator.validate(report, {"status": "success", "rows": rows})


if __name__ == "__main__":
    unittest.main()
