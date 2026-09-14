import argparse, concurrent.futures, http.server, json, os, platform, secrets, subprocess, sys, time, threading
from typing import Any, Dict, List, Optional, Set, Tuple

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from native_context_probe import pipe_ready
from perf_driver import collect_environment, compute_sha256, inspect_source_state, sanitize_for_json
from hook_timing_probe import RECORD as HOOK_OBSERVER_RECORD, SEND_COMPLETED, TimingRecord

MAX_BODY_BYTES = 2 * 1024 * 1024
MAX_SPANS = 1000
CAMPAIGN_DEADLINE = float("inf")
DAEMON_OUTPUT_LIMIT = 64 * 1024

class _BoundedCapture:
    def __init__(self, limit: int):
        self.remaining = limit
        self.parts = {"stdout": [], "stderr": []}
        self.exceeded = threading.Event()
        self.lock = threading.Lock()

    def add(self, stream: str, chunk: bytes) -> None:
        with self.lock:
            keep = min(len(chunk), self.remaining)
            if keep:
                self.parts[stream].append(chunk[:keep])
                self.remaining -= keep
            if keep != len(chunk):
                self.exceeded.set()

    def value(self, stream: str) -> bytes:
        with self.lock:
            return b"".join(self.parts[stream])

def _read_pipe(pipe, stream: str, capture: _BoundedCapture) -> None:
    try:
        while True:
            chunk = pipe.read(8192)
            if not chunk:
                return
            capture.add(stream, chunk)
    except (OSError, ValueError):
        return

def start_bounded_capture(proc: subprocess.Popen) -> Tuple[_BoundedCapture, List[threading.Thread]]:
    capture = _BoundedCapture(DAEMON_OUTPUT_LIMIT)
    readers = [
        threading.Thread(target=_read_pipe, args=(proc.stdout, "stdout", capture), daemon=True),
        threading.Thread(target=_read_pipe, args=(proc.stderr, "stderr", capture), daemon=True),
    ]
    for reader in readers:
        reader.start()
    return capture, readers

def parse_daemon_diagnostics(stderr: str) -> Optional[Dict[str, Any]]:
    """Return the daemon's bounded final fact summary, if it reached graceful shutdown."""
    for line in reversed(stderr.splitlines()):
        try:
            value = json.loads(line)
        except (TypeError, ValueError):
            continue
        if isinstance(value, dict) and value.get("kind") == "bridge_diagnostics":
            return value
    return None

def classify_observer(raw: bytes, expected_pid: int, response_ok: bool, exit_code: int) -> Dict[str, Any]:
    if not raw:
        inferred = response_ok and exit_code == 0
        return {
            "status": "missing_record",
            "record_bytes": 0,
            "raw_hex": "",
            "send_completed": False,
            "watchdog_inferred": inferred,
            "inference": "missing_record_with_valid_fail_open_exit" if inferred else None,
            "error": "observer record missing",
        }
    try:
        record = TimingRecord.decode(raw, expected_pid)
    except ValueError as exc:
        return {
            "status": "invalid_record",
            "record_bytes": len(raw),
            "raw_hex": raw.hex(),
            "send_completed": False,
            "watchdog_inferred": False,
            "inference": None,
            "error": str(exc),
        }
    completed = bool(record.flags & SEND_COMPLETED)
    return {
        "status": "send_completed" if completed else "send_not_completed",
        "record_bytes": len(raw),
        "raw_hex": raw.hex(),
        "send_completed": completed,
        "watchdog_inferred": False,
        "inference": None,
        "error": None,
        "response_completed_ns": record.response_completed_ns,
        "before_transport_ns": record.before_transport_ns,
        "work_completed_ns": record.work_completed_ns,
    }

