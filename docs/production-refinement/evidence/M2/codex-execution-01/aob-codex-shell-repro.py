import json,os,subprocess,uuid
from pathlib import Path
shell=Path(os.environ['USERPROFILE'])/'.cache/codex-runtimes/codex-primary-runtime/dependencies/native/powershell/pwsh.exe'
config=json.loads((Path(os.environ['USERPROFILE'])/'.codex/hooks.json').read_text())
result={'shell':str(shell),'synthetic':True,'cases':[]}
for event in ['PreToolUse','PostToolUse','Stop']:
 command=config['hooks'][event][0]['hooks'][0]['command']
 for label,prefix in [('configured',''),('call_operator','& ')]:
  payload={'session_id':'m2-shell-'+label,'hook_event_name':event,'tool_name':'Bash','tool_input':{},'cwd':str(Path.cwd())}
  argv=[str(shell),'-NoProfile','-Command',prefix+command]
  p=subprocess.run(argv,input=json.dumps(payload),capture_output=True,text=True,encoding='utf-8',timeout=5)
  result['cases'].append({'event':event,'variant':label,'argv':argv,'exit_code':p.returncode,'stdout':p.stdout,'stderr':p.stderr})
print(json.dumps(result))
