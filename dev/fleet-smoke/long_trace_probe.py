#!/usr/bin/env python3
"""
Standalone Long-Trace Performance Probe (dev/fleet-smoke/long_trace_probe.py)

Controls a 60-second synthetic continuous root span and orchestrates a chain of
30 native hook events paced over 60 seconds. Descendants flow:
  candidate hook -> private pipe -> candidate daemon -> forwarding OTLP collector -> OTLP endpoint.
"""

import argparse
import http.server
import json
import os
import platform
import secrets
import shutil
import struct
import subprocess
import sys
import tempfile
import threading
import time
from typing import Any, Dict, List, Optional, Tuple
import urllib.error
import urllib.parse
import urllib.request
import uuid

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from native_context_probe import pipe_ready
from perf_delivery import start_bounded_capture, stop_daemon_gracefully
from perf_driver import (collect_environment, compute_sha256,
                         default_active_manifest_path, inspect_source_state,
                         verify_active_manifest)

MAX_BODY_BYTES = 2 * 1024 * 1024
MAX_SPANS = 200
MAX_ERRORS = 32
MAX_RESPONSE_BYTES = 64 * 1024
MAX_WALL_CLOCK_CATCHUP_SECONDS = 1.0


# ---------------------------------------------------------------------------
# Protobuf wire decoding helpers
# ---------------------------------------------------------------------------

def parse_varint(data: bytes, offset: int) -> Tuple[int, int]:
    res, shift = 0, 0
    while offset < len(data):
        b = data[offset]
        offset += 1
        res |= (b & 0x7F) << shift
        if not (b & 0x80):
            return res, offset
        shift += 7
        if shift >= 64:
            raise ValueError("varint overflow")
    raise ValueError("truncated varint")


def skip_field(data: bytes, offset: int, wire_type: int) -> int:
    if wire_type == 0:
        _, offset = parse_varint(data, offset)
        return offset
    elif wire_type == 1:
        if offset + 8 > len(data):
            raise ValueError("truncated 64-bit field")
        return offset + 8
    elif wire_type == 2:
        length, offset = parse_varint(data, offset)
        if offset + length > len(data):
            raise ValueError("truncated length-delimited field")
        return offset + length
    elif wire_type == 5:
        if offset + 4 > len(data):
            raise ValueError("truncated 32-bit field")
        return offset + 4
    else:
        raise ValueError(f"unsupported wire type {wire_type}")


def decode_key_value(data: bytes) -> Tuple[str, Any]:
    offset = 0
    key = ""
    val = None
    while offset < len(data):
        tag, offset = parse_varint(data, offset)
        f_num, w_type = tag >> 3, tag & 0x07
        if f_num == 1 and w_type == 2:
            length, offset = parse_varint(data, offset)
            key = data[offset:offset + length].decode("utf-8", errors="replace")
            offset += length
        elif f_num == 2 and w_type == 2:
            length, offset = parse_varint(data, offset)
            v_data = data[offset:offset + length]
            offset += length
            v_off = 0
            while v_off < len(v_data):
                v_tag, v_off = parse_varint(v_data, v_off)
                v_num, v_w = v_tag >> 3, v_tag & 0x07
                if v_num == 1 and v_w == 2:
                    v_len, v_off = parse_varint(v_data, v_off)
                    val = v_data[v_off:v_off + v_len].decode("utf-8", errors="replace")
                    v_off += v_len
                elif v_num == 2 and v_w == 0:
                    b_val, v_off = parse_varint(v_data, v_off)
                    val = bool(b_val)
                elif v_num == 3 and v_w == 0:
                    i_val, v_off = parse_varint(v_data, v_off)
                    val = i_val
                elif v_num == 4 and v_w == 1:
                    if v_off + 8 <= len(v_data):
                        val = struct.unpack("<d", v_data[v_off:v_off + 8])[0]
                    v_off += 8
                else:
                    v_off = skip_field(v_data, v_off, v_w)
        else:
            offset = skip_field(data, offset, w_type)
    return key, val


def decode_span(data: bytes) -> Dict[str, Any]:
    offset = 0
    span = {
        "trace_id": "",
        "span_id": "",
        "parent_span_id": "",
        "name": "",
        "kind": 0,
        "start_time_unix_nano": 0,
        "end_time_unix_nano": 0,
        "attributes": {},
    }
    while offset < len(data):
        tag, offset = parse_varint(data, offset)
        f_num, w_type = tag >> 3, tag & 0x07
        if f_num == 1 and w_type == 2:
            length, offset = parse_varint(data, offset)
            span["trace_id"] = data[offset:offset + length].hex()
            offset += length
        elif f_num == 2 and w_type == 2:
            length, offset = parse_varint(data, offset)
            span["span_id"] = data[offset:offset + length].hex()
            offset += length
        elif f_num == 4 and w_type == 2:
            length, offset = parse_varint(data, offset)
            span["parent_span_id"] = data[offset:offset + length].hex()
            offset += length
        elif f_num == 5 and w_type == 2:
            length, offset = parse_varint(data, offset)
            span["name"] = data[offset:offset + length].decode("utf-8", errors="replace")
            offset += length
        elif f_num == 6 and w_type == 0:
            k_val, offset = parse_varint(data, offset)
            span["kind"] = k_val
        elif f_num == 7 and w_type == 1:
            if offset + 8 <= len(data):
                span["start_time_unix_nano"] = struct.unpack("<Q", data[offset:offset + 8])[0]
            offset += 8
        elif f_num == 8 and w_type == 1:
            if offset + 8 <= len(data):
                span["end_time_unix_nano"] = struct.unpack("<Q", data[offset:offset + 8])[0]
            offset += 8
        elif f_num == 9 and w_type == 2:
            length, offset = parse_varint(data, offset)
            kv_data = data[offset:offset + length]
            offset += length
            k, v = decode_key_value(kv_data)
            if k:
                span["attributes"][k] = v
        else:
            offset = skip_field(data, offset, w_type)
    return span


