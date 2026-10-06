"""Apply the reviewed three-command projection; never read or write trust entries."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import time
import uuid

repo = Path.cwd()
output = Path(__file__).parent
projection = json.loads((output.parent / 'codex-candidate-02/projection.json').read_text())
home = Path(os.environ['USERPROFILE']) / '.codex'
active = home / 'hooks.json'
policy = home / 'agent-otel-bridge-policy.json'
candidate = Path(projection['candidate_path'])
candidate_policy = Path(projection['policy_path'])

def sha(data):
    return hashlib.sha256(data).hexdigest()

original = active.read_bytes()
replacement = candidate.read_bytes()
policy_bytes = candidate_policy.read_bytes()
assert sha(original) == projection['active_sha256'], 'Active hooks drifted'
assert sha(replacement) == projection['candidate_sha256'], 'Candidate drifted'
assert not policy.exists(), 'Existing policy requires review'
assert json.loads(policy_bytes) == {'version': 1, 'windows_hook_shell': 'powershell'}
before, after = json.loads(original), json.loads(replacement)
changes = []
for event in ('PreToolUse', 'PostToolUse', 'Stop'):
    old = before['hooks'][event][0]['hooks'][0]['command']
    new = after['hooks'][event][0]['hooks'][0]['command']
    assert 'agent-hook.exe' in old and new == '& ' + old
    changes.append({'event': event, 'before': old, 'after': new})
    before['hooks'][event][0]['hooks'][0]['command'] = new
assert before == after, 'Changes beyond three bridge commands'
codex = Path(os.environ['LOCALAPPDATA']) / 'Programs/OpenAI/Codex/bin/codex.exe'
assert sha(codex.read_bytes()) == 'be96b992178b1e467c225800da0d65f2c86d5eba1ef0b14632f65db381cbdfde'
shell = shutil.which('pwsh')
assert shell and Path(shell).is_file(), 'Expected PowerShell unavailable'
config_hash = sha((home / 'config.toml').read_bytes())
binary = Path(os.environ['LOCALAPPDATA']) / 'agent-otel-bridge/bin/agent-hook.exe'
binary_hash = sha(binary.read_bytes())
assert binary_hash == 'e776f732e2773f7aef1a70c86f8b6057824aa8070082b902585876690e05e030'
backup = Path(os.environ['LOCALAPPDATA']) / 'agent-otel-bridge/backups' / ('codex-hooks-' + uuid.uuid4().hex)
backup.mkdir(parents=True)
(backup / 'hooks.json').write_bytes(original)
assert (backup / 'hooks.json').read_bytes() == original
receipt = {'started_utc': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()),
           'backup': str(backup), 'before_sha256': sha(original),
           'after_sha256': sha(replacement), 'changes': changes,
           'shell_on_path': shell, 'shell_evidence': 'Pinned 0.154.0 source selects PowerShell on Windows; PATH resolution verified. Real hook argv still not observed.',
           'native_binary_sha256': binary_hash, 'config_sha256': config_hash,
           'policy_previously_absent': True, 'trust_written': False, 'stage': 'backed_up'}
receipt_path = output / 'application.json'
def save():
    receipt_path.write_text(json.dumps(receipt, indent=2) + '\n', encoding='utf-8', newline='\n')
save()
staged = home / ('hooks.aob-' + uuid.uuid4().hex + '.tmp')
staged.write_bytes(replacement)
assert active.read_bytes() == original and not policy.exists(), 'State drifted before application'
with policy.open('xb') as f:
    f.write(policy_bytes)
os.replace(staged, active)
assert active.read_bytes() == replacement
assert sha((home / 'config.toml').read_bytes()) == config_hash
assert sha(binary.read_bytes()) == binary_hash
receipt.update(stage='applied_pending_trust_review', third_party_preserved=True,
               config_unchanged=True, native_binary_unchanged=True, policy_sha256=sha(policy_bytes))
save()
print(json.dumps(receipt, indent=2))
