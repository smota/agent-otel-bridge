import pathlib,os,json,subprocess,tempfile,hashlib
repo=pathlib.Path.cwd(); folder=repo/'docs/production-refinement/evidence/M2/codex-candidate-02'; user=pathlib.Path(os.environ['USERPROFILE']); original=(user/'.codex/hooks.json').read_bytes(); before=json.loads(original)
home=pathlib.Path(tempfile.mkdtemp(prefix='codex-projection-',dir=repo/'target')); config=home/'.codex'; config.mkdir(); (config/'hooks.json').write_bytes(original)
policy=(folder/'agent-otel-bridge-policy.json').read_bytes(); (config/'agent-otel-bridge-policy.json').write_bytes(policy)
env=os.environ.copy(); env.update(USERPROFILE=str(home),HOME=str(home)); binary=str(pathlib.Path(os.environ['LOCALAPPDATA'])/'agent-otel-bridge/bin/agent-hook.exe')
p=subprocess.run([str(repo/'target/debug/agent-otel-bridge.exe'),'install-hooks','--client','codex','--binary',binary],env=env,capture_output=True,text=True,timeout=15); assert p.returncode==0,p.stderr
after=json.loads((config/'hooks.json').read_bytes()); changes=[]
def compare(a,b,path=''):
 if isinstance(a,dict) and isinstance(b,dict):
  assert a.keys()==b.keys(),path
  for key in a: compare(a[key],b[key],path+'/'+key)
 elif isinstance(a,list) and isinstance(b,list):
  assert len(a)==len(b),path
  for i,(x,y) in enumerate(zip(a,b)): compare(x,y,path+'/'+str(i))
 elif a!=b: changes.append(path)
compare(before,after)
assert len(changes)==3 and all(p.endswith('/command') for p in changes),changes
assert (user/'.codex/hooks.json').read_bytes()==original
receipt={'active_sha256':hashlib.sha256(original).hexdigest(),'candidate_sha256':hashlib.sha256((config/'hooks.json').read_bytes()).hexdigest(),'candidate_path':str(config/'hooks.json'),'policy_path':str(config/'agent-otel-bridge-policy.json'),'changed_json_paths':changes,'active_unchanged':True,'scope':'preview only; three bridge command strings changed; no trust changes'}
(folder/'projection.json').write_text(json.dumps(receipt,indent=2)+'\n',encoding='utf8',newline='\n');print(json.dumps(receipt,indent=2))