def decode_export_trace_request(data: bytes) -> List[Dict[str, Any]]:
    spans = []
    offset = 0
    while offset < len(data):
        tag, offset = parse_varint(data, offset)
        f_num, w_type = tag >> 3, tag & 0x07
        if f_num == 1 and w_type == 2:
            rs_len, offset = parse_varint(data, offset)
            rs_data = data[offset:offset + rs_len]
            offset += rs_len
            rs_off = 0
            while rs_off < len(rs_data):
                rs_tag, rs_off = parse_varint(rs_data, rs_off)
                rs_f, rs_w = rs_tag >> 3, rs_tag & 0x07
                if rs_f == 2 and rs_w == 2:
                    ss_len, rs_off = parse_varint(rs_data, rs_off)
                    ss_data = rs_data[rs_off:rs_off + ss_len]
                    rs_off += ss_len
                    ss_off = 0
                    while ss_off < len(ss_data):
                        ss_tag, ss_off = parse_varint(ss_data, ss_off)
                        ss_f, ss_w = ss_tag >> 3, ss_tag & 0x07
                        if ss_f == 2 and ss_w == 2:
                            s_len, ss_off = parse_varint(ss_data, ss_off)
                            s_data = ss_data[ss_off:ss_off + s_len]
                            ss_off += s_len
                            spans.append(decode_span(s_data))
                        else:
                            ss_off = skip_field(ss_data, ss_off, ss_w)
                else:
                    rs_off = skip_field(rs_data, rs_off, rs_w)
        else:
            offset = skip_field(data, offset, w_type)
    return spans


def decode_json_spans(data: bytes) -> List[Dict[str, Any]]:
    spans = []
    obj = json.loads(data.decode("utf-8"))
    for rs in obj.get("resourceSpans", []):
        for ss in rs.get("scopeSpans", []):
            for s in ss.get("spans", []):
                trace_id = s.get("traceId", "")
                span_id = s.get("spanId", "")
                parent_span_id = s.get("parentSpanId", "")
                name = s.get("name", "")
                kind = s.get("kind", 0)
                start_nano = int(s.get("startTimeUnixNano", 0))
                end_nano = int(s.get("endTimeUnixNano", 0))
                attrs = {}
                for attr in s.get("attributes", []):
                    k = attr.get("key")
                    v = attr.get("value", {})
                    if isinstance(v, dict):
                        val = next(iter(v.values())) if v else None
                    else:
                        val = v
                    if k:
                        attrs[k] = val
                spans.append({
                    "trace_id": trace_id,
                    "span_id": span_id,
                    "parent_span_id": parent_span_id,
                    "name": name,
                    "kind": kind,
                    "start_time_unix_nano": start_nano,
                    "end_time_unix_nano": end_nano,
                    "attributes": attrs,
                })
    return spans


def decode_partial_success(data: bytes, content_type: str) -> Dict[str, Any]:
    res = {"rejected_spans": 0, "error_message": ""}
    if not data:
        return res
    if "json" in content_type.lower() or data.lstrip().startswith(b"{"):
        parsed = json.loads(data.decode("utf-8"))
        if not isinstance(parsed, dict):
            raise ValueError("OTLP JSON response root must be an object")
        partial = parsed.get("partialSuccess", {})
        if not isinstance(partial, dict):
            raise ValueError("OTLP JSON partialSuccess must be an object")
        res["rejected_spans"] = int(partial.get("rejectedSpans", 0))
        res["error_message"] = str(partial.get("errorMessage", ""))
        return res
    offset = 0
    while offset < len(data):
        tag, offset = parse_varint(data, offset)
        f_num, w_type = tag >> 3, tag & 0x07
        if f_num == 1 and w_type == 2:
            ps_len, offset = parse_varint(data, offset)
            if offset + ps_len > len(data):
                raise ValueError("truncated OTLP partial-success response")
            ps_data = data[offset:offset + ps_len]
            offset += ps_len
            ps_off = 0
            while ps_off < len(ps_data):
                ps_tag, ps_off = parse_varint(ps_data, ps_off)
                ps_f, ps_w = ps_tag >> 3, ps_tag & 0x07
                if ps_f == 1 and ps_w == 0:
                    rejected, ps_off = parse_varint(ps_data, ps_off)
                    res["rejected_spans"] = rejected
                elif ps_f == 2 and ps_w == 2:
                    msg_len, ps_off = parse_varint(ps_data, ps_off)
                    if ps_off + msg_len > len(ps_data):
                        raise ValueError("truncated OTLP partial-success error message")
                    res["error_message"] = ps_data[ps_off:ps_off + msg_len].decode("utf-8", errors="replace")
                    ps_off += msg_len
                else:
                    ps_off = skip_field(ps_data, ps_off, ps_w)
        else:
            offset = skip_field(data, offset, w_type)
    return res


def otlp_response_outcome(status_code: int, data: bytes, content_type: str) -> Dict[str, Any]:
    try:
        partial = decode_partial_success(data, content_type)
    except (TypeError, ValueError, json.JSONDecodeError) as exc:
        return {
            "ok": False,
            "rejected_spans": 0,
            "error_message": f"Malformed OTLP response: {exc}",
        }
    rejected = partial["rejected_spans"]
    message = partial["error_message"]
    return {
        "ok": status_code == 200 and rejected == 0 and not message,
        "rejected_spans": rejected,
        "error_message": message,
    }


# ---------------------------------------------------------------------------
# Forwarding OTLP Collector Server
# ---------------------------------------------------------------------------

