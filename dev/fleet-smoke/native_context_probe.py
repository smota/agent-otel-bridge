#!/usr/bin/env python3
"""Bounded native hook-to-daemon propagation probe.

This is a diagnostic probe: it reports child-process and transport evidence only.
Backend visibility must be checked separately against the configured collector.
"""

import argparse
import hashlib
import json
import os
import pathlib
import socket
import subprocess
import sys
import tempfile
import time
import uuid

WAIT_SECONDS = 5.0
HOOK_SECONDS = 2.0
OUTPUT_LIMIT = 64 * 1024


def bounded_hex(n):
    return os.urandom(n).hex()


def default_binary(name):
    suffix = ".exe" if os.name == "nt" else ""
    return pathlib.Path(__file__).resolve().parents[2] / "target" / "release" / (name + suffix)


def pipe_ready(pipe_name, deadline):
    if os.name == "nt":
        import ctypes
        wait = ctypes.windll.kernel32.WaitNamedPipeW
        wait.argtypes = [ctypes.c_wchar_p, ctypes.c_uint32]
        wait.restype = ctypes.c_int
        while time.monotonic() < deadline:
            remaining = max(1, int((deadline - time.monotonic()) * 1000))
            if wait(pipe_name, min(remaining, 250)):
                return True
            time.sleep(0.01)
        return False
    while time.monotonic() < deadline:
        try:
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as sock:
                sock.settimeout(0.2)
                sock.connect(pipe_name)
            return True
        except (FileNotFoundError, ConnectionRefusedError, TimeoutError, OSError):
            time.sleep(0.05)
    return False


def run_hook(hook, args, env, stdin):
    started = time.time()
    try:
        proc = subprocess.run(
            [str(hook)] + args, input=stdin, text=True, capture_output=True,
            env=env, timeout=HOOK_SECONDS, check=False,
        )
        out = proc.stdout[:OUTPUT_LIMIT]
        return {"exit_code": proc.returncode, "stdout": out, "timed_out": False,
                "duration_ms": round((time.time() - started) * 1000, 1)}
    except subprocess.TimeoutExpired:
        return {"exit_code": None, "stdout": "", "timed_out": True,
                "duration_ms": round((time.time() - started) * 1000, 1)}


