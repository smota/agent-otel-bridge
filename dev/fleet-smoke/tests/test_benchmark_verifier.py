"""Tests for benchmark reconciliation and assertions B01-B13."""
import unittest

import benchmark_verifier as verifier
from telemetry_backends.base import QueryResult, NormalizedSpan


class TestBenchmarkVerifier(unittest.TestCase):
    def test_b01_identities(self):
        plan1 = {"seed": 502, "corpus_hash": "abc", "run_id": "r1", "campaign_id": "c1"}
        plan2 = {"seed": 502, "corpus_hash": "abc", "run_id": "r2", "campaign_id": "c2"}
        res = verifier.verify_b01_identities(plan1, plan2)
        self.assertEqual(res.verdict, "passed")

        plan3 = {"seed": 502, "corpus_hash": "abc", "run_id": "r1", "campaign_id": "c1"}
        res_fail = verifier.verify_b01_identities(plan1, plan3)
        self.assertEqual(res_fail.verdict, "failed")

    def test_b02_budgets(self):
        self.assertEqual(verifier.verify_b02_budgets("quick", 0, 0).verdict, "passed")
        self.assertEqual(verifier.verify_b02_budgets("quick", 1, 0).verdict, "failed")
        self.assertEqual(verifier.verify_b02_budgets("fleet", 3, 3).verdict, "passed")
        self.assertEqual(verifier.verify_b02_budgets("fleet", 4, 3).verdict, "failed")

    def test_b03_handoff(self):
        valid = {
            "schema": "aob-handoff/v1",
            "task_id": "F1",
            "inputs": [{"path": "input.json", "sha256": "abc"}],
        }
        self.assertEqual(verifier.verify_b03_handoff(valid, ".").verdict, "passed")

        traversal = {
            "schema": "aob-handoff/v1",
            "task_id": "F1",
            "inputs": [{"path": "../secret.json", "sha256": "abc"}],
        }
        self.assertEqual(verifier.verify_b03_handoff(traversal, ".").verdict, "failed")

    def test_b07_sql_safety(self):
        safe = "SELECT trace_id FROM t WHERE trace_id = {trace_id:String} LIMIT 10001 FORMAT JSONEachRow"
        self.assertEqual(verifier.verify_b07_sql_safety(safe, {}).verdict, "passed")

        unsafe = "SELECT * FROM t WHERE trace_id = 1"
        self.assertEqual(verifier.verify_b07_sql_safety(unsafe, {}).verdict, "failed")

        drop = "DROP TABLE t LIMIT 10001"
        self.assertEqual(verifier.verify_b07_sql_safety(drop, {}).verdict, "failed")

    def test_b08_storage_response(self):
        complete = QueryResult(status="visible_complete", rows=[NormalizedSpan("a", "b", "c", 1, 2, "n", "k", "s", "svc")])
        self.assertEqual(verifier.verify_b08_storage_response(complete).verdict, "passed")

        sentinel = QueryResult(status="result_limit_exceeded", sentinel_hit=True)
        self.assertEqual(verifier.verify_b08_storage_response(sentinel).verdict, "inconclusive")

        unsupported = QueryResult(status="unsupported_schema")
        self.assertEqual(verifier.verify_b08_storage_response(unsupported).verdict, "failed")

    def test_b12_performance_contracts(self):
        passed = verifier.verify_b12_performance_contracts(hook_p99_us=150.0, binary_size_bytes=150000, parser_ops_sec=70000.0)
        self.assertEqual(passed.verdict, "passed")

        failed_size = verifier.verify_b12_performance_contracts(hook_p99_us=150.0, binary_size_bytes=350000, parser_ops_sec=70000.0)
        self.assertEqual(failed_size.verdict, "failed")


if __name__ == "__main__":
    unittest.main()
