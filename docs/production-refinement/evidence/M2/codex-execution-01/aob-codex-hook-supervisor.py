import os,sys,json,time,uuid,hashlib,subprocess,threading,struct,ctypes,http.server
from pathlib import Path
from ctypes import wintypes as W
base=Path(os.environ['TEMP'])/'aob-m2-real-20261006'
repo=Path.cwd()
client=sys.argv[1] if len(sys.argv)>1 else 'agy'
assert client in ['agy','grok','claude','codex']
profile=Path(os.environ['USERPROFILE'])
config=profile/{'agy':'.gemini/config/hooks.json','grok':'.grok/hooks/agent-otel.json','claude':'.claude/settings.json','codex':'.codex/hooks.json'}[client]
claude_config=profile/'.claude/settings.json'
claude_hash=hashlib.sha256(claude_config.read_bytes()).hexdigest()
config_before=hashlib.sha256(config.read_bytes()).hexdigest()
run=base/(client+'-'+uuid.uuid4().hex[:10]); run.mkdir()
exe=Path(os.environ['LOCALAPPDATA'])/'agent-otel-bridge/bin/agent-otel-bridge.exe'
expected='24c73837129c674d80df727b85d04915952fc9c73b9bffb451e6a531497c6e9d'
assert hashlib.sha256(exe.read_bytes()).hexdigest()==expected
pipe='\\\\.\\pipe\\agent-otel-m2-'+uuid.uuid4().hex
k=ctypes.WinDLL('kernel32',use_last_error=True)
k.CreateJobObjectW.argtypes=[ctypes.c_void_p,W.LPCWSTR]; k.CreateJobObjectW.restype=W.HANDLE
k.SetInformationJobObject.argtypes=[W.HANDLE,ctypes.c_int,ctypes.c_void_p,W.DWORD]
k.AssignProcessToJobObject.argtypes=[W.HANDLE,W.HANDLE]
k.CloseHandle.argtypes=[W.HANDLE]
k.TerminateJobObject.argtypes=[W.HANDLE,W.UINT]
k.CreateFileW.argtypes=[W.LPCWSTR,W.DWORD,W.DWORD,ctypes.c_void_p,W.DWORD,W.DWORD,W.HANDLE]; k.CreateFileW.restype=W.HANDLE
k.GetNamedPipeServerProcessId.argtypes=[W.HANDLE,ctypes.POINTER(W.ULONG)]
k.WriteFile.argtypes=[W.HANDLE,ctypes.c_void_p,W.DWORD,ctypes.POINTER(W.DWORD),ctypes.c_void_p]
class BASIC(ctypes.Structure):
 _fields_=[('PerProcessUserTimeLimit',ctypes.c_longlong),('PerJobUserTimeLimit',ctypes.c_longlong),('LimitFlags',W.DWORD),('MinimumWorkingSetSize',ctypes.c_size_t),('MaximumWorkingSetSize',ctypes.c_size_t),('ActiveProcessLimit',W.DWORD),('Affinity',ctypes.c_size_t),('PriorityClass',W.DWORD),('SchedulingClass',W.DWORD)]
class IO(ctypes.Structure):
 _fields_=[(n,ctypes.c_ulonglong) for n in ['ReadOperationCount','WriteOperationCount','OtherOperationCount','ReadTransferCount','WriteTransferCount','OtherTransferCount']]
class EXT(ctypes.Structure):
 _fields_=[('BasicLimitInformation',BASIC),('IoInfo',IO),('ProcessMemoryLimit',ctypes.c_size_t),('JobMemoryLimit',ctypes.c_size_t),('PeakProcessMemoryUsed',ctypes.c_size_t),('PeakJobMemoryUsed',ctypes.c_size_t)]
def pipes():
 return [n for n in os.listdir('\\\\.\\pipe\\') if n.startswith(('agent-otel','agy-otel'))]
assert not pipes(), 'Existing bridge pipes: do not disturb'
job=k.CreateJobObjectW(None,None); assert job
limit=EXT(); limit.BasicLimitInformation.LimitFlags=0x2000
assert k.SetInformationJobObject(job,9,ctypes.byref(limit),ctypes.sizeof(limit))
requests=[]; request_lock=threading.Lock()
class Sink(http.server.BaseHTTPRequestHandler):
 def do_POST(self):
  size=int(self.headers.get('Content-Length','0')); assert 0<=size<=2097152
  self.connection.settimeout(2)
  body=self.rfile.read(size)
  with request_lock:
   name='request-%03d.bin'%len(requests); (run/name).write_bytes(body)
   requests.append({'path':self.path,'bytes':len(body),'sha256':hashlib.sha256(body).hexdigest(),'file':name})
  self.send_response(200); self.send_header('Content-Type','application/x-protobuf'); self.send_header('Content-Length','0'); self.end_headers()
 def log_message(self,*args): pass