def request_graceful_shutdown(pipe_name: str) -> Dict[str, Any]:
    frame = b"AG\x01\xff\x00\x00\x00\x00"
    try:
        if os.name == "nt":
            import ctypes
            from ctypes import wintypes
            kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
            kernel32.WaitNamedPipeW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD]
            kernel32.WaitNamedPipeW.restype = wintypes.BOOL
            kernel32.CreateFileW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD, wintypes.LPVOID, wintypes.DWORD, wintypes.DWORD, wintypes.HANDLE]
            kernel32.CreateFileW.restype = wintypes.HANDLE
            kernel32.WriteFile.argtypes = [wintypes.HANDLE, wintypes.LPCVOID, wintypes.DWORD, wintypes.LPDWORD, wintypes.LPVOID]
            kernel32.WriteFile.restype = wintypes.BOOL
            kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
            kernel32.CloseHandle.restype = wintypes.BOOL
            if not kernel32.WaitNamedPipeW(pipe_name, 1000):
                return {"sent": False, "error": f"WaitNamedPipeW:{ctypes.get_last_error()}"}
            handle = kernel32.CreateFileW(pipe_name, 0x40000000, 0, None, 3, 0, None)
            if handle == ctypes.c_void_p(-1).value:
                return {"sent": False, "error": f"CreateFileW:{ctypes.get_last_error()}"}
            try:
                written = wintypes.DWORD()
                buffer = ctypes.create_string_buffer(frame)
                ok = kernel32.WriteFile(handle, buffer, len(frame), ctypes.byref(written), None)
                if not ok or written.value != len(frame):
                    return {"sent": False, "error": f"WriteFile:{ctypes.get_last_error()}", "bytes": written.value}
            finally:
                kernel32.CloseHandle(handle)
        else:
            import socket
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as sock:
                sock.settimeout(1.0)
                sock.connect(pipe_name)
                sock.sendall(frame)
        return {"sent": True, "error": None, "bytes": len(frame)}
    except Exception as exc:
        return {"sent": False, "error": str(exc)}

def stop_daemon_gracefully(proc: subprocess.Popen, pipe_name: str, capture: _BoundedCapture, readers: List[threading.Thread]) -> Dict[str, Any]:
    shutdown = request_graceful_shutdown(pipe_name)
    forced = None
    try:
        proc.wait(timeout=7.0)
    except subprocess.TimeoutExpired:
        forced = "terminate"
        proc.terminate()
        try:
            proc.wait(timeout=2.0)
        except subprocess.TimeoutExpired:
            forced = "kill"
            proc.kill()
            proc.wait(timeout=2.0)
    for reader in readers:
        reader.join(timeout=1.0)
    drain_complete = not any(reader.is_alive() for reader in readers)
    for reader, pipe in zip(readers, (proc.stdout, proc.stderr)):
        if not reader.is_alive():
            pipe.close()
    stderr_text = capture.value("stderr").decode("utf-8", errors="replace")
    diagnostics = parse_daemon_diagnostics(stderr_text)
    measured = shutdown.get("sent") is True and forced is None and diagnostics is not None and drain_complete and not capture.exceeded.is_set()
    return {
        "measurement_status": "measured" if measured else "not_measured",
        "not_measured_reason": None if measured else "graceful_shutdown_or_diagnostics_missing",
        "request": shutdown,
        "exit_code": proc.returncode,
        "forced": forced,
        "output_bounded": not capture.exceeded.is_set(),
        "output_drain_complete": drain_complete,
        "diagnostics": diagnostics,
        "stderr": stderr_text,
    }

def parse_varint(data: bytes, offset: int) -> Tuple[int, int]:
    res, shift = 0, 0
    while offset < len(data):
        b = data[offset]; offset += 1
        res |= (b & 0x7F) << shift
        if not (b & 0x80): return res, offset
        shift += 7
        if shift >= 64: raise ValueError("varint overflow")
    raise ValueError("truncated varint")

def decode_fields(data: bytes) -> List[Tuple[int, int, Any]]:
    offset, fields = 0, []
    while offset < len(data):
        tag, offset = parse_varint(data, offset)
        fn, wt = tag >> 3, tag & 7
        if fn == 0: raise ValueError("invalid protobuf field zero")
        if wt == 0: val, offset = parse_varint(data, offset)
        elif wt == 1:
            if offset + 8 > len(data): raise ValueError("truncated 64-bit")
            val, offset = data[offset:offset+8], offset + 8
        elif wt == 2:
            length, offset = parse_varint(data, offset)
            if offset + length > len(data): raise ValueError("truncated length-delimited")
            val, offset = data[offset:offset+length], offset + length
        elif wt == 5:
            if offset + 4 > len(data): raise ValueError("truncated 32-bit")
            val, offset = data[offset:offset+4], offset + 4
        else: raise ValueError(f"unsupported wire type {wt}")
        fields.append((fn, wt, val))
    return fields

