#!/usr/bin/env python3
"""Offline exact-trace validation for a saved long-trace report and SigNoz MCP response."""
import argparse
import json
import sys
from pathlib import Path
from typing import Any, Dict, List, Tuple


def _unwrap(value: Any) -> Tuple[Dict[str, Any], Dict[str, Any]]:
    """Return the direct MCP response and optional outer normalized wrapper."""
    if not isinstance(value, dict):
        raise ValueError("backend response must be a JSON object")
    wrapper = value
    content = value.get("content")
    if isinstance(content, list):
        texts = [item.get("text") for item in content
                 if isinstance(item, dict) and isinstance(item.get("text"), str)]
        if len(texts) != 1:
            raise ValueError("MCP content must contain exactly one JSON text response")
        try:
            value = json.loads(texts[0])
        except json.JSONDecodeError as exc:
            raise ValueError("MCP content text is not JSON") from exc
        if not isinstance(value, dict):
            raise ValueError("MCP content JSON must be an object")
        wrapper = {}
    if isinstance(value.get("raw"), dict):
        return value["raw"], value
    return value, wrapper if wrapper is not value else {}


def _extract_backend(value: Any) -> Dict[str, Any]:
    response, wrapper = _unwrap(value)
    status = response.get("status")
    data = response.get("data") if isinstance(response.get("data"), dict) else {}
    web_url = data.get("webUrl", response.get("webUrl"))
    query_data = data.get("data") if isinstance(data.get("data"), dict) else data
    results = query_data.get("results") if isinstance(query_data, dict) else None
    rows: List[Dict[str, Any]] = []
    cursors: List[Any] = []
    if isinstance(results, list):
        for result in results:
            if not isinstance(result, dict):
                raise ValueError("backend result must be an object")
            cursors.append(result.get("nextCursor"))
            result_rows = result.get("rows")
            if not isinstance(result_rows, list):
                raise ValueError("backend result rows must be an array")
            for row in result_rows:
                item = row.get("data") if isinstance(row, dict) and isinstance(row.get("data"), dict) else row
                if not isinstance(item, dict):
                    raise ValueError("backend row data must be an object")
                rows.append(item)
    else:
        raise ValueError("backend response is not a SigNoz MCP result envelope")
    return {"status": status, "webUrl": web_url, "rows": rows, "cursors": cursors}


def validate(report: Dict[str, Any], backend_value: Any) -> Dict[str, Any]:
    backend = _extract_backend(backend_value)
    trace_id = report.get("trace_id")
    root_id = report.get("root_span_id")
    expected = report.get("expected")
    requested = report.get("requested_duration_sec")
    local_duration = report.get("duration_sec")
    if not isinstance(expected, list) or not isinstance(requested, (int, float)) or not isinstance(local_duration, (int, float)):
        raise ValueError("local report lacks expected chain or duration fields")
    expected_parent = {root_id: ""}
    for item in expected:
        if not isinstance(item, dict) or not item.get("span_id"):
            raise ValueError("local expected chain contains an incomplete item")
        expected_parent[item["span_id"]] = item.get("parent_span_id")
    rows = backend["rows"]
    observed_ids = [row.get("span_id") for row in rows]
    duplicate_ids = sorted({span_id for span_id in observed_ids if observed_ids.count(span_id) > 1})
    expected_ids = set(expected_parent)
    observed_set = set(observed_ids)
    missing_ids = sorted(expected_ids - observed_set)
    unexpected_ids = sorted(str(value) for value in observed_set - expected_ids)
    foreign_trace_ids = sorted({str(row.get("trace_id")) for row in rows if row.get("trace_id") != trace_id})
    wrong_parent_ids = sorted(str(row.get("span_id")) for row in rows
                              if row.get("span_id") in expected_parent
                              and (row.get("parent_span_id") or "") != (expected_parent[row["span_id"]] or ""))
    root_rows = [row for row in rows if row.get("span_id") == root_id]
    root_duration_ns = root_rows[0].get("duration_nano") if len(root_rows) == 1 else None
    duration_numeric = isinstance(root_duration_ns, int) and not isinstance(root_duration_ns, bool)
    threshold_ok = duration_numeric and root_duration_ns >= int(requested * 1_000_000_000)
    local_duration_ns = round(local_duration * 1_000_000_000)
    duration_agrees = duration_numeric and abs(root_duration_ns - local_duration_ns) <= 1_000
    gates = {
        "status_success": backend["status"] == "success",
        "pagination_complete": all(cursor in (None, "") for cursor in backend["cursors"]),
        "exact_span_ids": not missing_ids and not unexpected_ids and not duplicate_ids and len(rows) == len(expected_ids),
        "single_expected_trace": not foreign_trace_ids,
        "parent_map_exact": not wrong_parent_ids,
        "single_root": len(root_rows) == 1,
        "root_duration_threshold": bool(threshold_ok),
        "root_duration_matches_local": bool(duration_agrees),
    }
    return {
        "schema": "agent-otel-backend-trace-validation/v1", "trace_id": trace_id,
        "verdict": "passed" if all(gates.values()) else "failed", "gates": gates,
        "expected_span_count": len(expected_ids), "observed_span_count": len(rows),
        "missing_ids": missing_ids, "duplicate_ids": duplicate_ids, "unexpected_ids": unexpected_ids,
        "foreign_trace_ids": foreign_trace_ids, "wrong_parent_ids": wrong_parent_ids,
        "requested_duration_ns": int(requested * 1_000_000_000),
        "local_duration_ns": local_duration_ns, "backend_root_duration_ns": root_duration_ns,
        "webUrl": backend["webUrl"],
        "evidence_source": "saved_signoz_mcp_response",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--backend", type=Path, required=True)
    args = parser.parse_args()
    try:
        report = json.loads(args.report.read_text(encoding="utf-8"))
        backend = json.loads(args.backend.read_text(encoding="utf-8"))
        result = validate(report, backend)
    except (OSError, ValueError, json.JSONDecodeError) as exc:
        result = {"schema": "agent-otel-backend-trace-validation/v1", "verdict": "failed",
                  "gates": {"input_valid": False}, "reason": f"{type(exc).__name__}: {exc}",
                  "webUrl": None, "evidence_source": "saved_signoz_mcp_response"}
    print(json.dumps(result, indent=2))
    return 0 if result["verdict"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
