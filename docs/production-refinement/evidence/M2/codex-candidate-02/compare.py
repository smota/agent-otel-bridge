import os,json,subprocess,pathlib,tempfile,time,hashlib,uuid,statistics
repo=pathlib.Path.cwd(); out=repo/'docs/production-refinement/evidence/M2/codex-candidate-02'; out.mkdir(exist_ok=True)
home=pathlib.Path(tempfile.mkdtemp(prefix='aob-codex-policy-')); config_dir=home/'.codex'; config_dir.mkdir()
env=os.environ.copy(); env.update(USERPROFILE=str(home),HOME=str(home))
cli=repo/'target/debug/agent-otel-bridge.exe'; binary=repo/'target/release/agent-hook.exe'; active=pathlib.Path(os.environ['LOCALAPPDATA'])/'agent-otel-bridge/bin/agent-hook.exe'
commands={}
for mode in ['portable','powershell']:
 policy={'version':1,'windows_hook_shell':mode}; (config_dir/'agent-otel-bridge-policy.json').write_text(json.dumps(policy),encoding='utf8')
 for kind,path in [('canonical',active),('native',binary)]:
  p=subprocess.run([str(cli),'install-hooks','--client','codex','--binary',str(path)],env=env,capture_output=True,text=True,timeout=15); assert p.returncode==0,(p.returncode,p.stderr)
  config=json.loads((config_dir/'hooks.json').read_text())
  if kind=='canonical': (out/f'preview-{mode}.json').write_text(json.dumps(config,indent=2)+'\n',encoding='utf8',newline='\n')
  else: commands[mode]={event:groups[0]['hooks'][0]['command'] for event,groups in config['hooks'].items()}
 if mode=='powershell': (out/'agent-otel-bridge-policy.json').write_text(json.dumps(policy,indent=2)+'\n',encoding='utf8',newline='\n')
root=os.environ['SystemRoot']; shells={'windows-powershell':str(pathlib.Path(root)/'System32/WindowsPowerShell/v1.0/powershell.exe'),'codex-pwsh':str(pathlib.Path(os.environ['USERPROFILE'])/'.cache/codex-runtimes/codex-primary-runtime/dependencies/native/powershell/pwsh.exe')}
env['PATH']=''; env['AGENT_OTEL_PIPE']=r'\\.\pipe\aob-policy-absent-'+uuid.uuid4().hex
results=[]
for shell,exe in shells.items():
 for repeat in range(3):
  for event in ['PreToolUse','PostToolUse','Stop']:
   modes=['portable','powershell'] if repeat%2==0 else ['powershell','portable']
   for mode in modes:
    command=commands[mode][event]; args=[exe,'-NoProfile','-NonInteractive','-Command',command]
    start=time.perf_counter(); p=subprocess.Popen(args,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,env=env)
    try: stdout,stderr=p.communicate(json.dumps({'session_id':'m2-candidate02-disconnected','hook_event_name':event}).encode(),timeout=8)
    except subprocess.TimeoutExpired:
     subprocess.run([str(pathlib.Path(root)/'System32/taskkill.exe'),'/PID',str(p.pid),'/T','/F'],capture_output=True); p.wait(); raise
    row={'shell':shell,'mode':mode,'event':event,'repeat':repeat,'elapsed_ms':round((time.perf_counter()-start)*1000,2),'exit_code':p.returncode,'stdout':stdout.decode().strip(),'stderr_empty':not stderr};results.append(row)
    assert p.returncode==0 and stdout.strip()==b'{}' and not stderr,row
summary={}
for shell in shells:
 summary[shell]={}
 for mode in commands:
  values=[r['elapsed_ms'] for r in results if r['shell']==shell and r['mode']==mode]
  summary[shell][mode]={'n':len(values),'min_ms':min(values),'median_ms':statistics.median(values),'max_ms':max(values)}
 summary[shell]['median_reduction_percent']=round((1-summary[shell]['powershell']['median_ms']/summary[shell]['portable']['median_ms'])*100,1)
receipt={'scope':'paired disconnected shell comparison; whole invocation including process startup, not native SLA or Codex E2E','binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'private_home':str(home),'pipe':env['AGENT_OTEL_PIPE'],'shells':shells,'summary':summary,'results':results}
(out/'comparison.json').write_text(json.dumps(receipt,indent=2)+'\n',encoding='utf8',newline='\n')
print(json.dumps(summary,indent=2))