def extract_trace_spans(body: bytes) -> List[Tuple[str, str]]:
    results = []
    for fn1, wt1, v1 in decode_fields(body):
        if fn1 == 1 and wt1 == 2:
            for fn2, wt2, v2 in decode_fields(v1):
                if fn2 == 2 and wt2 == 2:
                    for fn3, wt3, v3 in decode_fields(v2):
                        if fn3 == 2 and wt3 == 2:
                            t_id, s_id = "", ""
                            for sfn, swt, sval in decode_fields(v3):
                                if sfn == 1 and swt == 2: t_id = sval.hex()
                                elif sfn == 2 and swt == 2: s_id = sval.hex()
                            if len(t_id) != 32 or len(s_id) != 16 or int(t_id, 16) == 0 or int(s_id, 16) == 0:
                                raise ValueError("invalid span identity")
                            results.append((t_id, s_id))
    return results

class TraceCollector(http.server.ThreadingHTTPServer):
    def __init__(self, addr):
        super().__init__(addr, TraceHandler)
        self.received_spans: List[Tuple[str, str]] = []
        self.collector_errors: List[str] = []
        self.lock = threading.Lock()
        self.stopped = False

class TraceHandler(http.server.BaseHTTPRequestHandler):
    def setup(self):
        super().setup()
        self.connection.settimeout(2.0)
    def log_message(self, format, *args): pass
    def do_POST(self):
        try: clen = int(self.headers.get("Content-Length", 0))
        except ValueError: self.send_error(400); return
        if not 0 <= clen <= MAX_BODY_BYTES:
            self.send_error(413); return
        body = self.rfile.read(clen)
        if len(body) != clen:
            self.server.collector_errors.append("truncated_http_body")
            self.send_error(400); return
        if self.path.startswith("/v1/traces"):
            try:
                parsed = extract_trace_spans(body)
                with self.server.lock:
                    if len(self.server.received_spans) + len(parsed) <= MAX_SPANS:
                        self.server.received_spans.extend(parsed)
                    else: self.server.collector_errors.append("max_spans_exceeded")
            except Exception as exc: self.server.collector_errors.append(f"decode_error:{exc}")
        self.send_response(200); self.send_header("Content-Type", "application/x-protobuf")
        self.end_headers()

def _observed_process(command: List[str], payload: bytes, env: Dict[str, str], preload_stdin: bool) -> Tuple[subprocess.Popen, bytes, bytes, bytes]:
    if os.name != "nt":
        raise RuntimeError("hook observer is Windows-only")
    import msvcrt

    observer_read, observer_write = os.pipe()
    os.set_inheritable(observer_write, True)
    observer_handle = msvcrt.get_osfhandle(observer_write)
    startupinfo = subprocess.STARTUPINFO()
    startupinfo.lpAttributeList = {"handle_list": [observer_handle]}
    env["AGENT_OTEL_BENCH_HANDLE"] = str(observer_handle)
    stdin_file = None
    try:
        if preload_stdin:
            stdin_read, stdin_write = os.pipe()
            os.write(stdin_write, payload)
            os.close(stdin_write)
            stdin_file = os.fdopen(stdin_read, "rb", closefd=True)
        child = subprocess.Popen(
            command,
            stdin=stdin_file if stdin_file is not None else subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=env,
            startupinfo=startupinfo,
            close_fds=True,
            shell=False,
        )
    except Exception:
        os.close(observer_read)
        raise
    finally:
        os.close(observer_write)
    try:
        stdout, stderr = child.communicate(None if preload_stdin else payload, timeout=3.0)
    except subprocess.TimeoutExpired:
        child.kill()
        stdout, stderr = child.communicate()
    finally:
        if stdin_file is not None:
            stdin_file.close()
    chunks = []
    while True:
        chunk = os.read(observer_read, HOOK_OBSERVER_RECORD.size + 1)
        if not chunk:
            break
        chunks.append(chunk)
        if sum(len(part) for part in chunks) > HOOK_OBSERVER_RECORD.size:
            break
    os.close(observer_read)
    return child, stdout, stderr, b"".join(chunks)

