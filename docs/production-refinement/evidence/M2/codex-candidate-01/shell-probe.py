import os,json,subprocess,pathlib,tempfile,time,hashlib,uuid
repo=pathlib.Path.cwd(); out=repo/'docs/production-refinement/evidence/M2/codex-candidate-01'; out.mkdir(exist_ok=True)
home=pathlib.Path(tempfile.mkdtemp(prefix='aob-codex-candidate-'))
env=os.environ.copy(); env.update(USERPROFILE=str(home),HOME=str(home))
cli=repo/'target/debug/agent-otel-bridge.exe'; active=pathlib.Path(os.environ['LOCALAPPDATA'])/'agent-otel-bridge/bin/agent-hook.exe'
for name,binary in [('canonical',active),('native',repo/'target/release/agent-hook.exe')]:
 p=subprocess.run([str(cli),'install-hooks','--client','codex','--binary',str(binary)],env=env,capture_output=True,text=True,timeout=15)
 assert p.returncode==0,(p.returncode,p.stderr)
 config=json.loads((home/'.codex/hooks.json').read_text())
 if name=='canonical':
  (out/'hooks-preview.json').write_text(json.dumps(config,indent=2)+'\n',encoding='utf8')
 else: native=config
root=os.environ['SystemRoot']; shells={'cmd':str(pathlib.Path(root)/'System32/cmd.exe'),'powershell':str(pathlib.Path(root)/'System32/WindowsPowerShell/v1.0/powershell.exe'),'pwsh':str(pathlib.Path(os.environ['USERPROFILE'])/'.cache/codex-runtimes/codex-primary-runtime/dependencies/native/powershell/pwsh.exe')}
results=[]; env['AGENT_OTEL_PIPE']=r'\\.\pipe\aob-candidate-absent-'+uuid.uuid4().hex
env['PATH']=''
for shell,exe in shells.items():
 for event,groups in native['hooks'].items():
  command=groups[0]['hooks'][0]['command']; args=f'"{exe}" /D /S /C "{command}"' if shell=='cmd' else [exe,'-NoProfile','-NonInteractive','-Command',command]
  start=time.monotonic(); p=subprocess.run(args,input=json.dumps({'session_id':'m2-candidate-disconnected','hook_event_name':event}).encode(),capture_output=True,env=env,timeout=8)
  result={'shell':shell,'event':event,'exit_code':p.returncode,'stdout':p.stdout.decode().strip(),'stderr_empty':not p.stderr,'elapsed_ms':round((time.monotonic()-start)*1000)};results.append(result)
  assert p.returncode==0 and p.stdout.strip()==b'{}' and not p.stderr,result
receipt={'scope':'isolated candidate installer and disconnected native hook; no Codex turn or OTLP delivery','native_binary':str(binary),'native_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'private_home':str(home),'pipe':env['AGENT_OTEL_PIPE'],'results':results}
(out/'shell-results.json').write_text(json.dumps(receipt,indent=2)+'\n',encoding='utf8')
print(json.dumps(receipt,indent=2))