class ForwardingCollectorHandler(http.server.BaseHTTPRequestHandler):
    def setup(self):
        super().setup()
        self.connection.settimeout(3.0)

    def do_POST(self):
        if self.path not in ("/v1/traces", "/v1/metrics"):
            self.send_error(404, "Unknown OTLP path")
            return
        try:
            content_length = int(self.headers.get("Content-Length", ""))
        except ValueError:
            self.send_error(400, "Invalid Content-Length")
            return
        if not 0 <= content_length <= MAX_BODY_BYTES:
            self.send_error(413, "Payload Too Large")
            return
        body = self.rfile.read(content_length)
        if len(body) != content_length:
            self.send_error(400, "Truncated request body")
            return
        content_type = self.headers.get("Content-Type", "application/x-protobuf")

        upstream_url = f"{self.server.upstream_endpoint.rstrip('/')}{self.path}"
        req = urllib.request.Request(
            upstream_url,
            data=body,
            headers={
                "Content-Type": content_type,
                "User-Agent": "agent-otel-long-trace-collector/1.0"
            },
            method="POST"
        )
        forward_ok = False
        status_code = 502
        resp_body = b""
        rejected = 0
        err_msg = ""
        try:
            with urllib.request.urlopen(req, timeout=3.0) as resp:
                status_code = resp.status
                resp_body = resp.read(MAX_RESPONSE_BYTES + 1)
                if len(resp_body) > MAX_RESPONSE_BYTES:
                    raise ValueError("upstream response exceeded configured bound")
                resp_ct = resp.headers.get("Content-Type", "")
                outcome = otlp_response_outcome(status_code, resp_body, resp_ct)
                rejected = outcome["rejected_spans"]
                err_msg = outcome["error_message"]
                forward_ok = outcome["ok"]
        except urllib.error.HTTPError as exc:
            status_code = exc.code
            try:
                resp_body = exc.read(MAX_RESPONSE_BYTES + 1)
                if len(resp_body) > MAX_RESPONSE_BYTES:
                    resp_body = b""
                    err_msg = "upstream error response exceeded configured bound"
            except Exception:
                resp_body = b""
            if not err_msg:
                err_msg = f"HTTPError {exc.code}: {exc.reason}"
        except Exception as exc:
            status_code = 502
            err_msg = f"TransportError: {exc}"

        extracted_spans = []
        if "/v1/traces" in self.path:
            if "json" in content_type.lower() or body.lstrip().startswith(b"{"):
                try:
                    extracted_spans = decode_json_spans(body)
                except Exception as exc:
                    self.server.record_error(f"json span decode error: {exc}")
            else:
                try:
                    extracted_spans = decode_export_trace_request(body)
                except Exception as exc:
                    self.server.record_error(f"proto span decode error: {exc}")

        self.server.record_event(self.path, len(body), status_code, forward_ok, rejected, err_msg, extracted_spans)

        self.send_response(status_code)
        if resp_body:
            self.send_header("Content-Type", "application/x-protobuf")
            self.send_header("Content-Length", str(len(resp_body)))
            self.end_headers()
            self.wfile.write(resp_body)
        else:
            ack = b""
            self.send_header("Content-Type", "application/x-protobuf")
            self.send_header("Content-Length", "0")
            self.end_headers()
            self.wfile.write(ack)

    def log_message(self, format, *args):
        pass


class ForwardingCollectorServer(http.server.ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self, server_address, upstream_endpoint: str):
        super().__init__(server_address, ForwardingCollectorHandler)
        self.upstream_endpoint = upstream_endpoint
        self.lock = threading.Lock()
        self.condition = threading.Condition(self.lock)
        self.spans: List[Dict[str, Any]] = []
        self.requests_count = 0
        self.forwarded_ok_count = 0
        self.forward_error_count = 0
        self.rejected_spans_total = 0
        self.errors: List[str] = []

    def record_error(self, err: str) -> None:
        with self.lock:
            if len(self.errors) < MAX_ERRORS:
                self.errors.append(err)

    def record_event(self, path: str, body_len: int, status_code: int, forward_ok: bool,
                     rejected: int, err_msg: str, spans: List[Dict[str, Any]]) -> None:
        with self.condition:
            self.requests_count += 1
            if forward_ok:
                self.forwarded_ok_count += 1
            else:
                self.forward_error_count += 1
            self.rejected_spans_total += rejected
            if err_msg and len(self.errors) < MAX_ERRORS:
                self.errors.append(f"[{path}] status={status_code} rejected={rejected} error={err_msg}")
            for s in spans:
                if len(self.spans) < MAX_SPANS:
                    s["arrival_monotonic"] = time.monotonic()
                    self.spans.append(s)
            self.condition.notify_all()

    def wait_for_span_with_parent(self, trace_id: str, parent_span_id: str,
                                  timeout: float) -> Optional[Dict[str, Any]]:
        deadline = time.monotonic() + timeout
        with self.condition:
            while True:
                for s in self.spans:
                    if (s["trace_id"] == trace_id
                            and s["parent_span_id"] == parent_span_id):
                        return s
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    return None
                self.condition.wait(remaining)


# ---------------------------------------------------------------------------
# Hook execution & workspace fixtures
# ---------------------------------------------------------------------------