def run_single_client(hook_bin: str, pipe_name: str, ev_idx: int, t_id: str, s_id: str, observe_hook: bool = False, preload_stdin: bool = False) -> Dict[str, Any]:
    if time.monotonic() >= CAMPAIGN_DEADLINE:
        return {"event_idx": ev_idx, "trace_id": t_id, "error": "campaign_deadline", "response_ok": False}
    t_parent = f"00-{t_id}-{s_id}-01"
    payload = json.dumps({"conversationId": f"delivery-conv-{ev_idx}", "toolCall": {"name": "performance_delivery", "id": f"call-{ev_idx}"}, "stepIdx": ev_idx})
    env = os.environ.copy(); env.update({"AGENT_OTEL_PIPE": pipe_name, "AGY_OTEL_PIPE": pipe_name, "AGENT_OTEL_SOCKET": pipe_name, "TRACEPARENT": t_parent})
    t0 = time.monotonic()
    try:
        if observe_hook:
            child, stdout, stderr, raw_record = _observed_process(
                [hook_bin, "--client", "codex", "PostToolUse"], payload.encode(), env, preload_stdin
            )
            dur_ms = (time.monotonic() - t0) * 1000.0
            response_ok = child.returncode == 0 and stdout.strip() == b"{}"
            observer = classify_observer(raw_record, child.pid, response_ok, child.returncode)
            return {"event_idx": ev_idx, "trace_id": t_id, "exit_code": child.returncode, "response_ok": response_ok, "duration_ms": dur_ms, "duration_boundary": "external_process_not_internal_hook", "stdin_mode": "preloaded_pipe_before_start" if preload_stdin else "communicate_after_start", "observer": observer, "stderr": stderr.decode("utf-8", errors="replace"), "error": None if response_ok else "invalid_hook_response"}
        if preload_stdin:
            stdin_read, stdin_write = os.pipe()
            os.write(stdin_write, payload.encode())
            os.close(stdin_write)
            with os.fdopen(stdin_read, "rb", closefd=True) as child_stdin:
                proc = subprocess.run([hook_bin, "--client", "codex", "PostToolUse"], stdin=child_stdin, capture_output=True, timeout=3.0, env=env, shell=False)
            dur_ms = (time.monotonic() - t0) * 1000.0
            response_ok = proc.returncode == 0 and proc.stdout.strip() == b"{}"
            return {"event_idx": ev_idx, "trace_id": t_id, "exit_code": proc.returncode, "response_ok": response_ok, "duration_ms": dur_ms, "duration_boundary": "external_process_not_internal_hook", "stdin_mode": "preloaded_pipe_before_start", "error": None if response_ok else "invalid_hook_response"}
        proc = subprocess.run([hook_bin, "--client", "codex", "PostToolUse"], input=payload, text=True, capture_output=True, timeout=3.0, env=env, shell=False)
        dur_ms = (time.monotonic() - t0) * 1000.0
        response_ok = proc.returncode == 0 and proc.stdout.strip() == "{}"
        return {"event_idx": ev_idx, "trace_id": t_id, "exit_code": proc.returncode, "response_ok": response_ok, "duration_ms": dur_ms, "duration_boundary": "external_process_not_internal_hook", "stdin_mode": "communicate_after_start", "error": None if response_ok else "invalid_hook_response"}
    except Exception as exc: return {"event_idx": ev_idx, "trace_id": t_id, "exit_code": -1, "duration_ms": (time.monotonic() - t0) * 1000.0, "error": str(exc)}

def evaluate_delivery(expected_ids: List[str], received_tuples: List[Tuple[str, str]]) -> Dict[str, Any]:
    if not expected_ids: return {"verdict": "failed", "reason": "empty_expected_ids"}
    if len(set(expected_ids)) != len(expected_ids): return {"verdict": "failed", "reason": "duplicate_expected_ids"}
    rcv_ids = [t[0] for t in received_tuples]
    exp_set, rcv_set = set(expected_ids), set(rcv_ids)
    missing = sorted(list(exp_set - rcv_set))
    dup = sorted(list({x for x in rcv_ids if rcv_ids.count(x) > 1}))
    unexpected = sorted(list(rcv_set - exp_set))
    verdict = "passed" if (len(missing) == 0 and len(dup) == 0 and len(unexpected) == 0) else "failed"
    return {"verdict": verdict, "expected_count": len(expected_ids), "received_count": len(rcv_ids), "expected_ids": expected_ids, "received_spans": received_tuples, "missing_ids": missing, "duplicate_ids": dup, "unexpected_ids": unexpected, "normative_ref": "dev/fleet-smoke/performance-coordination.md#required-measurements-and-assertions", "implementation_ref": "crates/agent-otel-daemon/src/daemon.rs", "backend_visibility": "not_checked_local_capture_only"}