sink=http.server.ThreadingHTTPServer(('127.0.0.1',0),Sink)
threading.Thread(target=sink.serve_forever,daemon=True).start()
env={n:os.environ[n] for n in ['SystemRoot','WINDIR'] if n in os.environ}; env['PATH']=os.path.join(os.environ['SystemRoot'],'System32')
for n in ['USERPROFILE','HOME','LOCALAPPDATA','APPDATA','TEMP','TMP']:
 d=run/n; d.mkdir(); env[n]=str(d)
env.update(AGENT_OTEL_PIPE=pipe,AGY_OTEL_PIPE=pipe,OTEL_EXPORTER_OTLP_ENDPOINT='http://127.0.0.1:'+str(sink.server_port),OTEL_SERVICE_NAME='agent-otel-m2-isolated',AGENT_OTEL_IDLE_TIMEOUT_SECS='180',AGENT_OTEL_QUOTA_INTERVAL_SECS='60',AGENT_OTEL_QUOTA_FILE=str(run/'absent-quota.json'))
r={'started_utc':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),'exe':str(exe),'sha256':expected,'supervisor_pid':os.getpid(),'pipe':pipe,'endpoint':env['OTEL_EXPORTER_OTLP_ENDPOINT'],'env':env,'run':str(run),'requests':requests,'stage':'intent'}
def save(): (run/'receipt.json').write_text(json.dumps(r,indent=2),encoding='utf-8')
r['hook_config_sha256_before']=config_before
save(); p=None; timer=None; harness=None

def send(typ,body=b''):
 h=k.CreateFileW(pipe,0x40000000,0,None,3,0,None)
 if h==ctypes.c_void_p(-1).value: raise OSError(ctypes.get_last_error(),'CreateFile pipe')
 try:
  owner=W.ULONG(); assert k.GetNamedPipeServerProcessId(h,ctypes.byref(owner)); assert owner.value==p.pid, 'Wrong pipe owner'
  frame=b'AG'+bytes([1,typ])+struct.pack('<I',len(body))+body
  wrote=W.DWORD(); assert k.WriteFile(h,frame,len(frame),ctypes.byref(wrote),None); assert wrote.value==len(frame)
  return owner.value
 finally: k.CloseHandle(h)