def execute_hook(hook_bin: str, pipe_name: str, idx: int, trace_id: str, parent_span_id: str,
                 cwd: str, deadline: float = float("inf")) -> Dict[str, Any]:
    t_start = time.monotonic()
    if t_start >= deadline:
        return {
            "index": idx, "trace_id": trace_id, "parent_span_id": parent_span_id,
            "send_start": t_start, "hook_end": t_start, "duration_ms": 0.0,
            "exit_code": None, "timed_out": True, "response_ok": False,
            "stderr": "scenario deadline exceeded before hook spawn"
        }
    env = os.environ.copy()
    env.pop("AGENT_OTEL_BENCH_HANDLE", None)
    env.pop("TRACESTATE", None)
    env.update({
        "AGENT_OTEL_PIPE": pipe_name,
        "AGY_OTEL_PIPE": pipe_name,
        "AGENT_OTEL_SOCKET": pipe_name,
        "TRACEPARENT": f"00-{trace_id}-{parent_span_id}-01",
    })
    abs_ws = os.path.abspath(cwd)
    payload = json.dumps({
        "conversationId": trace_id,
        "toolCall": {"name": "Bash", "id": f"architecture-{idx}"},
        "stepIdx": idx,
        "workspace_path": abs_ws,
    }).encode("utf-8")
    cf = subprocess.CREATE_NO_WINDOW if platform.system() == "Windows" else 0
    proc = subprocess.Popen(
        [hook_bin, "--client", "codex", "PostToolUse"],
        env=env, cwd=abs_ws, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
        stderr=subprocess.PIPE, creationflags=cf, shell=False
    )
    hook_capture, hook_readers = start_bounded_capture(proc)
    timed_out = False
    try:
        timeout = max(0.001, min(5.0, deadline - time.monotonic()))
        proc.stdin.write(payload)
        proc.stdin.close()
        proc.wait(timeout=timeout)
    except subprocess.TimeoutExpired:
        timed_out = True
        proc.kill()
        proc.wait(timeout=2.0)
    finally:
        for reader in hook_readers:
            reader.join(timeout=1.0)
    stdout = hook_capture.value("stdout")
    stderr = hook_capture.value("stderr")
    t_end = time.monotonic()

    json_ok = False
    if proc.returncode == 0 and not timed_out:
        try:
            text_out = stdout.decode("utf-8").strip()
            if not text_out:
                raise ValueError("empty hook response")
            parsed_out = json.loads(text_out)
            json_ok = (parsed_out == {})
        except Exception:
            json_ok = False

    return {
        "index": idx,
        "trace_id": trace_id,
        "parent_span_id": parent_span_id,
        "send_start": t_start,
        "hook_end": t_end,
        "duration_ms": (t_end - t_start) * 1000.0,
        "exit_code": proc.returncode,
        "timed_out": timed_out,
        "response_ok": (proc.returncode == 0 and json_ok and not timed_out
                        and not hook_capture.exceeded.is_set()
                        and not any(reader.is_alive() for reader in hook_readers)),
        "output_bounded": not hook_capture.exceeded.is_set(),
        "output_drain_complete": not any(reader.is_alive() for reader in hook_readers),
        "stderr": stderr.decode("utf-8", errors="replace"),
    }


def inspect_installed_bridge() -> Dict[str, Any]:
    manifest_path = default_active_manifest_path()
    if manifest_path is None:
        return {"verified": False, "reason": "active_manifest_path_unavailable"}
    return verify_active_manifest(manifest_path)


def make_git_workspace(prefix: str = "longtrace") -> str:
    path = tempfile.mkdtemp(prefix=f"agent-otel-{prefix}-")
    try:
        subprocess.run(["git", "init", "-b", "lab"], cwd=path, capture_output=True, check=True)
    except subprocess.CalledProcessError:
        subprocess.run(["git", "init"], cwd=path, capture_output=True, check=True)
        subprocess.run(["git", "checkout", "-b", "lab"], cwd=path, capture_output=True, check=True)
    subprocess.run(["git", "config", "user.name", "fleet-smoke-lab"], cwd=path, capture_output=True)
    subprocess.run(["git", "config", "user.email", "fleet-smoke-lab@example.internal"], cwd=path, capture_output=True)
    readme = os.path.join(path, "README.md")
    with open(readme, "w", encoding="utf-8") as f:
        f.write("# controlled long trace lab\n")
    subprocess.run(["git", "add", "README.md"], cwd=path, capture_output=True)
    subprocess.run(["git", "commit", "-m", "init lab fixture"], cwd=path, capture_output=True)
    return path


def cleanup_git_workspace(path: str) -> bool:
    def _onerror(func, p, _):
        try:
            os.chmod(p, 0o777)
            func(p)
        except Exception:
            pass

    if path and os.path.isdir(path):
        shutil.rmtree(path, onerror=_onerror)
        return not os.path.exists(path)
    return True


def pacing_offsets(duration: float, events: int) -> List[float]:
    if duration <= 0 or events <= 0:
        raise ValueError("duration and events must be positive")
    interval = duration / events
    return [idx * interval for idx in range(events)]


def await_root_end(root_start_ns: int, monotonic_start: float, duration: float,
                   monotonic_fn=time.monotonic, wall_ns_fn=time.time_ns,
                   sleep_fn=time.sleep) -> Dict[str, Any]:
    monotonic_target = monotonic_start + duration
    wall_target_ns = root_start_ns + int(duration * 1_000_000_000)
    hard_deadline = monotonic_target + MAX_WALL_CLOCK_CATCHUP_SECONDS
    while True:
        observed_mono = monotonic_fn()
        observed_wall_ns = wall_ns_fn()
        monotonic_complete = observed_mono >= monotonic_target
        wall_complete = observed_wall_ns >= wall_target_ns
        if monotonic_complete and wall_complete:
            return {
                "complete": True,
                "end_time_unix_nano": observed_wall_ns,
                "monotonic_elapsed_sec": observed_mono - monotonic_start,
                "wall_elapsed_sec": (observed_wall_ns - root_start_ns) / 1_000_000_000.0,
                "max_catchup_sec": MAX_WALL_CLOCK_CATCHUP_SECONDS,
            }
        if observed_mono >= hard_deadline:
            return {
                "complete": False,
                "end_time_unix_nano": observed_wall_ns,
                "monotonic_elapsed_sec": observed_mono - monotonic_start,
                "wall_elapsed_sec": (observed_wall_ns - root_start_ns) / 1_000_000_000.0,
                "max_catchup_sec": MAX_WALL_CLOCK_CATCHUP_SECONDS,
            }
        monotonic_remaining = max(0.0, monotonic_target - observed_mono)
        wall_remaining = max(0.0, (wall_target_ns - observed_wall_ns) / 1_000_000_000.0)
        hard_remaining = hard_deadline - observed_mono
        sleep_fn(max(0.0005, min(max(monotonic_remaining, wall_remaining), hard_remaining, 0.25)))


