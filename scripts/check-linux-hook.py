#!/usr/bin/env python3
# Native Linux artifact checks; all IPC endpoints and child limits are isolated.
import os, json, socket, struct, subprocess, tempfile, time, resource, fcntl, selectors, hashlib, argparse
from pathlib import Path
if not __debug__:
    raise RuntimeError('Native hook checks require Python assertions; unset PYTHONOPTIMIZE')
parser = argparse.ArgumentParser(description='Check the actual packaged Linux hook against isolated native fixtures.')
parser.add_argument('binary', type=Path)
parser.add_argument('--report', type=Path)
args = parser.parse_args()
BINARY = args.binary.resolve()
EXPECTED = hashlib.sha256(BINARY.read_bytes()).hexdigest()
assert BINARY.stat().st_size < 300000, 'packaged hook exceeds the strict size gate'
TP = '00-11111111111111111111111111111111-2222222222222222-01'
RESULTS = []
TEMP = tempfile.TemporaryDirectory(prefix='aob-art-')
BASE = Path(TEMP.name)

def env(endpoint=None, watchdog='500', trace=None, state=None):
    e = os.environ.copy()
    for key in ['TRACEPARENT', 'TRACESTATE', 'AGENT_OTEL_WATCHDOG_MS']:
        e.pop(key, None)
    e['AGENT_OTEL_SOCKET'] = str(endpoint or BASE / 'missing.sock')
    if watchdog is not None:
        e['AGENT_OTEL_WATCHDOG_MS'] = watchdog
    if trace is not None:
        e['TRACEPARENT'] = trace
    if state is not None:
        e['TRACESTATE'] = state
    return e

def run(args, data=b'{}', **kw):
    return subprocess.run([str(BINARY), *args], input=data, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env(**kw), timeout=2)

def check(name, fn):
    start = time.monotonic()
    try:
        detail = fn() or {}
        RESULTS.append({'name': name, 'passed': True, 'wall_ms': (time.monotonic() - start) * 1000, **detail})
    except Exception as error:
        RESULTS.append({'name': name, 'passed': False, 'error': type(error).__name__ + ': ' + str(error)})
        raise
    finally:
        if args.report is not None:
            args.report.write_text(json.dumps({'artifact': str(BINARY), 'sha256': EXPECTED, 'checks': RESULTS}, indent=2) + '\n')

def clean(p):
    if p.poll() is None:
        p.kill()
        p.wait(timeout=2)

def responses():
    for client in ['antigravity', 'claude', 'codex', 'grok', 'pi']:
        r = run(['PreToolUse', '--client', client])
        assert r.returncode == 0 and (not r.stderr)
        assert r.stdout == (b'{}' if client == 'codex' else b'{"decision":"allow"}')
    for trace, state, expected in [(TP, 'vendor=a', {'TRACEPARENT': TP, 'TRACESTATE': 'vendor=a'}), ('bad"value', 'vendor=a', {'TRACESTATE': 'vendor=a'}), (TP, 'bad"value', {'TRACEPARENT': TP})]:
        r = run(['PreToolUse', '--client', 'claude'], trace=trace, state=state)
        assert r.returncode == 0
        assert json.loads(r.stdout) == {'decision': 'allow', 'hookSpecificOutput': {'env': expected}}
    r = run(['PreToolUse', '--client', 'codex'], trace=TP, state='vendor=a')
    assert r.returncode == 0 and r.stdout == b'{}'

def before_stdin():
    p = subprocess.Popen([str(BINARY), 'PreToolUse', '--client', 'claude'], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env())
    try:
        os.set_blocking(p.stdout.fileno(), False)
        expected = b'{"decision":"allow"}'
        response = b''
        deadline = time.monotonic() + 1
        selector = selectors.DefaultSelector()
        selector.register(p.stdout, selectors.EVENT_READ)
        while len(response) < len(expected) and time.monotonic() < deadline:
            if selector.select(0.05):
                chunk = os.read(p.stdout.fileno(), 4096)
                if not chunk:
                    break
                response += chunk
        selector.close()
        assert response == expected
        p.stdin.write(b'{}')
        p.stdin.close()
        p.stdin = None
        out, err = p.communicate(timeout=2)
        assert p.returncode == 0 and out == b'' and (err == b'')
    finally:
        clean(p)

def blocked_stdin():
    p = subprocess.Popen([str(BINARY), 'PostToolUse', '--client', 'codex'], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env(watchdog=None))
    start = time.monotonic()
    try:
        p.wait(timeout=2)
        assert p.returncode == 0
        out = p.stdout.read()
        assert out in [b'{}', b'']
        return {'default_watchdog_process_wall_ms': (time.monotonic() - start) * 1000, 'internal_deadline_measurement': False}
    finally:
        clean(p)

