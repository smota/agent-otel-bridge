"""Tests for ClickHouse and mock telemetry storage readers."""
import unittest

from telemetry_backends.base import NormalizedSpan, QueryResult
from telemetry_backends.clickhouse import ClickHouseReader, MockTelemetryReader
from telemetry_backends.schema_maps import build_trace_query, SCHEMA_V3, SCHEMA_V2


class TestTelemetryBackends(unittest.TestCase):
    def test_schema_maps_sql_construction(self):
        query_v3, schema = build_trace_query("v3")
        self.assertIn("signoz_traces.distributed_signoz_index_v3", query_v3)
        self.assertIn("LIMIT 10001", query_v3)
        self.assertIn("{trace_id:String}", query_v3)
        self.assertNotIn("SELECT *", query_v3)

        query_tenant, _ = build_trace_query("v3", tenant_id="t-123")
        self.assertIn("tenant = {tenant:String}", query_tenant)

    def test_mock_reader_query(self):
        span = NormalizedSpan(
            trace_id="00000000000000000000000000000001",
            span_id="0000000000000002",
            parent_span_id="0000000000000001",
            start_unix_ns=1000,
            duration_ns=500,
            name="execute_tool",
            kind="SPAN_KIND_INTERNAL",
            status="STATUS_CODE_OK",
            service_name="agent-bridge",
        )
        mock = MockTelemetryReader(fixture_spans=[span])
        self.assertTrue(mock.ping())

        res = mock.query_trace("00000000000000000000000000000001")
        self.assertEqual(res.status, "visible_complete")
        self.assertEqual(len(res.rows), 1)
        self.assertEqual(res.rows[0].span_id, "0000000000000002")
        self.assertTrue(len(res.digest) > 0)

        not_found = mock.query_trace("ffffffffffffffffffffffffffffffff")
        self.assertEqual(not_found.status, "not_found")

    def test_clickhouse_reader_offline_resilience(self):
        # Point to closed port
        reader = ClickHouseReader(endpoint="http://127.0.0.1:59999", timeout_sec=0.5)
        self.assertFalse(reader.ping())

        res = reader.query_trace("00000000000000000000000000000001")
        self.assertEqual(res.status, "query_failed")
        self.assertIsNotNone(res.error_message)


if __name__ == "__main__":
    unittest.main()