def analyze_long_trace(trace_id: str, root_span_id: str, expected_events: int,
                       captured_spans: List[Dict[str, Any]]) -> Dict[str, Any]:
    trace_spans = [span for span in captured_spans if span.get("trace_id") == trace_id]
    unexpected_trace_ids = sorted({
        str(span.get("trace_id")) for span in captured_spans
        if span.get("trace_id") != trace_id
    })
    logical_by_id: Dict[str, Dict[str, Any]] = {}
    conflicting_duplicate_ids = []
    for span in trace_spans:
        span_id = str(span.get("span_id", ""))
        prior = logical_by_id.get(span_id)
        if prior is None:
            logical_by_id[span_id] = span
        elif any(prior.get(key) != span.get(key) for key in
                 ("trace_id", "span_id", "parent_span_id", "name")):
            conflicting_duplicate_ids.append(span_id)

    logical = list(logical_by_id.values())
    invalid_span_ids = sorted({
        str(span.get("span_id", "")) for span in logical
        if len(str(span.get("span_id", ""))) != 16
        or not all(ch in "0123456789abcdefABCDEF" for ch in str(span.get("span_id", "")))
        or int(str(span.get("span_id")), 16) == 0
    })
    ordered: List[Dict[str, Any]] = []
    wrong_parent_ids: List[str] = []
    current_parent = root_span_id
    remaining = {str(span.get("span_id")): span for span in logical}
    for _ in range(expected_events):
        children = [span for span in remaining.values()
                    if span.get("parent_span_id") == current_parent]
        if len(children) != 1:
            wrong_parent_ids.extend(str(span.get("span_id")) for span in children)
            break
        child = children[0]
        ordered.append(child)
        current_parent = str(child.get("span_id"))
        remaining.pop(current_parent, None)

    orphan_ids = sorted(remaining)
    exact_chain = (len(ordered) == expected_events and not orphan_ids
                   and not conflicting_duplicate_ids and not invalid_span_ids)
    return {
        "valid": (exact_chain and not unexpected_trace_ids),
        "expected_count": expected_events,
        "captured_count": len(captured_spans),
        "trace_capture_count": len(trace_spans),
        "logical_count": len(logical),
        "duplicate_capture_count": len(trace_spans) - len(logical),
        "conflicting_duplicate_ids": sorted(set(conflicting_duplicate_ids)),
        "unexpected_trace_ids": unexpected_trace_ids,
        "invalid_span_ids": invalid_span_ids,
        "wrong_parent_ids": sorted(set(wrong_parent_ids)),
        "orphan_ids": orphan_ids,
        "ordered": ordered,
    }


def daemon_reconciliation_ok(diagnostics: Optional[Dict[str, Any]], expected_events: int) -> bool:
    if not isinstance(diagnostics, dict) or expected_events <= 0:
        return False
    ingress = diagnostics.get("ingress")
    pipeline = diagnostics.get("pipeline")
    if not isinstance(ingress, dict) or not isinstance(pipeline, dict):
        return False
    if any(pipeline.get(key) != expected_events for key in
           ("transformed", "accepted")) or ingress.get("admitted") != expected_events:
        return False
    pipeline_zero = ("invalid", "span_size", "export_capacity", "rejected", "unknown",
                     "shutdown_dropped", "queued_bytes", "queued_items")
    ingress_zero = ("invalid", "capacity", "read_failed", "read_deadline", "reserved_bytes")
    return (all(pipeline.get(key) == 0 for key in pipeline_zero)
            and all(ingress.get(key) == 0 for key in ingress_zero))


def validate_upstream_endpoint(endpoint: str) -> str:
    parsed = urllib.parse.urlparse(endpoint)
    if (parsed.scheme != "http" or parsed.hostname not in {"127.0.0.1", "localhost"}
            or parsed.port != 4318 or parsed.path not in {"", "/"}
            or parsed.params or parsed.query or parsed.fragment or parsed.username or parsed.password):
        raise ValueError("endpoint must be the local OTLP HTTP receiver http://127.0.0.1:4318")
    return "http://127.0.0.1:4318"


# ---------------------------------------------------------------------------
# Main probe controller
# ---------------------------------------------------------------------------

class OwnedProbeResources:
    def __init__(self) -> None:
        self.workspace: Optional[str] = None
        self.collector: Optional[ForwardingCollectorServer] = None
        self.collector_thread: Optional[threading.Thread] = None
        self.daemon: Optional[subprocess.Popen] = None
        self.capture = None
        self.readers: List[threading.Thread] = []
        self.pipe_name: Optional[str] = None

    def cleanup(self) -> None:
        if self.daemon is not None and self.daemon.poll() is None:
            try:
                if self.capture is not None and self.pipe_name is not None:
                    stop_daemon_gracefully(
                        self.daemon, self.pipe_name, self.capture, self.readers
                    )
            except Exception:
                pass
            if self.daemon.poll() is None:
                self.daemon.terminate()
                try:
                    self.daemon.wait(timeout=2.0)
                except subprocess.TimeoutExpired:
                    self.daemon.kill()
                    self.daemon.wait(timeout=2.0)
        if self.collector is not None:
            try:
                self.collector.shutdown()
            except Exception:
                pass
            try:
                self.collector.server_close()
            except Exception:
                pass
        if self.collector_thread is not None:
            self.collector_thread.join(timeout=2.0)
        if self.workspace is not None:
            cleanup_git_workspace(self.workspace)