def blocked_stdout():
    reader, writer = socket.socketpair()
    try:
        writer.setblocking(False)
        for block in [b'x' * 4096, b'x']:
            while True:
                try:
                    writer.send(block)
                except BlockingIOError:
                    break
        writer.setblocking(True)
        p = subprocess.Popen([str(BINARY), 'PostToolUse', '--client', 'codex'], stdin=subprocess.DEVNULL, stdout=writer, stderr=subprocess.PIPE, env=env(watchdog='25'))
        try:
            p.wait(timeout=2)
            assert p.returncode == 0
        finally:
            clean(p)
    finally:
        reader.close()
        writer.close()

def thread_failure():
    assert os.geteuid() != 0, 'RLIMIT_NPROC fixture needs unprivileged caller'

    def restrict():
        resource.setrlimit(resource.RLIMIT_NPROC, (0, 0))
    r = subprocess.run([str(BINARY), 'PostToolUse', '--client', 'codex'], stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env(), preexec_fn=restrict, timeout=2)
    assert r.returncode == 0 and r.stdout == b'' and (r.stderr == b'')

def invalid_argv():
    r = run([b'invalid-\xff', b'PostToolUse', b'--client', b'codex'])
    assert r.returncode == 0 and r.stdout == b'{}' and (not r.stderr)

def receiver():
    payload = json.dumps({'session_id': 'synthetic-artifact-fixture', 'cwd': '/fixture', 'tool_name': 'Read', 'note': 'synthetic λ\nfixture'}, ensure_ascii=False).encode()
    for client, wire_id in [('antigravity', 1), ('claude', 2), ('codex', 3), ('grok', 4), ('pi', 5)]:
        endpoint = BASE / (client + '.sock')
        listener = socket.socket(socket.AF_UNIX)
        listener.settimeout(2)
        listener.bind(str(endpoint))
        listener.listen(1)
        try:
            r = run(['PostToolUse', '--client', client], data=payload, endpoint=endpoint, trace=TP)
            assert r.returncode == 0 and r.stdout == b'{}'
            connection, _ = listener.accept()
            connection.settimeout(2)
            with connection:
                data = b''
                while True:
                    chunk = connection.recv(65536)
                    if not chunk:
                        break
                    data += chunk
            assert data[:4] == b'AG\x01\x04'
            length = struct.unpack_from('<I', data, 4)[0]
            assert len(data) == 8 + length
            body = data[8:]
            assert struct.unpack_from('<BH', body, 0) == (4, wire_id)
            assert body[3] == 1
            context_length = struct.unpack_from('<H', body, 4)[0]
            assert body[6:6 + context_length] == TP.encode()
            assert body[6 + context_length:] == payload
        finally:
            listener.close()
            endpoint.unlink(missing_ok=True)
    return {'client_wire_identities_verified': 5, 'json_bytes_exact': True, 'traceparent_bytes_exact': True}

def fd_flags():
    incoming, child_in = socket.socketpair()
    outgoing, child_out = socket.socketpair()
    try:
        before = [fcntl.fcntl(x.fileno(), fcntl.F_GETFL) for x in [child_in, child_out]]
        incoming.sendall(b'{}')
        incoming.shutdown(socket.SHUT_WR)
        p = subprocess.Popen([str(BINARY), 'PostToolUse', '--client', 'codex'], stdin=child_in, stdout=child_out, stderr=subprocess.PIPE, env=env())
        try:
            p.wait(timeout=2)
            assert p.returncode == 0
        finally:
            clean(p)
        after = [fcntl.fcntl(x.fileno(), fcntl.F_GETFL) for x in [child_in, child_out]]
        assert before == after
        outgoing.settimeout(2)
        assert outgoing.recv(16) == b'{}'
        return {'stdin_stdout_F_GETFL_before': before, 'stdin_stdout_F_GETFL_after': after}
    finally:
        for stream in [incoming, child_in, outgoing, child_out]:
            stream.close()
try:
    for name, fn in [('all_harness_responses_and_safe_context', responses), ('response_before_stdin_and_once', before_stdin), ('default_3ms_policy_blocked_stdin', blocked_stdin), ('blocked_stdout_fail_open', blocked_stdout), ('thread_creation_failure_fail_open', thread_failure), ('invalid_UTF8_argv_fail_open', invalid_argv), ('Unix_receiver_exact_wire_header_trace_and_payload', receiver), ('inherited_stdin_stdout_flags_unchanged', fd_flags)]:
        check(name, fn)
    assert hashlib.sha256(BINARY.read_bytes()).hexdigest() == EXPECTED
    print(json.dumps({'passed': len(RESULTS), 'artifact_sha256': EXPECTED, 'artifact': str(BINARY)}))
finally:
    TEMP.cleanup()
