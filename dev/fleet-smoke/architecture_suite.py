import argparse
import concurrent.futures
import contextlib
import hashlib
import http.server
import json
import os
import platform
import random
import secrets
import shutil
import subprocess
import sys
import tempfile
import threading
import time
from typing import Any, Dict, List, Optional, Tuple

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from native_context_probe import pipe_ready
from perf_driver import collect_environment, compute_sha256, inspect_source_state, sanitize_for_json
from perf_delivery import _BoundedCapture, decode_fields, parse_daemon_diagnostics, request_graceful_shutdown, start_bounded_capture

MAX_BODY_BYTES = 2 * 1024 * 1024
MAX_COLLECTOR_ITEMS = 1000
MAX_COLLECTOR_ERRORS = 32
SCENARIO_TIMEOUT_SECONDS = 20.0
CAMPAIGN_DEADLINE = float("inf")


def parse_any_value(data: bytes) -> Any:
    for fn, wt, val in decode_fields(data):
        if fn == 1 and wt == 2:
            return val.decode("utf-8", errors="replace")
        if fn == 2 and wt == 0:
            return bool(val)
        if fn == 3 and wt == 0:
            return val
    return None


def parse_key_value(data: bytes) -> Tuple[str, Any]:
    key, val = "", None
    for fn, wt, v in decode_fields(data):
        if fn == 1 and wt == 2:
            key = v.decode("utf-8", errors="replace")
        elif fn == 2 and wt == 2:
            val = parse_any_value(v)
    return key, val


def extract_spans_with_attrs(body: bytes) -> List[Dict[str, Any]]:
    if not body:
        raise ValueError("empty protobuf body")
    spans = []
    for fn1, wt1, v1 in decode_fields(body):
        if fn1 == 1 and wt1 == 2:
            for fn2, wt2, v2 in decode_fields(v1):
                if fn2 == 2 and wt2 == 2:
                    for fn3, wt3, v3 in decode_fields(v2):
                        if fn3 == 2 and wt3 == 2:
                            t_id, s_id, p_id, name, attrs = "", "", None, "", {}
                            for sfn, swt, sval in decode_fields(v3):
                                if sfn == 1 and swt == 2:
                                    t_id = sval.hex()
                                elif sfn == 2 and swt == 2:
                                    s_id = sval.hex()
                                elif sfn == 4 and swt == 2:
                                    p_id = sval.hex()
                                elif sfn == 5 and swt == 2:
                                    name = sval.decode("utf-8", errors="replace")
                                elif sfn == 9 and swt == 2:
                                    k, val = parse_key_value(sval)
                                    if k:
                                        attrs[k] = val
                            if (len(t_id) == 32 and len(s_id) == 16 and int(t_id, 16) != 0
                                    and int(s_id, 16) != 0 and (p_id is None or len(p_id) == 16)):
                                spans.append({
                                    "trace_id": t_id,
                                    "span_id": s_id,
                                    "parent_span_id": p_id,
                                    "name": name,
                                    "attributes": attrs,
                                })
                            else:
                                raise ValueError(f"malformed span identity: trace={t_id} span={s_id}")
    if not spans:
        raise ValueError("protobuf body contains no spans")
    return spans


def _collector_error(server: "MockOTLPServer", message: str) -> None:
    if len(server.collector_errors) < MAX_COLLECTOR_ERRORS:
        server.collector_errors.append(message)


class MockOTLPHandler(http.server.BaseHTTPRequestHandler):
    def setup(self):
        super().setup()
        self.connection.settimeout(3.0)

    def log_message(self, *args):
        pass

    def do_POST(self):
        if self.path not in ("/v1/traces", "/v1/metrics"):
            self.send_error(404, "Unknown endpoint")
            return
        try:
            clen = int(self.headers.get("Content-Length", 0))
        except ValueError:
            self.send_error(400, "Bad Content-Length")
            return
        if not 0 <= clen <= MAX_BODY_BYTES:
            self.send_error(413, "Body size exceeded")
            return
        body = self.rfile.read(clen)
        if len(body) != clen:
            self.send_error(400, "Truncated body")
            return

        if self.path == "/v1/metrics":
            with self.server.lock:
                if len(self.server.received_metrics) < MAX_COLLECTOR_ITEMS:
                    self.server.received_metrics.append({"time": time.monotonic(), "bytes": len(body)})
                else:
                    _collector_error(self.server, "metrics collector capacity exceeded")
            self.send_response(200)
            self.send_header("Content-Type", "application/x-protobuf")
            self.end_headers()
            self.wfile.write(b"")
            return

        with self.server.lock:
            if not self.server.trace_entered.is_set():
                self.server.trace_entered_time = time.monotonic()
                self.server.trace_entered.set()
        gate_ok = self.server.trace_gate.wait(timeout=10.0)
        arrival = time.monotonic()

        try:
            parsed = extract_spans_with_attrs(body)
        except Exception as exc:
            with self.server.lock:
                _collector_error(self.server, f"Protobuf decode error: {exc}")
            self.send_error(400, f"Malformed Protobuf: {exc}")
            return

        with self.server.lock:
            for s in parsed:
                if len(self.server.received_spans) < MAX_COLLECTOR_ITEMS:
                    s["arrival_time"] = arrival
                    s["gate_held"] = not gate_ok
                    self.server.received_spans.append(s)
                else:
                    _collector_error(self.server, "span collector capacity exceeded")

        self.send_response(200)
        self.send_header("Content-Type", "application/x-protobuf")
        self.end_headers()
        self.wfile.write(b"")


class MockOTLPServer(http.server.ThreadingHTTPServer):
    def __init__(self, addr):
        super().__init__(addr, MockOTLPHandler)
        self.received_spans: List[Dict[str, Any]] = []
        self.received_metrics: List[Dict[str, Any]] = []
        self.collector_errors: List[str] = []
        self.trace_gate = threading.Event()
        self.trace_gate.set()
        self.trace_entered = threading.Event()
        self.trace_entered_time: float = 0.0
        self.lock = threading.Lock()