def summarize_observers(clients: List[Dict[str, Any]]) -> Optional[Dict[str, int]]:
    observed = [client["observer"] for client in clients if "observer" in client]
    if not observed:
        return None
    return {
        "clients": len(observed),
        "send_completed": sum(item["send_completed"] for item in observed),
        "send_not_completed": sum(item["status"] == "send_not_completed" for item in observed),
        "missing_record": sum(item["status"] == "missing_record" for item in observed),
        "invalid_record": sum(item["status"] == "invalid_record" for item in observed),
        "watchdog_inferred": sum(item["watchdog_inferred"] for item in observed),
    }

def correlate_missing_clients(missing_ids: List[str], clients: List[Dict[str, Any]]) -> List[Dict[str, Any]]:
    by_trace = {client.get("trace_id"): client for client in clients}
    result = []
    for trace_id in missing_ids:
        client = by_trace.get(trace_id, {})
        observer = client.get("observer") or {}
        result.append({
            "trace_id": trace_id,
            "event_idx": client.get("event_idx"),
            "client_response_ok": client.get("response_ok"),
            "observer_status": observer.get("status", "not_observed"),
            "send_completed": observer.get("send_completed"),
            "watchdog_inferred": observer.get("watchdog_inferred"),
        })
    return result

def run_scenario(daemon_bin: str, hook_bin: str, repeats: int, events: int, concurrency: int, observe_hook: bool = False, preload_stdin: bool = False) -> Dict[str, Any]:
    runs = []
    for r_idx in range(repeats):
        if time.monotonic() >= CAMPAIGN_DEADLINE:
            runs.append({"verdict": "not_measured", "reason": "campaign_deadline"})
            break
        pipe_name = f"\\\\.\\pipe\\agent-otel-test-{secrets.token_hex(6)}" if platform.system() == "Windows" else f"/tmp/agent-otel-test-{secrets.token_hex(6)}.sock"
        col = TraceCollector(("127.0.0.1", 0))
        port = col.server_address[1]
        t_thread = concurrent.futures.ThreadPoolExecutor(max_workers=1); t_thread.submit(col.serve_forever)
        d_env = os.environ.copy()
        for k in ["TRACEPARENT", "TRACESTATE"]: d_env.pop(k, None)
        d_env.update({"AGENT_OTEL_PIPE": pipe_name, "AGY_OTEL_PIPE": pipe_name, "AGENT_OTEL_SOCKET": pipe_name, "OTEL_EXPORTER_OTLP_ENDPOINT": f"http://127.0.0.1:{port}", "OTEL_SERVICE_NAME": f"probe-delivery-{r_idx}", "AGENT_OTEL_IDLE_TIMEOUT_SECS": "30"})
        cf = subprocess.CREATE_NO_WINDOW if platform.system() == "Windows" else 0
        try:
            daemon_proc = subprocess.Popen([daemon_bin, "daemon"], env=d_env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, creationflags=cf, shell=False)
            daemon_capture, daemon_readers = start_bounded_capture(daemon_proc)
        except Exception as exc:
            col.shutdown(); col.server_close(); t_thread.shutdown(wait=True)
            return {"verdict": "not_measured", "reason": f"daemon_spawn_failed: {exc}", "runs": []}
        try:
            if not pipe_ready(pipe_name, min(CAMPAIGN_DEADLINE, time.monotonic() + 10.0)):
                runs.append({"run_index": r_idx, "verdict": "not_measured", "reason": "pipe_readiness_timeout"}); break
            ev_plan = [(secrets.token_hex(16), secrets.token_hex(8)) for _ in range(events)]
            exp_trace_ids = [t[0] for t in ev_plan]
            with concurrent.futures.ThreadPoolExecutor(max_workers=concurrency) as ex:
                futs = [ex.submit(run_single_client, hook_bin, pipe_name, idx, ev_plan[idx][0], ev_plan[idx][1], observe_hook, preload_stdin) for idx in range(events)]
                c_res = [f.result() for f in concurrent.futures.as_completed(futs)]
            deadline = min(CAMPAIGN_DEADLINE, time.monotonic() + 10.0)
            while time.monotonic() < deadline and len(col.received_spans) < events: time.sleep(0.05)
            time.sleep(0.15)
            daemon_result = stop_daemon_gracefully(daemon_proc, pipe_name, daemon_capture, daemon_readers)
            eval_res = evaluate_delivery(exp_trace_ids, col.received_spans)
            if col.collector_errors or any(not c.get("response_ok") for c in c_res):
                eval_res["verdict"] = "failed"
            runs.append({"run_index": r_idx, "eval": eval_res, "clients": c_res, "missing_client_evidence": correlate_missing_clients(eval_res.get("missing_ids", []), c_res), "hook_observer_summary": summarize_observers(c_res), "daemon_shutdown": daemon_result, "collector_errors": col.collector_errors})
        finally:
            if daemon_proc.poll() is None:
                daemon_proc.terminate()
                try: daemon_proc.wait(timeout=2.0)
                except subprocess.TimeoutExpired: daemon_proc.kill(); daemon_proc.wait(timeout=2.0)
                for reader in daemon_readers: reader.join(timeout=1.0)
                for reader, pipe in zip(daemon_readers, (daemon_proc.stdout, daemon_proc.stderr)):
                    if not reader.is_alive(): pipe.close()
            col.shutdown(); col.server_close(); t_thread.shutdown(wait=True)
            if platform.system() != "Windows" and os.path.exists(pipe_name):
                try: os.unlink(pipe_name)
                except OSError: pass
    all_passed = len(runs) == repeats and all(r.get("eval", {}).get("verdict") == "passed" for r in runs)
    any_failed = any(r.get("eval", {}).get("verdict") == "failed" for r in runs)
    return {"verdict": "passed" if all_passed else ("failed" if any_failed else "not_measured"), "daemon": {"path": daemon_bin, "sha256": compute_sha256(daemon_bin)}, "events": events, "concurrency": concurrency, "repeats": repeats, "flush_deadline_seconds": 10, "observe_hook": observe_hook, "stdin_mode": "preloaded_pipe_before_start" if preload_stdin else "communicate_after_start", "runs": runs}