try:
 with (run/'stdout.txt').open('wb') as out,(run/'stderr.txt').open('wb') as err:
  p=subprocess.Popen([str(exe),'daemon'],cwd=run,env=env,stdout=out,stderr=err,stdin=subprocess.DEVNULL,creationflags=0x4|0x08000000)
  r['pid']=p.pid; r['stage']='suspended'; save()
  assert k.AssignProcessToJobObject(job,W.HANDLE(int(p._handle))), 'Job assignment failed'
  timer=threading.Timer(150,lambda:k.TerminateJobObject(job,124)); timer.start()
  nt=ctypes.WinDLL('ntdll'); nt.NtResumeProcess.argtypes=[W.HANDLE]; assert nt.NtResumeProcess(W.HANDLE(int(p._handle)))==0
  r['stage']='running'; save()
  deadline=time.monotonic()+6
  while time.monotonic()<deadline and p.poll() is None and pipe.split('\\')[-1] not in pipes(): time.sleep(.05)
  r['pipes_after_start']=pipes()
  assert not any(n in ['agent-otel','agy-otel'] for n in r['pipes_after_start']), 'Unexpected default pipe'
  if p.poll() is None and pipe.split('\\')[-1] in r['pipes_after_start']:
   r['pipe_owner_pid']=send(3)
   marker=run/'marker.txt'; marker.write_text('m2-real-'+client+'-'+uuid.uuid4().hex,encoding='utf-8')
   harness_env=os.environ.copy()
   removed=[]
   for name in ['CODEX_THREAD_ID','CODEX_SESSION_ID','CODEX_CI','CODEX_INTERNAL_ORIGINATOR_OVERRIDE','CODEX_APP_TOOLS_PIPE_PATH','CODEX_TASK_WORKSPACE_VERIFYING_IDENTITY']:
    if name in harness_env:
     harness_env.pop(name);removed.append(name)
   r['removed_coordinator_env_names']=removed
   r['comparison']='coordinator identity variables removed; sandbox/dependency/home variables preserved'
   for key in ['OPENAI_API_KEY','ANTHROPIC_API_KEY','GEMINI_API_KEY','GOOGLE_API_KEY','XAI_API_KEY','GROK_API_KEY']:
    harness_env.pop(key,None)
   for key in list(harness_env):
    if key.startswith('OTEL_'):harness_env.pop(key,None)
   harness_env.update(AGENT_OTEL_PIPE=pipe,AGY_OTEL_PIPE=pipe,OTEL_EXPORTER_OTLP_ENDPOINT=env['OTEL_EXPORTER_OTLP_ENDPOINT'],OTEL_EXPORTER_OTLP_PROTOCOL='http/protobuf')
   agy=Path(os.environ['LOCALAPPDATA'])/'agy/bin/agy.exe'
   prompt='Read exactly this file with your file-read tool: '+str(marker)+'. Return only its exact content. Do not use shell, edit files, read other files, browse, or spawn agents. This is an authorized observability test; perform one file read and stop.'
   argv=[str(agy),'--model','gemini-3.8-flash-low','--mode','plan','--print-timeout','90s','--output-format','stream-json','--print',prompt]
   if client=='grok':
    agy=Path(os.environ['LOCALAPPDATA'])/'Microsoft/WinGet/Links/grok.exe'
    argv=[str(agy),'--model','grok-4.7-build-fast','--reasoning-effort','low','--permission-mode','plan','--no-subagents','--max-turns','3','--disable-web-search','--output-format','streaming-messages-json','-p',prompt]
   elif client=='claude':
    agy=Path(os.environ['LOCALAPPDATA'])/'Microsoft/WinGet/Links/claude.exe'
    argv=[str(agy),'--model','haiku','--effort','low','--permission-mode','plan','--allowedTools','Read','--output-format','stream-json','--verbose','-p',prompt]
   elif client=='codex':
    agy=Path(os.environ['LOCALAPPDATA'])/'Programs/OpenAI/Codex/bin/codex.exe'
    prompt='Execute exactly one read-only shell command to print the content of this file: '+str(marker)+'. Return only its exact content. Do not edit files, read other files, browse, or spawn agents. This is an authorized observability test. Stop after this one read.'
    argv=[sys.executable,str(Path(os.environ['TEMP'])/'aob-codex-hook-execution.py'),str(marker)]
   r['marker_file']=str(marker); r['marker_sha256']=hashlib.sha256(marker.read_bytes()).hexdigest(); r['requested_model']={'agy':'gemini-3.8-flash-low','grok':'grok-4.7-build-fast','claude':'haiku','codex':'gpt-5.6-luna'}[client]; r['harness_exe']=str(agy);save()
   with (run/'harness.stream.jsonl').open('wb') as ho,(run/'harness.stderr.txt').open('wb') as he:
    harness=subprocess.Popen(argv,cwd=repo,env=harness_env,stdin=subprocess.DEVNULL,stdout=ho,stderr=he,creationflags=0x4|0x08000000)
    r['harness_pid']=harness.pid;save()
    if not k.AssignProcessToJobObject(job,W.HANDLE(int(harness._handle))):
     harness.kill();harness.wait(timeout=3);raise RuntimeError('harness job assignment failed')
    assert nt.NtResumeProcess(W.HANDLE(int(harness._handle)))==0
    try:r['harness_exit_code']=harness.wait(timeout=100)
    except subprocess.TimeoutExpired:k.TerminateJobObject(job,124);r['harness_timeout']=True;r['harness_exit_code']=harness.wait(timeout=3)
   time.sleep(2)
   r['hook_config_sha256_after']=hashlib.sha256(config.read_bytes()).hexdigest()
   r['hook_config_unchanged']=r['hook_config_sha256_after']==config_before
   r['claude_config_unchanged']=hashlib.sha256(claude_config.read_bytes()).hexdigest()==claude_hash
   send(255); r['shutdown_sent']=True
  try: r['exit_code']=p.wait(timeout=5)
  except subprocess.TimeoutExpired: k.TerminateJobObject(job,124); r['forced_stop']=True; r['exit_code']=p.wait(timeout=3)
  r['stage']='exited'
except Exception as e:
 r['error']=repr(e)
finally:
 if p is not None and p.poll() is None:
  k.TerminateJobObject(job,125)
  try:p.wait(timeout=3)
  except subprocess.TimeoutExpired:p.kill(); p.wait(timeout=3)
 if timer:timer.cancel()
 k.CloseHandle(job); sink.shutdown(); sink.server_close()
 r['harness_alive_after_cleanup']=harness.poll() is None if harness else False
 r['daemon_alive_after_cleanup']=p.poll() is None if p else False
 r['exit_code']=p.returncode if p else None; r['pipes_after_cleanup']=pipes(); r['requests']=requests; save()
 print('RECEIPT',run/'receipt.json'); print('STDERR:',(run/'stderr.txt').read_text(errors='replace') if (run/'stderr.txt').exists() else '')