def make_git_workspace(branch: str) -> str:
    ws = tempfile.mkdtemp(prefix="agy_arch_")
    git_dir = os.path.join(ws, ".git")
    os.makedirs(os.path.join(git_dir, "refs", "heads"), exist_ok=True)
    with open(os.path.join(git_dir, "HEAD"), "w", encoding="utf-8") as f:
        f.write(f"ref: refs/heads/{branch}\n")
    commit = hashlib.sha1(branch.encode()).hexdigest()
    with open(os.path.join(git_dir, "refs", "heads", branch), "w", encoding="utf-8") as f:
        f.write(commit + "\n")
    return ws


def mutate_git_head(ws: str, new_branch: str) -> None:
    git_dir = os.path.join(ws, ".git")
    os.makedirs(os.path.join(git_dir, "refs", "heads"), exist_ok=True)
    with open(os.path.join(git_dir, "HEAD"), "w", encoding="utf-8") as f:
        f.write(f"ref: refs/heads/{new_branch}\n")
    commit = hashlib.sha1(new_branch.encode()).hexdigest()
    with open(os.path.join(git_dir, "refs", "heads", new_branch), "w", encoding="utf-8") as f:
        f.write(commit + "\n")


def execute_hook(hook_bin: str, pipe_name: str, idx: int, trace_id: str, span_id: str, cwd: str, deadline: float = float("inf")) -> Dict[str, Any]:
    t_start = time.monotonic()
    if t_start >= deadline:
        return {"index": idx, "trace_id": trace_id, "span_id": span_id, "send_start": t_start,
                "hook_end": t_start, "duration_ms": 0.0, "exit_code": None, "timed_out": True,
                "response_ok": False, "stderr": "scenario deadline exceeded before hook spawn"}
    env = os.environ.copy()
    env.pop("AGENT_OTEL_BENCH_HANDLE", None)
    env.update({
        "AGENT_OTEL_PIPE": pipe_name,
        "AGY_OTEL_PIPE": pipe_name,
        "AGENT_OTEL_SOCKET": pipe_name,
        "TRACEPARENT": f"00-{trace_id}-{span_id}-01",
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
    timed_out = False
    stdout, stderr = b"", b""
    try:
        timeout = max(0.001, min(5.0, deadline - time.monotonic()))
        stdout, stderr = proc.communicate(input=payload, timeout=timeout)
    except subprocess.TimeoutExpired:
        timed_out = True
        proc.kill()
        stdout, stderr = proc.communicate()
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
        "index": idx, "trace_id": trace_id, "span_id": span_id,
        "send_start": t_start, "hook_end": t_end, "duration_ms": (t_end - t_start) * 1000.0,
        "exit_code": proc.returncode, "timed_out": timed_out,
        "response_ok": (proc.returncode == 0 and json_ok and not timed_out),
        "stderr": stderr.decode("utf-8", errors="replace"),
    }


class CleanupState(dict):
    def __init__(self):
        super().__init__()
        self.results: List[Dict[str, Any]] = []


class ScenarioSession:
    def __init__(self, col: MockOTLPServer, pipe: str, proc: Optional[subprocess.Popen], cap: Optional[_BoundedCapture], readers: List[threading.Thread], port: int, deadline: float):
        self.col = col
        self.pipe = pipe
        self.proc = proc
        self.cap = cap
        self.readers = readers
        self.port = port
        self.deadline = deadline
        self.fixtures: List[str] = []
        self.cleanup_info = CleanupState()

    def until(self, seconds: float) -> float:
        return min(self.deadline, CAMPAIGN_DEADLINE, time.monotonic() + seconds)

    def hook(self, hook_bin: str, idx: int, trace_id: str, span_id: str, cwd: str) -> Dict[str, Any]:
        return execute_hook(hook_bin, self.pipe, idx, trace_id, span_id, cwd, self.deadline)

    def new_fixture(self, branch: str) -> str:
        ws = make_git_workspace(branch)
        self.fixtures.append(ws)
        return ws


def _stop_daemon(proc: subprocess.Popen, pipe: str, cap: _BoundedCapture, readers: List[threading.Thread], deadline: float) -> Dict[str, Any]:
    shutdown = request_graceful_shutdown(pipe)
    forced = None
    try:
        proc.wait(timeout=max(0.001, min(7.0, deadline - time.monotonic())))
    except subprocess.TimeoutExpired:
        forced = "terminate"
        proc.terminate()
        try:
            proc.wait(timeout=max(0.001, min(2.0, deadline - time.monotonic())))
        except subprocess.TimeoutExpired:
            forced = "kill"
            proc.kill()
            proc.wait(timeout=2.0)
    for reader in readers:
        reader.join(timeout=max(0.0, min(1.0, deadline - time.monotonic())))
    drain_complete = not any(reader.is_alive() for reader in readers)
    stderr = cap.value("stderr").decode("utf-8", errors="replace")
    diagnostics = parse_daemon_diagnostics(stderr)
    measured = shutdown.get("sent") is True and forced is None and diagnostics is not None and drain_complete and not cap.exceeded.is_set()
    return {"measurement_status": "measured" if measured else "not_measured",
            "not_measured_reason": None if measured else "graceful_shutdown_or_diagnostics_missing",
            "request": shutdown, "exit_code": proc.returncode, "forced": forced,
            "output_bounded": not cap.exceeded.is_set(), "output_drain_complete": drain_complete,
            "diagnostics": diagnostics, "stderr": stderr}


def _finalize_registered_results(sess: ScenarioSession) -> None:
    all_clients = []
    seen_client_ids = set()
    for result in sess.cleanup_info.results:
        for client in result.get("_clients", []):
            if client.get("trace_id") not in seen_client_ids:
                all_clients.append(client)
                seen_client_ids.add(client.get("trace_id"))
    diag = (sess.cleanup_info.get("daemon_shutdown", {}).get("diagnostics") or {})
    pipeline = diag.get("pipeline") if isinstance(diag, dict) else None
    ingress = diag.get("ingress") if isinstance(diag, dict) else None
    expected_total = len(all_clients)
    session_evidence = analyze_evidence(all_clients, list(sess.col.received_spans))
    sess.cleanup_info["session_trace_evidence"] = session_evidence
    reconciliation = ({"expected_events": expected_total, "ingress_admitted": ingress.get("admitted"),
                       "pipeline_transformed": pipeline.get("transformed"), "export_accepted": pipeline.get("accepted")}
                      if isinstance(pipeline, dict) and isinstance(ingress, dict) else None)
    reconciliation_ok = (reconciliation is not None and expected_total > 0
                         and all(reconciliation[name] == expected_total for name in
                                 ("ingress_admitted", "pipeline_transformed", "export_accepted")))
    if reconciliation_ok:
        zero_pipeline = ("invalid", "span_size", "export_capacity", "rejected", "unknown",
                         "shutdown_dropped", "possible_duplicate_batches", "queued_bytes", "queued_items")
        zero_ingress = ("invalid", "capacity", "read_failed", "read_deadline", "reserved_bytes")
        reconciliation_ok = (all(pipeline.get(name) == 0 for name in zero_pipeline)
                             and all(ingress.get(name) == 0 for name in zero_ingress))
    sess.cleanup_info["session_reconciliation"] = reconciliation or "not_measured"
    for result in sess.cleanup_info.results:
        clients = result.pop("_clients")
        expected = {client.get("trace_id") for client in clients}
        spans = [span for span in sess.col.received_spans if span.get("trace_id") in expected]
        result["metrics"] = calculate_metrics(clients, spans)
        result["trace_evidence"] = analyze_evidence(clients, spans)
        result["metrics"]["reconciliation"] = reconciliation or "not_measured"
        evidence_ok = result["trace_evidence"]["valid"]
        cleanup_ok = (session_evidence["valid"]
                      and sess.cleanup_info.get("fixtures_purged") is True
                      and sess.cleanup_info.get("collector_stopped") is True
                      and sess.cleanup_info.get("daemon_shutdown", {}).get("measurement_status") == "measured"
                      and sess.cleanup_info.get("daemon_shutdown", {}).get("exit_code") == 0
                      and reconciliation_ok
                      and not sess.cleanup_info.get("collector_errors"))
        if result["verdict"] == "passed" and (not evidence_ok or not cleanup_ok):
            result["verdict"] = "failed"
            result["assertions"].append({"id": "final_evidence_and_cleanup", "observed": evidence_ok and cleanup_ok,
                                         "expected": True, "status": "failed", "reason": "evidence_or_cleanup_failed"})


@contextlib.contextmanager
def scenario_session(daemon_bin: str, extra_env: Optional[Dict[str, str]] = None, deadline: Optional[float] = None, *, pipe_name: Optional[str] = None):
    deadline = min(CAMPAIGN_DEADLINE, deadline or (time.monotonic() + SCENARIO_TIMEOUT_SECONDS))
    if pipe_name is not None and "aob-round-" not in pipe_name:
        raise ValueError("explicit scenario endpoint must be round-owned")
    pipe = pipe_name or (f"\\\\.\\pipe\\agy-arch-{secrets.token_hex(6)}" if os.name == "nt" else f"/tmp/agy-arch-{secrets.token_hex(6)}.sock")
    col = MockOTLPServer(("127.0.0.1", 0))
    port = col.server_address[1]
    pool = concurrent.futures.ThreadPoolExecutor(max_workers=1)
    pool.submit(col.serve_forever)

    env = os.environ.copy()
    for k in ["TRACEPARENT", "TRACESTATE"]:
        env.pop(k, None)
    env.update({
        "AGENT_OTEL_PIPE": pipe,
        "AGY_OTEL_PIPE": pipe,
        "AGENT_OTEL_SOCKET": pipe,
        "OTEL_EXPORTER_OTLP_ENDPOINT": f"http://127.0.0.1:{port}",
        "OTEL_SERVICE_NAME": "agent-otel-arch-suite",
        "AGENT_OTEL_BATCH_TIMEOUT_MS": "200",
        "AGENT_OTEL_BATCH_SIZE": "50",
        "AGENT_OTEL_IDLE_TIMEOUT_SECS": "30",
        "OTEL_METRIC_EXPORT_INTERVAL": "1000",
    })
    if extra_env:
        env.update(extra_env)
    sess_profile = {
        "batch_size": int(env["AGENT_OTEL_BATCH_SIZE"]),
        "batch_timeout_ms": int(env["AGENT_OTEL_BATCH_TIMEOUT_MS"]),
        "context_fresh_ttl_seconds": 1,
    }

    cf = subprocess.CREATE_NO_WINDOW if platform.system() == "Windows" else 0
    proc = None
    cap = None
    readers: List[threading.Thread] = []
    sess = ScenarioSession(col, pipe, proc, cap, readers, port, deadline)
    sess.cleanup_info["runtime_profile"] = sess_profile

    try:
        try:
            proc = subprocess.Popen([daemon_bin, "daemon"], env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, creationflags=cf, shell=False)
            cap, readers = start_bounded_capture(proc)
            sess.proc, sess.cap, sess.readers = proc, cap, readers
            ready = pipe_ready(pipe, min(deadline, time.monotonic() + 8.0))
        except Exception as exc:
            sess.cleanup_info["spawn_error"] = str(exc)
            ready = False
        yield sess, ready
    finally:
        col.trace_gate.set()
        shut = _stop_daemon(proc, pipe, cap, readers, deadline) if proc is not None and cap is not None else {
            "measurement_status": "not_measured", "not_measured_reason": "daemon_spawn_failed",
            "diagnostics": None, "forced": None, "output_bounded": True, "output_drain_complete": True}
        if proc is not None and proc.poll() is None:
            proc.kill(); proc.wait(timeout=2.0)
        all_purged = True
        for fdir in sess.fixtures:
            shutil.rmtree(fdir, ignore_errors=True)
            if os.path.exists(fdir):
                all_purged = False
        sess.cleanup_info.update({
            "daemon_shutdown": shut,
            "fixtures_purged": all_purged,
            "collector_errors": list(col.collector_errors),
        })
        col.shutdown(); col.server_close(); pool.shutdown(wait=True)
        sess.cleanup_info["collector_stopped"] = True
        _finalize_registered_results(sess)


def get_attr(attrs: Dict[str, Any], *keys: str) -> Any:
    for k in keys:
        if k in attrs:
            return attrs[k]
    return None


def workspace_matches(actual: Any, expected: str) -> bool:
    try:
        return isinstance(actual, str) and os.path.samefile(actual, expected)
    except OSError:
        return False


def calculate_metrics(offered_clients: List[Dict[str, Any]], received_spans: List[Dict[str, Any]]) -> Dict[str, Any]:
    off_cnt = len(offered_clients)
    comp_cnt = sum(1 for c in offered_clients if c.get("response_ok"))
    deliv_cnt = len(received_spans)
    starts = [c["send_start"] for c in offered_clients]
    ends = [c["hook_end"] for c in offered_clients]
    arrivals = [s["arrival_time"] for s in received_spans]

    first_start = min(starts) if starts else 0.0
    last_start = max(starts) if starts else 0.0
    last_end = max(ends) if ends else 0.0
    last_arrival = max(arrivals) if arrivals else 0.0

    off_rate = round(off_cnt / (last_start - first_start), 2) if (last_start - first_start) > 0 else "not_measured"
    comp_rate = round(comp_cnt / (last_end - first_start), 2) if (last_end - first_start) > 0 else "not_measured"
    deliv_rate = round(deliv_cnt / (last_arrival - first_start), 2) if (last_arrival - first_start) > 0 else "not_measured"

    durations = [c["duration_ms"] for c in offered_clients]
    e2e_lats = []
    for s in received_spans:
        m = next((c for c in offered_clients if c["trace_id"] == s["trace_id"]), None)
        if m:
            e2e_lats.append(round((s["arrival_time"] - m["send_start"]) * 1000.0, 2))

    return {
        "offered": off_cnt, "completed": comp_cnt, "delivered": deliv_cnt,
        "offered_rate_eps": off_rate, "completed_rate_eps": comp_rate, "delivered_rate_eps": deliv_rate,
        "external_hook_durations_ms": durations, "e2e_latencies_ms": e2e_lats,
        "ipc_latencies_ms": "not_measured", "rss_bytes": "not_measured", "stage_histograms": "not_measured",
    }


def analyze_evidence(offered_clients: List[Dict[str, Any]], received_spans: List[Dict[str, Any]]) -> Dict[str, Any]:
    exp_ids = [c["trace_id"] for c in offered_clients]
    recv_ids = [s["trace_id"] for s in received_spans]
    missing = [t for t in exp_ids if t not in recv_ids]
    dups = [t for t in set(recv_ids) if recv_ids.count(t) > 1]
    unexpected = [t for t in recv_ids if t not in exp_ids]
    response_failures = [c["trace_id"] for c in offered_clients if not c.get("response_ok")]
    expected_parent = {c["trace_id"]: c.get("span_id") for c in offered_clients}
    wrong_parents = [s.get("trace_id") for s in received_spans
                     if s.get("trace_id") in expected_parent and s.get("parent_span_id") != expected_parent[s.get("trace_id")]]
    valid = bool(exp_ids) and len(set(exp_ids)) == len(exp_ids) and not any(
        (missing, dups, unexpected, response_failures, wrong_parents)
    ) and len(recv_ids) == len(exp_ids)
    return {
        "offered_count": len(exp_ids), "received_count": len(recv_ids),
        "missing_ids": missing, "duplicate_ids": dups, "unexpected_ids": unexpected,
        "response_failure_ids": response_failures, "wrong_parent_ids": wrong_parents, "valid": valid,
        "expected": [{"trace_id": c["trace_id"], "parent_span_id": c.get("span_id"),
                      "response_ok": bool(c.get("response_ok"))} for c in offered_clients],
        "received": [{"trace_id": s.get("trace_id"), "span_id": s.get("span_id"),
                      "parent_span_id": s.get("parent_span_id"), "name": s.get("name"),
                      "attributes": s.get("attributes", {})} for s in received_spans],
    }


def make_scenario_result(s_id: str, verdict: str, assertions: List[Dict[str, Any]], clients: List[Dict[str, Any]], spans: List[Dict[str, Any]], cleanup: Dict[str, Any]) -> Dict[str, Any]:
    metrics = calculate_metrics(clients, spans)
    evidence = analyze_evidence(clients, spans)
    metrics["reconciliation"] = "not_measured"
    result = {
        "id": s_id, "verdict": verdict, "assertions": assertions,
        "metrics": metrics, "trace_evidence": evidence, "cleanup": cleanup,
    }
    if isinstance(cleanup, CleanupState):
        result["_clients"] = clients
        cleanup.results.append(result)
    return result


def run_cold_and_warm(daemon_bin: str, hook_bin: str, rng: random.Random) -> Tuple[Dict[str, Any], Dict[str, Any]]:
    with scenario_session(daemon_bin) as (sess, ready):
        if not ready:
            fail_clean = sess.cleanup_info
            nm = [{"id": "ready", "observed": False, "expected": True, "status": "not_measured", "reason": "pipe_timeout"}]
            return (make_scenario_result("cold_cache", "not_measured", nm, [], [], fail_clean),
                    make_scenario_result("warm_cache", "not_measured", nm, [], [], fail_clean))
        ws = sess.new_fixture("alpha")
        c_tid, c_sid = f"{rng.getrandbits(128):032x}", f"{rng.getrandbits(64):016x}"
        c_res = sess.hook(hook_bin, 0, c_tid, c_sid, ws)
        dl = sess.until(6.0)
        while time.monotonic() < dl and not any(s["trace_id"] == c_tid for s in sess.col.received_spans):
            time.sleep(0.05)

        c_spans = [s for s in sess.col.received_spans if s["trace_id"] == c_tid]
        c_attrs = c_spans[0].get("attributes", {}) if c_spans else {}
        state_c = get_attr(c_attrs, "agent.context.state", "state")
        source_c = get_attr(c_attrs, "agent.context.source", "source")
        branch_c = get_attr(c_attrs, "vcs.branch.name", "branch")
        cold_ok = (len(c_spans) == 1 and c_res["response_ok"] and state_c == "provided" and source_c == "event" and branch_c is None)

        cold_asserts = [
            {"id": "single_span_delivered", "observed": len(c_spans), "expected": 1, "status": "passed" if len(c_spans) == 1 else "failed", "reason": None},
            {"id": "hook_clean_exit", "observed": c_res["response_ok"], "expected": True, "status": "passed" if c_res["response_ok"] else "failed", "reason": None},
            {"id": "state_provided", "observed": str(state_c), "expected": "provided", "status": "passed" if state_c == "provided" else "failed", "reason": None},
            {"id": "source_event", "observed": str(source_c), "expected": "event", "status": "passed" if source_c == "event" else "failed", "reason": None},
            {"id": "branch_absent", "observed": str(branch_c), "expected": "None", "status": "passed" if branch_c is None else "failed", "reason": None},
        ]
        cold_res = make_scenario_result("cold_cache", "passed" if cold_ok else "failed", cold_asserts, [c_res], c_spans, sess.cleanup_info)

        warm_clients = []
        warm_spans = []
        warm_ok = False
        warm_dl = sess.until(10.0)
        p_idx = 1
        last_state, last_branch = None, None
        while time.monotonic() < warm_dl:
            w_tid, w_sid = f"{rng.getrandbits(128):032x}", f"{rng.getrandbits(64):016x}"
            w_res = sess.hook(hook_bin, p_idx, w_tid, w_sid, ws)
            warm_clients.append(w_res)
            p_dl = sess.until(2.0)
            while time.monotonic() < p_dl and not any(s["trace_id"] == w_tid for s in sess.col.received_spans):
                time.sleep(0.04)
            matching = [s for s in sess.col.received_spans if s["trace_id"] == w_tid]
            if matching:
                warm_spans.extend(matching)
                attrs = matching[0].get("attributes", {})
                last_state = get_attr(attrs, "agent.context.state", "state")
                last_branch = get_attr(attrs, "vcs.branch.name", "branch")
                if w_res["response_ok"] and last_state == "fresh" and last_branch == "alpha":
                    warm_ok = True
                    break
            p_idx += 1
            time.sleep(0.1)

        warm_asserts = [
            {"id": "warm_state_fresh", "observed": str(last_state), "expected": "fresh", "status": "passed" if last_state == "fresh" else "failed", "reason": None},
            {"id": "warm_branch_alpha", "observed": str(last_branch), "expected": "alpha", "status": "passed" if last_branch == "alpha" else "failed", "reason": None},
            {"id": "verified_delivery", "observed": warm_ok, "expected": True, "status": "passed" if warm_ok else "failed", "reason": None},
        ]
        warm_res = make_scenario_result("warm_cache", "passed" if warm_ok else "failed", warm_asserts, warm_clients, warm_spans, sess.cleanup_info)
        return cold_res, warm_res


def run_stale_refresh(daemon_bin: str, hook_bin: str, rng: random.Random) -> Dict[str, Any]:
    with scenario_session(daemon_bin) as (sess, ready):
        if not ready:
            return make_scenario_result("stale_refresh", "not_measured", [{"id": "ready", "observed": False, "expected": True, "status": "not_measured", "reason": "pipe_timeout"}], [], [], sess.cleanup_info)
        ws = sess.new_fixture("alpha")
        all_clients = []
        primed = False
        prime_dl = sess.until(8.0)
        p_idx = 0
        while time.monotonic() < prime_dl:
            tid, sid = f"{rng.getrandbits(128):032x}", f"{rng.getrandbits(64):016x}"
            res = sess.hook(hook_bin, p_idx, tid, sid, ws)
            all_clients.append(res)
            dl = sess.until(2.0)
            while time.monotonic() < dl and not any(s["trace_id"] == tid for s in sess.col.received_spans):
                time.sleep(0.04)
            matching = [s for s in sess.col.received_spans if s["trace_id"] == tid]
            if matching and res["response_ok"]:
                attrs = matching[0].get("attributes", {})
                if get_attr(attrs, "agent.context.state", "state") == "fresh" and get_attr(attrs, "vcs.branch.name", "branch") == "alpha":
                    primed = True
                    break
            p_idx += 1
            time.sleep(0.08)

        if not primed:
            return make_scenario_result("stale_refresh", "failed", [{"id": "prime_fresh_alpha", "observed": False, "expected": True, "status": "failed", "reason": "could_not_prime_alpha"}], all_clients, sess.col.received_spans, sess.cleanup_info)

        mutate_git_head(ws, "beta")
        time.sleep(max(0.0, min(1.1, sess.deadline - time.monotonic())))

        p_idx += 1
        s_tid, s_sid = f"{rng.getrandbits(128):032x}", f"{rng.getrandbits(64):016x}"
        s_res = sess.hook(hook_bin, p_idx, s_tid, s_sid, ws)
        all_clients.append(s_res)
        dl = sess.until(2.5)
        while time.monotonic() < dl and not any(s["trace_id"] == s_tid for s in sess.col.received_spans):
            time.sleep(0.04)
        s_spans = [s for s in sess.col.received_spans if s["trace_id"] == s_tid]
        s_attrs = s_spans[0].get("attributes", {}) if s_spans else {}
        s_state = get_attr(s_attrs, "agent.context.state", "state")
        s_branch = get_attr(s_attrs, "vcs.branch.name", "branch")
        stale_ok = (len(s_spans) == 1 and s_res["response_ok"] and s_state == "stale" and s_branch == "alpha")

        fresh_beta_ok = False
        f_state, f_branch = None, None
        post_dl = sess.until(6.0)
        while time.monotonic() < post_dl:
            p_idx += 1
            b_tid, b_sid = f"{rng.getrandbits(128):032x}", f"{rng.getrandbits(64):016x}"
            b_res = sess.hook(hook_bin, p_idx, b_tid, b_sid, ws)
            all_clients.append(b_res)
            dl = sess.until(2.0)
            while time.monotonic() < dl and not any(s["trace_id"] == b_tid for s in sess.col.received_spans):
                time.sleep(0.04)
            b_spans = [s for s in sess.col.received_spans if s["trace_id"] == b_tid]
            if b_spans and b_res["response_ok"]:
                attrs = b_spans[0].get("attributes", {})
                f_state = get_attr(attrs, "agent.context.state", "state")
                f_branch = get_attr(attrs, "vcs.branch.name", "branch")
                if f_state == "fresh" and f_branch == "beta":
                    fresh_beta_ok = True
                    break
            time.sleep(0.08)

        pass_all = (stale_ok and fresh_beta_ok and len(sess.col.received_spans) == len(all_clients))
        asserts = [
            {"id": "stale_transition_observed", "observed": f"state={s_state},branch={s_branch}", "expected": "state=stale,branch=alpha", "status": "passed" if stale_ok else "failed", "reason": None},
            {"id": "fresh_beta_observed", "observed": f"state={f_state},branch={f_branch}", "expected": "state=fresh,branch=beta", "status": "passed" if fresh_beta_ok else "failed", "reason": None},
            {"id": "all_probes_delivered_once", "observed": len(sess.col.received_spans), "expected": len(all_clients), "status": "passed" if len(sess.col.received_spans) == len(all_clients) else "failed", "reason": None},
        ]
        return make_scenario_result("stale_refresh", "passed" if pass_all else "failed", asserts, all_clients, sess.col.received_spans, sess.cleanup_info)


def run_multi_workspace(daemon_bin: str, hook_bin: str, rng: random.Random) -> Dict[str, Any]:
    with scenario_session(daemon_bin, {"AGENT_OTEL_BATCH_SIZE": "1"}) as (sess, ready):
        if not ready:
            return make_scenario_result("multi_workspace", "not_measured", [{"id": "ready", "observed": False, "expected": True, "status": "not_measured", "reason": "pipe_timeout"}], [], [], sess.cleanup_info)
        ws_a = sess.new_fixture("alpha")
        ws_b = sess.new_fixture("beta")
        clients = []

        def prime_ws(ws: str, target_branch: str) -> bool:
            dl = sess.until(6.0)
            idx = 0
            while time.monotonic() < dl:
                tid, sid = f"{rng.getrandbits(128):032x}", f"{rng.getrandbits(64):016x}"
                res = sess.hook(hook_bin, idx, tid, sid, ws)
                clients.append(res)
                p_dl = sess.until(2.0)
                while time.monotonic() < p_dl and not any(s["trace_id"] == tid for s in sess.col.received_spans):
                    time.sleep(0.04)
                sp = [s for s in sess.col.received_spans if s["trace_id"] == tid]
                if sp and res["response_ok"]:
                    at = sp[0].get("attributes", {})
                    if get_attr(at, "agent.context.state", "state") == "fresh" and get_attr(at, "vcs.branch.name", "branch") == target_branch:
                        return True
                idx += 1
                time.sleep(0.08)
            return False

        if not (prime_ws(ws_a, "alpha") and prime_ws(ws_b, "beta")):
            return make_scenario_result("multi_workspace", "failed", [{"id": "prime_both", "observed": False, "expected": True, "status": "failed", "reason": "prime_failed"}], clients, sess.col.received_spans, sess.cleanup_info)

        t_a, s_a = f"{rng.getrandbits(128):032x}", f"{rng.getrandbits(64):016x}"
        t_b, s_b = f"{rng.getrandbits(128):032x}", f"{rng.getrandbits(64):016x}"
        r_a = sess.hook(hook_bin, 100, t_a, s_a, ws_a)
        r_b = sess.hook(hook_bin, 101, t_b, s_b, ws_b)
        clients.extend([r_a, r_b])

        dl = sess.until(5.0)
        while time.monotonic() < dl and not (any(s["trace_id"] == t_a for s in sess.col.received_spans) and any(s["trace_id"] == t_b for s in sess.col.received_spans)):
            time.sleep(0.04)

        span_a = next((s for s in sess.col.received_spans if s["trace_id"] == t_a), None)
        span_b = next((s for s in sess.col.received_spans if s["trace_id"] == t_b), None)
        attrs_a = span_a.get("attributes", {}) if span_a else {}
        attrs_b = span_b.get("attributes", {}) if span_b else {}

        branch_a = get_attr(attrs_a, "vcs.branch.name", "branch")
        branch_b = get_attr(attrs_b, "vcs.branch.name", "branch")
        path_a = get_attr(attrs_a, "workspace.path", "agent.workspace.path")
        path_b = get_attr(attrs_b, "workspace.path", "agent.workspace.path")
        state_a = get_attr(attrs_a, "agent.context.state", "state")
        state_b = get_attr(attrs_b, "agent.context.state", "state")

        received_ok = bool(span_a and span_b)
        responses_ok = r_a["response_ok"] and r_b["response_ok"]
        states_ok = state_a == "fresh" and state_b == "fresh"
        branches_ok = branch_a == "alpha" and branch_b == "beta"
        paths_ok = workspace_matches(path_a, ws_a) and workspace_matches(path_b, ws_b)
        iso_ok = received_ok and responses_ok and states_ok and branches_ok and paths_ok

        asserts = [
            {"id": "both_spans_received", "observed": received_ok, "expected": True, "status": "passed" if received_ok else "failed", "reason": None if received_ok else "span_missing"},
            {"id": "both_hook_responses_valid", "observed": responses_ok, "expected": True, "status": "passed" if responses_ok else "failed", "reason": None if responses_ok else "invalid_hook_response"},
            {"id": "both_contexts_fresh", "observed": f"a={state_a},b={state_b}", "expected": "a=fresh,b=fresh", "status": "passed" if states_ok else "failed", "reason": None if states_ok else "fresh_ttl_expired"},
            {"id": "distinct_branches_isolated", "observed": f"a={branch_a},b={branch_b}", "expected": "a=alpha,b=beta", "status": "passed" if branches_ok else "failed", "reason": None if branches_ok else "branch_cross_contamination"},
            {"id": "workspace_paths_accurate", "observed": f"a={path_a},b={path_b}", "expected": f"a={ws_a},b={ws_b}", "status": "passed" if paths_ok else "failed", "reason": None if paths_ok else "workspace_path_mismatch"},
        ]
        return make_scenario_result("multi_workspace", "passed" if iso_ok else "failed", asserts, clients, sess.col.received_spans, sess.cleanup_info)


def run_exporter_blocked(daemon_bin: str, hook_bin: str, rng: random.Random) -> Dict[str, Any]:
    with scenario_session(daemon_bin, {"AGENT_OTEL_BATCH_SIZE": "1", "AGENT_OTEL_BATCH_TIMEOUT_MS": "50"}) as (sess, ready):
        if not ready:
            return make_scenario_result("exporter_blocked", "not_measured", [{"id": "ready", "observed": False, "expected": True, "status": "not_measured", "reason": "pipe_timeout"}], [], [], sess.cleanup_info)
        ws = sess.new_fixture("alpha")
        sess.col.trace_gate.clear()
        tid, sid = f"{rng.getrandbits(128):032x}", f"{rng.getrandbits(64):016x}"
        c_res = sess.hook(hook_bin, 0, tid, sid, ws)

        entered = sess.col.trace_entered.wait(timeout=max(0.0, min(5.0, sess.deadline - time.monotonic())))
        t_enter = sess.col.trace_entered_time
        metrics_while_blocked = False
        m_dl = sess.until(4.0)
        while time.monotonic() < m_dl:
            with sess.col.lock:
                if any(m["time"] >= t_enter for m in sess.col.received_metrics):
                    metrics_while_blocked = True
                    break
            time.sleep(0.05)

        release_start = time.monotonic()
        sess.col.trace_gate.set()
        delivered = False
        d_dl = sess.until(4.0)
        while time.monotonic() < d_dl:
            if any(s["trace_id"] == tid for s in sess.col.received_spans):
                delivered = True
                break
            time.sleep(0.04)
        release_dur = time.monotonic() - release_start

        ok = (entered and metrics_while_blocked and delivered and release_dur <= 2.5 and c_res["response_ok"])
        asserts = [
            {"id": "trace_handler_blocked", "observed": entered, "expected": True, "status": "passed" if entered else "failed", "reason": None},
            {"id": "daemon_metrics_while_traces_held", "observed": metrics_while_blocked, "expected": True, "status": "passed" if metrics_while_blocked else "failed", "reason": None},
            {"id": "trace_delivered_on_gate_release", "observed": delivered, "expected": True, "status": "passed" if delivered else "failed", "reason": None},
            {"id": "drain_within_deadline", "observed": round(release_dur, 2) <= 2.5, "expected": True, "status": "passed" if release_dur <= 2.5 else "failed", "reason": None},
        ]
        return make_scenario_result("exporter_blocked", "passed" if ok else "failed", asserts, [c_res], sess.col.received_spans, sess.cleanup_info)


def run_concurrent_delivery(daemon_bin: str, hook_bin: str, rng: random.Random, events: int, concurrency: int) -> Dict[str, Any]:
    with scenario_session(daemon_bin) as (sess, ready):
        if not ready:
            return make_scenario_result("concurrent_delivery", "not_measured", [{"id": "ready", "observed": False, "expected": True, "status": "not_measured", "reason": "pipe_timeout"}], [], [], sess.cleanup_info)
        ws = sess.new_fixture("alpha")
        plan = [(f"{rng.getrandbits(128):032x}", f"{rng.getrandbits(64):016x}") for _ in range(events)]
        with concurrent.futures.ThreadPoolExecutor(max_workers=concurrency) as ex:
            futs = [ex.submit(sess.hook, hook_bin, i, plan[i][0], plan[i][1], ws) for i in range(events)]
            clients = [f.result() for f in concurrent.futures.as_completed(futs)]

        dl = sess.until(10.0)
        while time.monotonic() < dl and len(sess.col.received_spans) < events:
            time.sleep(0.04)

        ev_res = analyze_evidence(clients, sess.col.received_spans)
        all_comp = all(c.get("response_ok") for c in clients)
        ok = (len(ev_res["missing_ids"]) == 0 and len(ev_res["duplicate_ids"]) == 0 and len(ev_res["unexpected_ids"]) == 0 and len(sess.col.received_spans) == events and all_comp)
        asserts = [
            {"id": "all_events_delivered", "observed": len(sess.col.received_spans), "expected": events, "status": "passed" if len(ev_res["missing_ids"]) == 0 else "failed", "reason": None},
            {"id": "zero_duplicates", "observed": len(ev_res["duplicate_ids"]), "expected": 0, "status": "passed" if len(ev_res["duplicate_ids"]) == 0 else "failed", "reason": None},
            {"id": "all_clients_completed", "observed": sum(1 for c in clients if c.get("response_ok")), "expected": events, "status": "passed" if all_comp else "failed", "reason": None},
        ]
        return make_scenario_result("concurrent_delivery", "passed" if ok else "failed", asserts, clients, sess.col.received_spans, sess.cleanup_info)


def inspect_installed_bridge() -> Dict[str, Any]:
    app_data = os.environ.get("LOCALAPPDATA") or os.path.expanduser("~\\AppData\\Local")
    base = os.path.join(app_data, "agent-otel-bridge")
    active_json = os.path.join(base, "active.json")
    bin_dir = os.path.join(base, "bin")
    active_sha = compute_sha256(active_json) if os.path.isfile(active_json) else None
    bin_hashes = {}
    if os.path.isdir(bin_dir):
        for fname in sorted(os.listdir(bin_dir)):
            fp = os.path.join(bin_dir, fname)
            if os.path.isfile(fp):
                bin_hashes[fname] = compute_sha256(fp)
    return {"active_json_sha256": active_sha, "bin_hashes": bin_hashes}


def _valid_hex(value: Any, length: int) -> bool:
    return isinstance(value, str) and len(value) == length and all(c in "0123456789abcdefABCDEF" for c in value)


def _valid_sha(value: Any) -> bool:
    return _valid_hex(value, 64)


def source_state_available(state: Dict[str, Any]) -> bool:
    return _valid_hex(state.get("candidate_git_revision"), 40) and _valid_sha(state.get("git_diff_head_sha256"))


def installed_bridge_available(state: Dict[str, Any]) -> bool:
    hashes = state.get("bin_hashes")
    return (_valid_sha(state.get("active_json_sha256")) and isinstance(hashes, dict)
            and _valid_sha(hashes.get("agent-hook.exe")) and _valid_sha(hashes.get("agent-otel-bridge.exe")))


def exception_result(scenario_id: str, exc: Exception) -> Dict[str, Any]:
    reason = f"{type(exc).__name__}: {exc}"
    return make_scenario_result(scenario_id, "not_measured" if isinstance(exc, (FileNotFoundError, TimeoutError)) else "failed",
                                [{"id": "scenario_execution", "observed": reason, "expected": "completed",
                                  "status": "not_measured" if isinstance(exc, (FileNotFoundError, TimeoutError)) else "failed",
                                  "reason": reason}], [], [], {"exception": reason})


def main():
    global CAMPAIGN_DEADLINE
    p = argparse.ArgumentParser(description="Agent-OTEL Architecture Acceptance Suite")
    p.add_argument("--daemon-bin", required=True)
    p.add_argument("--hook-bin", required=True)
    p.add_argument("--seed", type=int, default=42)
    p.add_argument("--repeats", type=int, default=3)
    p.add_argument("--events", type=int, default=24)
    p.add_argument("--concurrency", type=int, default=4)
    scenario_ids = ("cold_cache", "warm_cache", "stale_refresh", "multi_workspace", "exporter_blocked", "concurrent_delivery")
    p.add_argument("--scenario", choices=scenario_ids, default=None)
    args = p.parse_args()

    if not (1 <= args.repeats <= 3 and 1 <= args.events <= 100 and 1 <= args.concurrency <= 8):
        sys.exit("Bounds violation: repeats 1..3, events 1..100, concurrency 1..8")

    suite_start = time.monotonic()
    CAMPAIGN_DEADLINE = suite_start + 175.0
    rng = random.Random(args.seed)

    src_before = inspect_source_state()
    daemon_before = compute_sha256(args.daemon_bin)
    hook_before = compute_sha256(args.hook_bin)
    bridge_before = inspect_installed_bridge()

    runners = {
        "stale_refresh": lambda: run_stale_refresh(args.daemon_bin, args.hook_bin, rng),
        "multi_workspace": lambda: run_multi_workspace(args.daemon_bin, args.hook_bin, rng),
        "exporter_blocked": lambda: run_exporter_blocked(args.daemon_bin, args.hook_bin, rng),
        "concurrent_delivery": lambda: run_concurrent_delivery(args.daemon_bin, args.hook_bin, rng, args.events, args.concurrency),
    }

    scenarios_out = []
    for r_idx in range(args.repeats):
        target_keys = [args.scenario] if args.scenario else list(scenario_ids)
        rng.shuffle(target_keys)
        pair_results = None
        for key in target_keys:
            if time.monotonic() >= CAMPAIGN_DEADLINE:
                scenarios_out.append(make_scenario_result(key, "not_measured", [{"id": "deadline", "observed": False, "expected": True, "status": "not_measured", "reason": "campaign_deadline_exceeded"}], [], [], {}))
                continue
            try:
                if key in ("cold_cache", "warm_cache"):
                    if pair_results is None:
                        pair_results = run_cold_and_warm(args.daemon_bin, args.hook_bin, rng)
                    res = pair_results[0 if key == "cold_cache" else 1]
                else:
                    res = runners[key]()
            except Exception as exc:
                res = exception_result(key, exc)
            res["round"] = r_idx
            scenarios_out.append(res)

    src_after = inspect_source_state()
    daemon_after = compute_sha256(args.daemon_bin)
    hook_after = compute_sha256(args.hook_bin)
    bridge_after = inspect_installed_bridge()

    intact_src = source_state_available(src_before) and source_state_available(src_after) and src_before == src_after
    intact_cand = (all(_valid_sha(value) for value in (daemon_before, daemon_after, hook_before, hook_after))
                   and daemon_before == daemon_after and hook_before == hook_after)
    intact_bridge = (installed_bridge_available(bridge_before) and installed_bridge_available(bridge_after)
                     and bridge_before == bridge_after)
    gates_pass = (intact_src and intact_cand and intact_bridge)

    all_passed = len(scenarios_out) > 0 and gates_pass and all(s.get("verdict") == "passed" for s in scenarios_out)
    any_failed = (not gates_pass) or any(s.get("verdict") == "failed" for s in scenarios_out)
    overall_verdict = "passed" if all_passed else ("failed" if any_failed else "not_measured")

    out = {
        "schema": "agent-otel-new-architecture/v1",
        "timestamp": int(time.time()),
        "seed": args.seed,
        "repeats": args.repeats,
        "environment": collect_environment(),
        "candidate_hashes": {
            "daemon": {"path": args.daemon_bin, "sha256": daemon_after},
            "hook": {"path": args.hook_bin, "sha256": hook_after},
        },
        "source_state": {"before": src_before, "after": src_after},
        "installed_bridge": {"before": bridge_before, "after": bridge_after},
        "integrity_gates": {
            "source_intact": intact_src,
            "candidate_binaries_intact": intact_cand,
            "installed_bridge_intact": intact_bridge,
        },
        "campaign_duration_seconds": round(time.monotonic() - suite_start, 2),
        "overall_verdict": overall_verdict,
        "scenarios": scenarios_out,
    }
    sys.stdout.write(json.dumps(sanitize_for_json(out), indent=2) + "\n")
    sys.exit(0 if overall_verdict == "passed" else 1)


if __name__ == "__main__":
    main()
