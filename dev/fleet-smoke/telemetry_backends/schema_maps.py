"""Schema definitions and SQL query builders for ClickHouse trace storage."""
from typing import Dict, Any, Tuple


SCHEMA_V3 = {
    "version": "v3",
    "table": "signoz_traces.distributed_signoz_index_v3",
    "trace_col": "traceID",
    "span_col": "spanID",
    "parent_col": "parentSpanID",
    "time_col": "timestamp",
    "duration_col": "durationNano",
    "name_col": "name",
    "kind_col": "kind",
    "status_col": "statusCode",
    "service_col": "serviceName",
}

SCHEMA_V2 = {
    "version": "v2",
    "table": "signoz_traces.distributed_signoz_index_v2",
    "trace_col": "traceID",
    "span_col": "spanID",
    "parent_col": "parentSpanID",
    "time_col": "timestamp",
    "duration_col": "durationNano",
    "name_col": "name",
    "kind_col": "kind",
    "status_col": "statusCode",
    "service_col": "serviceName",
}

SCHEMAS = {
    "v3": SCHEMA_V3,
    "v2": SCHEMA_V2,
}


def build_trace_query(schema_version: str = "v3", tenant_id: str = None) -> Tuple[str, Dict[str, str]]:
    """Build fixed, versioned parameterized query for ClickHouse HTTP interface."""
    schema = SCHEMAS.get(schema_version, SCHEMA_V3)
    table = schema["table"]

    tenant_clause = " AND tenant = {tenant:String}" if tenant_id else ""

    query = (
        f"SELECT {schema['trace_col']} AS trace_id, "
        f"{schema['span_col']} AS span_id, "
        f"{schema['parent_col']} AS parent_span_id, "
        f"{schema['time_col']} AS start_time, "
        f"{schema['duration_col']} AS duration_nano, "
        f"{schema['name_col']} AS name, "
        f"{schema['kind_col']} AS kind, "
        f"{schema['status_col']} AS status_code, "
        f"{schema['service_col']} AS service_name "
        f"FROM {table} "
        f"WHERE {schema['trace_col']} = {{trace_id:String}}{tenant_clause} "
        f"ORDER BY {schema['time_col']} ASC, {schema['span_col']} ASC "
        f"LIMIT 10001 FORMAT JSONEachRow"
    )
    return query, schema
