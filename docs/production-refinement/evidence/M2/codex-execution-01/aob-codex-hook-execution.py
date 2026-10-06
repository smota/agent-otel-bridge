import json,os,queue,subprocess,sys,threading,time
from pathlib import Path
marker=Path(sys.argv[1]); deadline=time.monotonic()+95
exe=Path(os.environ['LOCALAPPDATA'])/'Programs/OpenAI/Codex/bin/codex.exe'
p=subprocess.Popen([str(exe),'app-server','--stdio'],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.DEVNULL,text=True,encoding='utf-8',creationflags=0x08000000)
q=queue.Queue(); result={'app_server_pid':p.pid,'hooks':[],'commands':[]}
def read():
 for line in p.stdout:q.put(json.loads(line))
threading.Thread(target=read,daemon=True).start()
def send(method,params,id=None):
 obj={'jsonrpc':'2.0','method':method,'params':params}
 if id is not None:obj['id']=id
 p.stdin.write(json.dumps(obj)+'\n');p.stdin.flush()
def next_message():
 obj=q.get(timeout=max(.01,deadline-time.monotonic()))
 method=obj.get('method','');params=obj.get('params',{})
 if method=='hook/completed':
  run=params['run']
  if run.get('eventName') in ['preToolUse','postToolUse','stop'] and str(run.get('sourcePath','')).lower().endswith('.codex\\hooks.json'):
   result['hooks'].append(params)
 if method=='item/completed':
  item=params.get('item',{})
  if item.get('type')=='commandExecution':
   result['commands'].append({'id':item.get('id'),'status':item.get('status'),'exitCode':item.get('exitCode'),'output_matches_marker':str(item.get('aggregatedOutput','')).strip()==marker.read_text()})
  if item.get('type')=='agentMessage':result['response_matches_marker']=item.get('text','').strip()==marker.read_text()
 if 'id' in obj and 'method' in obj:
  raise RuntimeError('Unexpected server request: '+method)
 return obj
def response(id):
 while time.monotonic()<deadline:
  obj=next_message()
  if obj.get('id')==id:
   if 'error' in obj:raise RuntimeError(obj['error'])
   return obj['result']
 raise TimeoutError('RPC deadline')
try:
 send('initialize',{'clientInfo':{'name':'aob-hook-execution','version':'1.0.0'},'capabilities':{}},1);response(1)
 send('initialized',{})
 send('thread/start',{'model':'gpt-5.6-luna','cwd':str(Path.cwd()),'approvalPolicy':'never','sandbox':'read-only'},2)
 started=response(2);result['thread_id']=started['thread']['id'];result['model']=started.get('model');result['sandbox']=started.get('sandbox')
 prompt='Execute exactly one read-only shell command to print the content of this file: '+str(marker)+'. Return only its exact content. Do not edit files, read other files, browse, or spawn agents. This is an authorized observability test. Stop after this one read.'
 send('turn/start',{'threadId':result['thread_id'],'input':[{'type':'text','text':prompt,'text_elements':[]}],'effort':'medium'},3);response(3)
 while time.monotonic()<deadline:
  obj=next_message()
  if obj.get('method')=='turn/completed':
   result['turn_status']=obj['params']['turn']['status'];break
 else:raise TimeoutError('turn deadline')
except Exception as e:result['error']=str(e)
finally:
 p.stdin.close()
 try:p.wait(timeout=3)
 except subprocess.TimeoutExpired:p.kill();p.wait(timeout=3);result['forced_stop']=True
 result['app_server_exit_code']=p.returncode
 print(json.dumps(result))
 sys.exit(1 if 'error' in result else 0)