def _run_probe_owned(daemon_bin: str, hook_bin: str, upstream_endpoint: str,
                     duration: int, events: int,
                     owned: OwnedProbeResources) -> Dict[str, Any]:
    upstream_endpoint = validate_upstream_endpoint(upstream_endpoint)
    run_id = uuid.uuid4().hex
    while True:
        trace_id = secrets.token_hex(16)
        if any(c != "0" for c in trace_id):
            break
    while True:
        root_span_id = secrets.token_hex(8)
        if any(c != "0" for c in root_span_id):
            break

    service_name = f"agent-otel-long-trace-{run_id[:8]}"
    probe_start_mono = time.monotonic()
    probe_deadline = probe_start_mono + duration + 25.0

    integrity_before = {
        "candidate_daemon_sha256": compute_sha256(daemon_bin),
        "candidate_hook_sha256": compute_sha256(hook_bin),
        "installed_bridge": inspect_installed_bridge(),
        "source_state": inspect_source_state(),
        "environment": collect_environment(),
    }

    workspace_dir = make_git_workspace("longtrace")
    owned.workspace = workspace_dir
    collector_server = ForwardingCollectorServer(("127.0.0.1", 0), upstream_endpoint)
    owned.collector = collector_server
    collector_port = collector_server.server_address[1]
    collector_thread = threading.Thread(target=collector_server.serve_forever, daemon=True)
    collector_thread.start()
    owned.collector_thread = collector_thread

    if os.name == "nt":
        pipe_name = f"\\\\.\\pipe\\agent-otel-longtrace-{run_id}"
    else:
        pipe_name = os.path.join(tempfile.gettempdir(), f"agent-otel-longtrace-{run_id}.sock")
    owned.pipe_name = pipe_name

    daemon_env = os.environ.copy()
    daemon_env.pop("TRACEPARENT", None)
    daemon_env.pop("TRACESTATE", None)
    daemon_env.pop("AGENT_OTEL_BENCH_HANDLE", None)
    daemon_env.update({
        "AGENT_OTEL_PIPE": pipe_name,
        "AGY_OTEL_PIPE": pipe_name,
        "AGENT_OTEL_SOCKET": pipe_name,
        "OTEL_EXPORTER_OTLP_ENDPOINT": f"http://127.0.0.1:{collector_port}",
        "OTEL_SERVICE_NAME": service_name,
        "AGENT_OTEL_BATCH_SIZE": "1",
        "OTEL_BSP_MAX_EXPORT_BATCH_SIZE": "1",
        "AGENT_OTEL_TIMEOUT_MS": "50",
        "AGENT_OTEL_BATCH_TIMEOUT_MS": "50",
        "OTEL_BSP_SCHEDULE_DELAY": "50",
        "AGENT_OTEL_IDLE_TIMEOUT_MS": "0",
        "AGENT_OTEL_DAEMON_IDLE_TIMEOUT_MS": "0",
    })

    cf = subprocess.CREATE_NO_WINDOW if platform.system() == "Windows" else 0
    try:
        daemon_proc = subprocess.Popen(
            [daemon_bin, "daemon"],
            env=daemon_env,
            cwd=workspace_dir,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            creationflags=cf,
            shell=False,
        )
        owned.daemon = daemon_proc
    except Exception:
        collector_server.shutdown()
        collector_server.server_close()
        collector_thread.join(timeout=2.0)
        cleanup_git_workspace(workspace_dir)
        raise
    capture, readers = start_bounded_capture(daemon_proc)
    owned.capture = capture
    owned.readers = readers

    if not pipe_ready(pipe_name, time.monotonic() + 5.0):
        daemon_shutdown = stop_daemon_gracefully(daemon_proc, pipe_name, capture, readers)
        collector_server.shutdown()
        collector_server.server_close()
        collector_thread.join(timeout=2.0)
        cleanup_ok = cleanup_git_workspace(workspace_dir)
        return {
            "schema": "agent-otel-long-trace/v1",
            "run_id": run_id,
            "trace_id": trace_id,
            "root_span_id": root_span_id,
            "service_name": service_name,
            "start_ms": int(time.time() * 1000),
            "end_ms": int(time.time() * 1000),
            "duration_sec": 0.0,
            "requested_duration_sec": duration,
            "pacing_interval_sec": duration / events,
            "expected_events": events,
            "received_events": 0,
            "expected": [],
            "received": [],
            "hook_executions": [],
            "transport": {"error": "daemon pipe failed to become ready"},
            "backend_visibility": {"status": "not_checked", "reason": "root MCP read required externally"},
            "diagnostics": {"daemon": daemon_shutdown},
            "cleanup": {"daemon_stopped": daemon_proc.poll() is not None,
                        "workspace_removed": cleanup_ok, "collector_stopped": not collector_thread.is_alive()},
            "integrity": {"before": integrity_before, "after": {
                "candidate_daemon_sha256": compute_sha256(daemon_bin),
                "candidate_hook_sha256": compute_sha256(hook_bin),
                "installed_bridge": inspect_installed_bridge(),
                "source_state": inspect_source_state(),
                "environment": collect_environment(),
            }},
            "hard_gates": {"pipe_ready": False},
            "verdict": "failed",
        }

    expected_chain: List[Dict[str, Any]] = []
    received_chain: List[Dict[str, Any]] = []
    hook_executions: List[Dict[str, Any]] = []
    arrival_offsets: Dict[str, Optional[float]] = {
        "first_arrival_offset_ms": None,
        "middle_arrival_offset_ms": None,
        "last_arrival_offset_ms": None,
    }

    current_parent_span_id = root_span_id
    root_start_ns = time.time_ns()
    scenario_start_mono = time.monotonic()
    offsets = pacing_offsets(float(duration), events)
    aborted_error: Optional[str] = None

    for idx in range(events):
        target_mono = scenario_start_mono + offsets[idx]
        now_mono = time.monotonic()
        if target_mono > now_mono:
            time.sleep(target_mono - now_mono)

        expected_chain.append({
            "index": idx,
            "parent_span_id": current_parent_span_id,
            "tool_call_id": f"architecture-{idx}",
        })

        h_res = execute_hook(
            hook_bin=hook_bin,
            pipe_name=pipe_name,
            idx=idx,
            trace_id=trace_id,
            parent_span_id=current_parent_span_id,
            cwd=workspace_dir,
            deadline=probe_deadline,
        )
        hook_executions.append(h_res)

        if not h_res.get("response_ok"):
            aborted_error = f"hook {idx} execution failed (exit_code={h_res.get('exit_code')}, timed_out={h_res.get('timed_out')})"
            break

        span = collector_server.wait_for_span_with_parent(
            trace_id, current_parent_span_id, timeout=5.0
        )
        if not span:
            aborted_error = f"collector timed out waiting for native span with parent {current_parent_span_id} for event {idx}"
            break

        received_chain.append(span)
        expected_chain[-1]["span_id"] = span["span_id"]
        offset_ms = (span.get("arrival_monotonic", time.monotonic()) - scenario_start_mono) * 1000.0
        if idx == 0:
            arrival_offsets["first_arrival_offset_ms"] = offset_ms
        if idx == events // 2:
            arrival_offsets["middle_arrival_offset_ms"] = offset_ms
        if idx == events - 1:
            arrival_offsets["last_arrival_offset_ms"] = offset_ms

        current_parent_span_id = span["span_id"]

    root_finish = await_root_end(root_start_ns, scenario_start_mono, float(duration))
    root_end_ns = root_finish["end_time_unix_nano"]
    actual_duration_sec = (root_end_ns - root_start_ns) / 1_000_000_000.0

    daemon_shutdown = stop_daemon_gracefully(daemon_proc, pipe_name, capture, readers)

    time.sleep(0.1)
    collector_server.shutdown()
    collector_server.server_close()
    collector_thread.join(timeout=2.0)

    root_payload = {
        "resourceSpans": [
            {
                "resource": {
                    "attributes": [
                        {
                            "key": "service.name",
                            "value": {"stringValue": service_name}
                        }
                    ]
                },
                "scopeSpans": [
                    {
                        "scope": {
                            "name": "fleet-smoke-lab.controller"
                        },
                        "spans": [
                            {
                                "traceId": trace_id,
                                "spanId": root_span_id,
                                "name": "invoke_agent controlled_long_trace",
                                "kind": 1,
                                "startTimeUnixNano": str(root_start_ns),
                                "endTimeUnixNano": str(root_end_ns),
                                "attributes": [
                                    {
                                        "key": "origin",
                                        "value": {"stringValue": "fleet-smoke-lab.controlled-protocol"}
                                    },
                                    {
                                        "key": "agent.execution.mode",
                                        "value": {"stringValue": "test"}
                                    }
                                ],
                                "status": {"code": 1}
                            }
                        ]
                    }
                ]
            }
        ]
    }

    root_export_status: Dict[str, Any] = {
        "sent": False,
        "status_code": None,
        "rejected_spans": 0,
        "error": None,
        "ok": False,
    }

    try:
        root_req = urllib.request.Request(
            f"{upstream_endpoint.rstrip('/')}/v1/traces",
            data=json.dumps(root_payload).encode("utf-8"),
            headers={
                "Content-Type": "application/json",
                "User-Agent": "agent-otel-long-trace-controller/1.0"
            },
            method="POST"
        )
        with urllib.request.urlopen(root_req, timeout=5.0) as resp:
            root_export_status["sent"] = True
            root_export_status["status_code"] = resp.status
            body_bytes = resp.read(MAX_RESPONSE_BYTES + 1)
            if len(body_bytes) > MAX_RESPONSE_BYTES:
                raise ValueError("root OTLP response exceeded configured bound")
            ct = resp.headers.get("Content-Type", "")
            outcome = otlp_response_outcome(resp.status, body_bytes, ct)
            root_export_status["rejected_spans"] = outcome["rejected_spans"]
            if outcome["error_message"]:
                root_export_status["error"] = outcome["error_message"]
            root_export_status["ok"] = outcome["ok"]
    except urllib.error.HTTPError as exc:
        root_export_status["sent"] = True
        root_export_status["status_code"] = exc.code
        root_export_status["error"] = f"HTTPError {exc.code}: {exc.reason}"
    except Exception as exc:
        root_export_status["error"] = f"TransportError: {exc}"

    workspace_removed = cleanup_git_workspace(workspace_dir)

    all_hooks_response_valid = (len(hook_executions) == events and
                                all(h.get("response_ok") for h in hook_executions))
    with collector_server.lock:
        captured_spans = list(collector_server.spans)
    chain_evidence = analyze_long_trace(trace_id, root_span_id, events, captured_spans)
    received_chain = chain_evidence["ordered"]
    event_count_met = chain_evidence["logical_count"] == events
    span_ids = [s["span_id"] for s in received_chain]
    unique_span_ids = (len(span_ids) == events and len(span_ids) == len(set(span_ids))
                       and root_span_id not in set(span_ids))

    walltimes_ok = True
    for s in received_chain:
        st = s.get("start_time_unix_nano", 0)
        et = s.get("end_time_unix_nano", 0)
        if st > 0 and st < root_start_ns:
            walltimes_ok = False
        if et > 0 and et > root_end_ns:
            walltimes_ok = False
    root_contains_native_walltimes = (
        root_finish["complete"] and actual_duration_sec >= float(duration) and walltimes_ok
    )

    chain_parent_exact = chain_evidence["valid"]

    all_same_trace = (len(received_chain) == events and
                      all(s.get("trace_id") == trace_id for s in received_chain))

    diag = daemon_shutdown.get("diagnostics") or {}
    daemon_accepted_zero_errors = (
        daemon_shutdown.get("measurement_status") == "measured"
        and daemon_reconciliation_ok(diag, events)
    )

    root_transport_ok = bool(root_export_status.get("ok"))
    descendant_transport_ok = (
        collector_server.forward_error_count == 0
        and collector_server.rejected_spans_total == 0
        and not collector_server.errors
        and chain_evidence["logical_count"] == events
    )
    cleanup_gate = bool(workspace_removed and daemon_shutdown.get("exit_code") is not None
                        and not collector_thread.is_alive())

    integrity_after = {
        "candidate_daemon_sha256": compute_sha256(daemon_bin),
        "candidate_hook_sha256": compute_sha256(hook_bin),
        "installed_bridge": inspect_installed_bridge(),
        "source_state": inspect_source_state(),
        "environment": collect_environment(),
    }
    candidate_binaries_intact = (
        integrity_before["candidate_daemon_sha256"] is not None
        and integrity_before["candidate_hook_sha256"] is not None
        and integrity_before["candidate_daemon_sha256"] == integrity_after["candidate_daemon_sha256"]
        and integrity_before["candidate_hook_sha256"] == integrity_after["candidate_hook_sha256"]
    )
    installed_bridge_intact = (
        integrity_before["installed_bridge"].get("verified") is True
        and integrity_after["installed_bridge"].get("verified") is True
        and integrity_before["installed_bridge"] == integrity_after["installed_bridge"]
    )
    source_before = integrity_before["source_state"]
    source_after = integrity_after["source_state"]
    source_snapshot_valid = (
        isinstance(source_before.get("candidate_git_revision"), str)
        and len(source_before["candidate_git_revision"]) == 40
        and isinstance(source_before.get("is_dirty"), bool)
        and isinstance(source_before.get("git_diff_head_sha256"), str)
        and len(source_before["git_diff_head_sha256"]) == 64
    )
    source_intact = source_snapshot_valid and source_before == source_after

    hard_gates = {
        "all_hooks_response_valid": all_hooks_response_valid,
        "event_count_met": event_count_met,
        "unique_span_ids": unique_span_ids,
        "root_contains_native_walltimes": root_contains_native_walltimes,
        "chain_parent_exact": chain_parent_exact,
        "all_same_trace": all_same_trace,
        "daemon_accepted_zero_errors": daemon_accepted_zero_errors,
        "descendant_transport_ok": descendant_transport_ok,
        "root_transport_ok": root_transport_ok,
        "candidate_binaries_intact": candidate_binaries_intact,
        "installed_bridge_intact": installed_bridge_intact,
        "source_intact": source_intact,
        "cleanup": cleanup_gate,
    }

    passed = all(hard_gates.values())

    report = {
        "schema": "agent-otel-long-trace/v1",
        "run_id": run_id,
        "trace_id": trace_id,
        "root_span_id": root_span_id,
        "service_name": service_name,
        "start_ms": int(root_start_ns // 1_000_000),
        "end_ms": int(root_end_ns // 1_000_000),
        "duration_sec": actual_duration_sec,
        "requested_duration_sec": duration,
        "pacing_interval_sec": duration / events,
        "expected_events": events,
        "received_events": len(received_chain),
        "arrival_progress": arrival_offsets,
        "expected": expected_chain,
        "received": received_chain,
        "trace_evidence": {key: value for key, value in chain_evidence.items() if key != "ordered"},
        "hook_executions": hook_executions,
        "transport": {
            "collector_requests": collector_server.requests_count,
            "collector_forwarded_ok": collector_server.forwarded_ok_count,
            "collector_forward_errors": collector_server.forward_error_count,
            "collector_rejected_spans": collector_server.rejected_spans_total,
            "root_export": root_export_status,
        },
        "backend_visibility": {
            "status": "not_checked",
            "reason": "root MCP read required externally"
        },
        "diagnostics": {
            "daemon": daemon_shutdown,
            "collector_errors": collector_server.errors,
            "abort_reason": aborted_error,
            "root_clock": root_finish,
        },
        "cleanup": {
            "daemon_stopped": (daemon_shutdown.get("exit_code") is not None),
            "workspace_removed": workspace_removed,
            "collector_stopped": not collector_thread.is_alive(),
        },
        "integrity": {
            "before": integrity_before,
            "after": integrity_after,
        },
        "hard_gates": hard_gates,
        "verdict": "passed" if passed else "failed",
    }
    return report


def run_probe(daemon_bin: str, hook_bin: str, upstream_endpoint: str,
              duration: int, events: int) -> Dict[str, Any]:
    owned = OwnedProbeResources()
    try:
        return _run_probe_owned(
            daemon_bin, hook_bin, upstream_endpoint, duration, events, owned
        )
    finally:
        owned.cleanup()


def main() -> None:
    parser = argparse.ArgumentParser(description="Standalone continuous long-trace probe")
    parser.add_argument("--daemon-bin", required=True, help="Path to candidate agent-otel-bridge daemon binary")
    parser.add_argument("--hook-bin", required=True, help="Path to candidate agent-hook binary")
    parser.add_argument("--endpoint", default=os.getenv("OTEL_EXPORTER_OTLP_ENDPOINT", "http://127.0.0.1:4318"),
                        help="Upstream OTLP HTTP endpoint base URL")
    parser.add_argument("--duration", type=int, default=60, help="Total trace duration in seconds [10..120]")
    parser.add_argument("--events", type=int, default=30, help="Total native hook events to pace [3..120]")
    parser.add_argument("--output", default=None, help="Optional path to write JSON report output")
    args = parser.parse_args()

    if not (10 <= args.duration <= 120):
        sys.stderr.write(f"Error: --duration must be in range [10, 120], got {args.duration}\n")
        sys.exit(2)
    if not (3 <= args.events <= 120):
        sys.stderr.write(f"Error: --events must be in range [3, 120], got {args.events}\n")
        sys.exit(2)

    daemon_bin = os.path.abspath(args.daemon_bin)
    hook_bin = os.path.abspath(args.hook_bin)
    if not os.path.isfile(daemon_bin):
        sys.stderr.write(f"Error: daemon binary not found at {daemon_bin}\n")
        sys.exit(2)
    if not os.path.isfile(hook_bin):
        sys.stderr.write(f"Error: hook binary not found at {hook_bin}\n")
        sys.exit(2)
    try:
        endpoint = validate_upstream_endpoint(args.endpoint)
    except ValueError as exc:
        sys.stderr.write(f"Error: {exc}\n")
        sys.exit(2)

    report = run_probe(
        daemon_bin=daemon_bin,
        hook_bin=hook_bin,
        upstream_endpoint=endpoint,
        duration=args.duration,
        events=args.events,
    )

    rendered = json.dumps(report, indent=2)
    if args.output:
        with open(args.output, "w", encoding="utf-8") as f:
            f.write(rendered + "\n")
    print(rendered)

    sys.exit(0 if report.get("verdict") == "passed" else 1)


if __name__ == "__main__":
    main()
