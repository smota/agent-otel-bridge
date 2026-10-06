"""Read-only hook discovery. No model turn, hook invocation, or trust mutation."""
import hashlib
import json
import os
from pathlib import Path
import queue
import subprocess
import sys
import threading
import time

repo = Path.cwd()
output = Path(sys.argv[1])
profile = Path(os.environ['USERPROFILE'])
exe = Path(os.environ['LOCALAPPDATA']) / 'Programs/OpenAI/Codex/bin/codex.exe'
configs = [profile / '.codex/hooks.json', profile / '.codex/config.toml']


def hashes():
    return {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in configs}


receipt = {'started_utc': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()),
           'exe_sha256': hashlib.sha256(exe.read_bytes()).hexdigest(),
           'config_before': hashes(), 'mode': 'inherited environment',
           'methods': ['initialize', 'initialized', 'hooks/list']}
env = os.environ.copy()
for key in ['OPENAI_API_KEY', 'ANTHROPIC_API_KEY', 'GEMINI_API_KEY',
            'GOOGLE_API_KEY', 'XAI_API_KEY', 'GROK_API_KEY']:
    env.pop(key, None)
p = subprocess.Popen([str(exe), 'app-server', '--stdio'], cwd=repo, env=env,
                     stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                     stderr=subprocess.DEVNULL, text=True, encoding='utf-8',
                     creationflags=0x08000000)
receipt['pid'] = p.pid
lines = queue.Queue()


def reader():
    for line in p.stdout:
        lines.put(line)


threading.Thread(target=reader, daemon=True).start()
deadline = time.monotonic() + 30


def send(message):
    p.stdin.write(json.dumps(message) + '\n')
    p.stdin.flush()


def response(wanted):
    while time.monotonic() < deadline:
        obj = json.loads(lines.get(timeout=max(.01, deadline-time.monotonic())))
        if obj.get('id') == wanted:
            if 'error' in obj:
                raise RuntimeError(obj['error'])
            return obj['result']
    raise TimeoutError('RPC deadline')


try:
    send({'jsonrpc': '2.0', 'id': 1, 'method': 'initialize', 'params': {
        'clientInfo': {'name': 'aob-hook-diagnostic', 'version': '1.0.0'},
        'capabilities': {}}})
    receipt['initialized'] = bool(response(1))
    send({'jsonrpc': '2.0', 'method': 'initialized', 'params': {}})
    send({'jsonrpc': '2.0', 'id': 2, 'method': 'hooks/list',
          'params': {'cwds': [str(repo)]}})
    result = response(2)
    receipt['entries'] = []
    fields = ['eventName', 'enabled', 'trustStatus', 'currentHash', 'key',
              'source', 'sourcePath', 'matcher', 'command', 'timeoutSec']
    for entry in result['data']:
        hooks = entry['hooks']
        receipt['entries'].append({
            'cwd': entry['cwd'], 'errors': entry['errors'],
            'warnings': entry['warnings'], 'total_hook_count': len(hooks),
            'bridge_hooks': [{k: h.get(k) for k in fields}
                             for h in hooks if 'agent-hook.exe' in h.get('command', '')]})
except Exception as exc:
    receipt['error'] = str(exc)
finally:
    p.stdin.close()
    try:
        p.wait(timeout=5)
    except subprocess.TimeoutExpired:
        p.kill()
        p.wait(timeout=5)
        receipt['forced_process_stop'] = True
    receipt['exit_code'] = p.returncode
    receipt['alive_after'] = p.poll() is None
    receipt['config_after'] = hashes()
    receipt['config_unchanged'] = receipt['config_before'] == receipt['config_after']
    output.write_text(json.dumps(receipt, indent=2) + '\n', encoding='utf-8')
    print(json.dumps(receipt, indent=2))