def expected_context(value):
    if not value or len(value) != 55 or value[2] != "-":
        return None
    parts = value.split("-")
    if len(parts) != 4 or len(parts[1]) != 32 or len(parts[2]) != 16:
        return None
    return {"trace_id": parts[1], "parent_span_id": parts[2], "flags": parts[3], "source": "input"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--daemon", type=pathlib.Path, default=default_binary("agent-otel-bridge"))
    parser.add_argument("--hook", type=pathlib.Path, default=default_binary("agent-hook"))
    parser.add_argument("--legacy-hook", type=pathlib.Path)
    parser.add_argument("--endpoint", default="http://127.0.0.1:4318")
    args = parser.parse_args()
    args.daemon = args.daemon.resolve(strict=True)
    args.hook = args.hook.resolve(strict=True)
    if args.legacy_hook:
        args.legacy_hook = args.legacy_hook.resolve(strict=True)
    run_id = uuid.uuid4().hex
    service = "agent-otel-native-probe-" + run_id[:12]
    pipe = (r"\\.\pipe\agent-otel-native-" + run_id[:12]) if os.name == "nt" else str(
        pathlib.Path(tempfile.gettempdir()) / ("agent-otel-native-" + run_id[:12] + ".sock"))
    base_env = os.environ.copy()
    base_env.update({"AGENT_OTEL_PIPE": pipe, "AGENT_OTEL_SOCKET": pipe,
                     "OTEL_EXPORTER_OTLP_ENDPOINT": args.endpoint,
                     "OTEL_SERVICE_NAME": service, "AGENT_OTEL_IDLE_TIMEOUT_SECS": "30"})
    daemon_context = "00-" + bounded_hex(16) + "-" + bounded_hex(8) + "-01"
    base_env["TRACEPARENT"] = daemon_context
    daemon = None
    started = time.time()
    results = []
    try:
        creation = {}
        if os.name == "nt":
            creation["creationflags"] = subprocess.CREATE_NO_WINDOW | subprocess.DETACHED_PROCESS
        daemon = subprocess.Popen([str(args.daemon), "daemon"], env=base_env,
                                  stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                                  stderr=subprocess.DEVNULL, **creation)
        ready = pipe_ready(pipe, time.monotonic() + WAIT_SECONDS)
        cases = [
            ("env_a", "00-" + bounded_hex(16) + "-" + bounded_hex(8) + "-01", None),
            ("env_b", "00-" + bounded_hex(16) + "-" + bounded_hex(8) + "-01", None),
            ("explicit_d", "00-" + bounded_hex(16) + "-" + bounded_hex(8) + "-01", "00-" + bounded_hex(16) + "-" + bounded_hex(8) + "-01"),
            ("noenv_conversation_fallback", None, None),
            ("invalid_env_conversation_fallback", "not-a-traceparent", None),
        ]
        if args.legacy_hook:
            cases.append(("legacy_hook_explicit_and_env", cases[0][1], cases[2][2]))
            cases.append(("legacy_hook_env_only", cases[0][1], None))
        if not ready:
            results.append({"status": "UNSET", "reason": "daemon_not_ready"})
        else:
            for name, source, explicit in cases:
                child_env = base_env.copy()
                child_env.pop("TRACEPARENT", None)
                if source is not None:
                    child_env["TRACEPARENT"] = source
                payload = json.dumps({"conversationId": "native-probe-" + name + "-" + run_id[:8],
                                      "toolCall": {"name": "native_context_probe"},
                                      "traceparent": explicit} if explicit else
                                     {"conversationId": "native-probe-" + name + "-" + run_id[:8],
                                      "toolCall": {"name": "native_context_probe"}}, separators=(",", ":"))
                hook = args.legacy_hook if name.startswith("legacy_") else args.hook
                outcome = run_hook(hook, ["--client", "codex", "PreToolUse"], child_env, payload)
                conversation = json.loads(payload)["conversationId"]
                expected = expected_context(explicit or source)
                if name == "legacy_hook_env_only" or expected is None:
                    expected = {"trace_id": hashlib.sha256(b"agy-otel:trace_id:" + conversation.encode()).hexdigest()[:32],
                                "parent_span_id": "", "flags": "01", "source": "conversation"}
                results.append({"case": name, "source_traceparent": source,
                                "explicit_traceparent": explicit, "hook": str(hook),
                                "expected": expected,
                                "response": outcome})
        time.sleep(1.0)
    finally:
        if daemon is not None:
            daemon.terminate()
            try:
                daemon.wait(timeout=2.0)
            except subprocess.TimeoutExpired:
                daemon.kill()
                daemon.wait(timeout=1.0)
        if os.name != "nt":
            try:
                pathlib.Path(pipe).unlink()
            except FileNotFoundError:
                pass
    ended = time.time()
    plan_hash = hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest()
    report = {"status": "UNSET", "run_id": run_id, "service_name": service,
              "pipe": pipe, "daemon": str(args.daemon), "hook": str(args.hook),
              "start_unix": started, "end_unix": ended, "probe_sha256": plan_hash,
              "daemon_context": daemon_context,
              "binary_sha256": {"hook": hashlib.sha256(args.hook.read_bytes()).hexdigest(),
                                "daemon": hashlib.sha256(args.daemon.read_bytes()).hexdigest()},
              "cases": results, "backend_visibility": "UNSET"}
    print(json.dumps(report, ensure_ascii=True, separators=(",", ":")))
    return 0 if results and all(r.get("response", {}).get("exit_code") == 0 and
                               r.get("response", {}).get("stdout", "").strip() == "{}"
                               for r in results) else 1


if __name__ == "__main__":
    sys.exit(main())
