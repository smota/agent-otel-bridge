"""Abstract base class and models for direct telemetry storage readers."""
from abc import ABC, abstractmethod
from dataclasses import dataclass, field
import hashlib
import json
from typing import Any, Dict, List, Optional


@dataclass
class NormalizedSpan:
    trace_id: str
    span_id: str
    parent_span_id: str
    start_unix_ns: int
    duration_ns: int
    name: str
    kind: str
    status: str
    service_name: str
    service_version: Optional[str] = None
    scope_name: Optional[str] = None
    provenance: str = "storage_query"
    task_id: Optional[str] = None
    attributes: Dict[str, Any] = field(default_factory=dict)

    def to_dict(self) -> Dict[str, Any]:
        return {
            "trace_id": self.trace_id,
            "span_id": self.span_id,
            "parent_span_id": self.parent_span_id,
            "start_unix_ns": str(self.start_unix_ns),
            "duration_ns": str(self.duration_ns),
            "name": self.name,
            "kind": self.kind,
            "status": self.status,
            "service_name": self.service_name,
            "service_version": self.service_version,
            "scope_name": self.scope_name,
            "provenance": self.provenance,
            "task_id": self.task_id,
        }

    def digest(self) -> str:
        data = f"{self.trace_id}:{self.span_id}:{self.parent_span_id}:{self.name}:{self.status}"
        return hashlib.sha256(data.encode("utf-8")).hexdigest()


@dataclass
class QueryResult:
    status: str  # visible_complete, visible_partial, not_found, query_failed, unsupported_schema, result_limit_exceeded, not_measured
    rows: List[NormalizedSpan] = field(default_factory=list)
    query_id: Optional[str] = None
    execution_time_sec: float = 0.0
    bytes_read: int = 0
    mapping_version: str = "v3"
    error_message: Optional[str] = None
    sentinel_hit: bool = False
    digest: str = ""

    def compute_digest(self) -> str:
        sorted_digests = sorted(r.digest() for r in self.rows)
        joined = "|".join(sorted_digests)
        self.digest = hashlib.sha256(joined.encode("utf-8")).hexdigest()
        return self.digest


class TelemetryReader(ABC):
    """Abstract interface for querying telemetry traces directly from storage."""

    @abstractmethod
    def ping(self) -> bool:
        """Check if storage endpoint is reachable."""
        pass

    @abstractmethod
    def query_trace(
        self,
        trace_id: str,
        start_unix_ns: int,
        end_unix_ns: int,
        tenant_id: Optional[str] = None,
    ) -> QueryResult:
        """Query all spans for a specific trace within the time window."""
        pass
