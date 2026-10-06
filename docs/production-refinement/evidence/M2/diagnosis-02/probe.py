import os,sys,json,time,uuid,hashlib,subprocess,threading,struct,ctypes,http.server
from pathlib import Path
from ctypes import wintypes as W
base=Path(os.environ['TEMP'])/'aob-m2-20261006'
run=base/('probe-'+uuid.uuid4().hex[:10]); run.mkdir()
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
requests=[]
class Sink(http.server.BaseHTTPRequestHandler):
 def do_POST(self):
  size=int(self.headers.get('Content-Length','0')); assert 0<=size<=2097152
  body=self.rfile.read(size); name='request-%02d.bin'%len(requests); (run/name).write_bytes(body)
  requests.append({'path':self.path,'bytes':len(body),'sha256':hashlib.sha256(body).hexdigest(),'file':name})
  self.send_response(200); self.send_header('Content-Type','application/x-protobuf'); self.send_header('Content-Length','0'); self.end_headers()
 def log_message(self,*args): pass
sink=http.server.ThreadingHTTPServer(('127.0.0.1',0),Sink)
threading.Thread(target=sink.serve_forever,daemon=True).start()
env={n:os.environ[n] for n in ['SystemRoot','WINDIR'] if n in os.environ}; env['PATH']=os.path.join(os.environ['SystemRoot'],'System32')
for n in ['USERPROFILE','HOME','LOCALAPPDATA','APPDATA','TEMP','TMP']:
 d=run/n; d.mkdir(); env[n]=str(d)
env.update(AGENT_OTEL_PIPE=pipe,AGY_OTEL_PIPE=pipe,OTEL_EXPORTER_OTLP_ENDPOINT='http://127.0.0.1:'+str(sink.server_port),OTEL_SERVICE_NAME='agent-otel-m2-isolated',AGENT_OTEL_IDLE_TIMEOUT_SECS='120',AGENT_OTEL_QUOTA_INTERVAL_SECS='60',AGENT_OTEL_QUOTA_FILE=str(run/'absent-quota.json'))
r={'started_utc':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),'exe':str(exe),'sha256':expected,'supervisor_pid':os.getpid(),'pipe':pipe,'endpoint':env['OTEL_EXPORTER_OTLP_ENDPOINT'],'env':env,'run':str(run),'requests':requests,'stage':'intent'}
def save(): (run/'receipt.json').write_text(json.dumps(r,indent=2),encoding='utf-8')
save(); p=None; timer=None

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
  timer=threading.Timer(20,lambda:k.TerminateJobObject(job,124)); timer.start()
  nt=ctypes.WinDLL('ntdll'); nt.NtResumeProcess.argtypes=[W.HANDLE]; assert nt.NtResumeProcess(W.HANDLE(int(p._handle)))==0
  r['stage']='running'; save()
  deadline=time.monotonic()+6
  while time.monotonic()<deadline and p.poll() is None and pipe.split('\\')[-1] not in pipes(): time.sleep(.05)
  r['pipes_after_start']=pipes()
  assert not any(n in ['agent-otel','agy-otel'] for n in r['pipes_after_start']), 'Unexpected default pipe'
  if p.poll() is None and pipe.split('\\')[-1] in r['pipes_after_start']:
   r['pipe_owner_pid']=send(3)
   r['sent_clients']=[]
   for cid,name in [(1,'antigravity'),(2,'claude'),(3,'codex'),(4,'grok')]:
    payload=json.dumps({'conversationId':'m2-synthetic-'+name,'stepIdx':1,'toolCall':{'name':'read_file','id':'m2-'+name}}).encode()
    send(1,bytes([3])+struct.pack('<H',cid)+payload); r['sent_clients'].append(name)
   time.sleep(1)
   r['tcp_sample']=subprocess.run([os.path.join(os.environ['SystemRoot'],'System32','netstat.exe'),'-ano','-p','tcp'],capture_output=True,text=True,timeout=2).stdout.splitlines()
   r['tcp_sample']=[line.strip() for line in r['tcp_sample'] if line.split() and line.split()[-1]==str(p.pid)]
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
 r['exit_code']=p.returncode if p else None; r['pipes_after_cleanup']=pipes(); r['requests']=requests; save()
 print(json.dumps(r,indent=2)); print('STDERR:',(run/'stderr.txt').read_text(errors='replace') if (run/'stderr.txt').exists() else '')
