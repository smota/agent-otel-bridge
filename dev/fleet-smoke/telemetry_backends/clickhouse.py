"""ClickHouse HTTP telemetry reader for direct SigNoz trace storage verification."""
import json
import os
import time
import urllib.parse
import urllib.request
import urllib.error
from typing import Dict, Any, List, Optional

from .base import TelemetryReader, QueryResult, NormalizedSpan
from .schema_maps import build_trace_query


class ClickHouseReader(TelemetryReader):
    """Direct, read-only HTTP reader for ClickHouse trace storage."""

    def __init__(
        self,
        endpoint: Optional[str] = "http://localhost:8123",
        database: str = "signoz_traces",
        schema_version: str = "v3",
        credentials_env_var: Optional[str] = None,
        timeout_sec: float = 5.0,
    ):
        self.endpoint = endpoint.rstrip("/") if endpoint else None
        self.database = database
        self.schema_version = schema_version
        self.credentials_env_var = credentials_env_var
        self.timeout_sec = timeout_sec

    def _get_auth_headers(self) -> Dict[str, str]:
        headers = {
            "Accept": "application/x-ndjson, application/json",
            "User-Agent": "agent-otel-bridge-benchmark/1.0",
        }
        if self.credentials_env_var and self.credentials_env_var in os.environ:
            secret = os.environ[self.credentials_env_var].strip()
            if secret:
                if ":" in secret:
                    import base64
                    encoded = base64.b64encode(secret.encode("utf-8")).decode("ascii")
                    headers["Authorization"] = f"Basic {encoded}"
                else:
                    headers["Authorization"] = f"Bearer {secret}"
        return headers

    def ping(self) -> bool:
        if not self.endpoint:
            return False
        ping_url = f"{self.endpoint}/ping"
        try:
            req = urllib.request.Request(ping_url, headers=self._get_auth_headers())
            with urllib.request.urlopen(req, timeout=3.0) as resp:
                return resp.status == 200 and resp.read().strip() == b"Ok."
        except Exception:
            return False

    def query_trace(
        self,
        trace_id: str,
        start_unix_ns: int = 0,
        end_unix_ns: int = 0,
        tenant_id: Optional[str] = None,
    ) -> QueryResult:
        if not self.endpoint:
            return QueryResult(
                status="not_measured",
                error_message="ClickHouse endpoint not configured or offline",
            )

        sql, schema = build_trace_query(self.schema_version, tenant_id=tenant_id)

        # ClickHouse HTTP parameters: query params define parameter values
        params = {
            "query": sql,
            "database": self.database,
            "param_trace_id": trace_id,
            "max_execution_time": "5",
            "max_result_rows": "10001",
            "max_result_bytes": "16777216",
        }
        if tenant_id:
            params["param_tenant"] = tenant_id

        url = f"{self.endpoint}/?{urllib.parse.urlencode(params)}"
        start_t = time.monotonic()

        try:
            req = urllib.request.Request(url, headers=self._get_auth_headers(), method="POST")
            with urllib.request.urlopen(req, timeout=self.timeout_sec) as response:
                body = response.read()
                exec_time = time.monotonic() - start_t
                query_id = response.headers.get("X-ClickHouse-Query-Id")

                lines = [line.strip() for line in body.split(b"\n") if line.strip()]
                bytes_read = len(body)

                if len(lines) > 10000:
                    return QueryResult(
                        status="result_limit_exceeded",
                        query_id=query_id,
                        execution_time_sec=exec_time,
                        bytes_read=bytes_read,
                        mapping_version=schema["version"],
                        sentinel_hit=True,
                        error_message="Result limit exceeded (10,001 rows returned)",
                    )

                if not lines:
                    return QueryResult(
                        status="not_found",
                        query_id=query_id,
                        execution_time_sec=exec_time,
                        bytes_read=bytes_read,
                        mapping_version=schema["version"],
                    )

                normalized_rows: List[NormalizedSpan] = []
                for line in lines:
                    try:
                        record = json.loads(line.decode("utf-8"))
                        span = NormalizedSpan(
                            trace_id=str(record.get("trace_id", "")).lower(),
                            span_id=str(record.get("span_id", "")).lower(),
                            parent_span_id=str(record.get("parent_span_id", "")).lower(),
                            start_unix_ns=int(record.get("start_time", 0)),
                            duration_ns=int(record.get("duration_nano", 0)),
                            name=str(record.get("name", "")),
                            kind=str(record.get("kind", "")),
                            status=str(record.get("status_code", "STATUS_CODE_UNSET")),
                            service_name=str(record.get("service_name", "")),
                        )
                        normalized_rows.append(span)
                    except (ValueError, KeyError, json.JSONDecodeError):
                        continue

                result = QueryResult(
                    status="visible_complete" if normalized_rows else "not_found",
                    rows=normalized_rows,
                    query_id=query_id,
                    execution_time_sec=exec_time,
                    bytes_read=bytes_read,
                    mapping_version=schema["version"],
                )
                result.compute_digest()
                return result

        except urllib.error.HTTPError as http_err:
            exec_time = time.monotonic() - start_t
            err_body = http_err.read().decode("utf-8", errors="replace")
            status = "unsupported_schema" if "UNKNOWN_IDENTIFIER" in err_body or "UNKNOWN_TABLE" in err_body else "query_failed"
            return QueryResult(
                status=status,
                execution_time_sec=exec_time,
                error_message=f"HTTP {http_err.code}: {err_body[:200]}",
            )
        except Exception as exc:
            exec_time = time.monotonic() - start_t
            return QueryResult(
                status="query_failed",
                execution_time_sec=exec_time,
                error_message=str(exc),
            )

    def query_trace_stable(
        self,
        trace_id: str,
        start_unix_ns: int = 0,
        end_unix_ns: int = 0,
        tenant_id: Optional[str] = None,
        max_wait_sec: float = 30.0,
    ) -> QueryResult:
        """Polls until two consecutive reads separated by 2s yield identical digests."""
        poll_offsets = [0.0, 2.0, 5.0, 10.0, 20.0]
        first_result: Optional[QueryResult] = None

        start_wait = time.monotonic()
        for delay in poll_offsets:
            if time.monotonic() - start_wait > max_wait_sec:
                break
            if delay > 0:
                time.sleep(delay)

            res = self.query_trace(trace_id, start_unix_ns, end_unix_ns, tenant_id)
            if res.status not in ("visible_complete", "visible_partial"):
                continue

            if first_result is None:
                first_result = res
                time.sleep(2.0)
                continue

            if res.digest == first_result.digest and len(res.rows) == len(first_result.rows):
                res.status = "visible_complete"
                return res
            else:
                first_result = res

        return first_result or QueryResult(status="not_found")


class MockTelemetryReader(TelemetryReader):
    """In-memory mock reader for fixture-driven and offline testing."""

    def __init__(self, fixture_spans: Optional[List[NormalizedSpan]] = None):
        self.spans = fixture_spans or []
        self.reachable = True

    def ping(self) -> bool:
        return self.reachable

    def query_trace(
        self,
        trace_id: str,
        start_unix_ns: int = 0,
        end_unix_ns: int = 0,
        tenant_id: Optional[str] = None,
    ) -> QueryResult:
        if not self.reachable:
            return QueryResult(status="query_failed", error_message="Mock storage unreachable")

        matched = [s for s in self.spans if s.trace_id == trace_id.lower()]
        if not matched:
            return QueryResult(status="not_found")

        res = QueryResult(
            status="visible_complete",
            rows=matched,
            execution_time_sec=0.005,
            bytes_read=len(matched) * 128,
            mapping_version="v3",
        )
        res.compute_digest()
        return res