def main():
    global CAMPAIGN_DEADLINE
    CAMPAIGN_DEADLINE = time.monotonic() + 165.0
    p = argparse.ArgumentParser(description="Concurrent event delivery probe")
    p.add_argument("--hook", required=True); p.add_argument("--debug-daemon", default=None); p.add_argument("--release-daemon", default=None)
    p.add_argument("--repeats", type=int, default=3); p.add_argument("--events", type=int, default=24); p.add_argument("--concurrency", type=int, default=4)
    p.add_argument("--observe-hook", action="store_true", help="diagnostic-only inherited-handle observer per hook process")
    p.add_argument("--preload-stdin", action="store_true", help="diagnostic-only: fill a stdin pipe before starting each hook")
    args = p.parse_args()
    if not (1 <= args.repeats <= 5): sys.exit("repeats must be 1..5")
    if not (1 <= args.events <= 24 and 1 <= args.concurrency <= 4): sys.exit("events must be 1..24 and concurrency 1..4")
    out = {"environment": collect_environment(), "source": inspect_source_state(), "hook": {"path": args.hook, "sha256": compute_sha256(args.hook)}, "scenarios": {}}
    scenarios = {}
    if (args.observe_hook or args.preload_stdin) and os.name != "nt": sys.exit("--observe-hook/--preload-stdin require Windows")
    if args.debug_daemon: scenarios["debug"] = run_scenario(args.debug_daemon, args.hook, args.repeats, args.events, args.concurrency, args.observe_hook, args.preload_stdin)
    if args.release_daemon: scenarios["release"] = run_scenario(args.release_daemon, args.hook, args.repeats, args.events, args.concurrency, args.observe_hook, args.preload_stdin)
    out["scenarios"] = scenarios
    all_pass = len(scenarios) > 0 and all(s.get("verdict") == "passed" for s in scenarios.values())
    any_fail = any(s.get("verdict") == "failed" for s in scenarios.values())
    diagnostic_only = args.observe_hook or args.preload_stdin
    out["campaign_mode"] = "diagnostic_only" if diagnostic_only else "acceptance_comparable"
    out["diagnostic_verdict"] = "passed" if all_pass else ("failed" if any_fail else "not_measured")
    out["overall_verdict"] = "not_measured" if diagnostic_only else out["diagnostic_verdict"]
    sys.stdout.write(json.dumps(sanitize_for_json(out), indent=2) + "\n")
    completed_verdict = out["diagnostic_verdict"] if diagnostic_only else out["overall_verdict"]
    sys.exit(0 if completed_verdict == "passed" else 1)

if __name__ == "__main__":
    main()
