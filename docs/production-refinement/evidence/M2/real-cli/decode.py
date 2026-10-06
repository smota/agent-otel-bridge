import json,sys,hashlib
from pathlib import Path
run=Path(sys.argv[1])
def fields(b):
 i=0
 def vint():
  nonlocal i
  n=s=0
  while True:
   x=b[i];i+=1;n|=(x&127)<<s
   if x<128:return n
   s+=7
 while i<len(b):
  t=vint();w=t&7;f=t>>3
  if w==0:v=vint()
  elif w==1:v=b[i:i+8];i+=8
  elif w==2:n=vint();v=b[i:i+n];i+=n
  elif w==5:v=b[i:i+4];i+=4
  else:raise ValueError(w)
  yield f,v
allowed={'gen_ai.agent.name','agent.hook.event','gen_ai.conversation.id','gen_ai.tool.name','gen_ai.tool.call.id','gen_ai.request.model','gen_ai.provider.name','gen_ai.system','agent.context.state','agent.step.index'}
spans=[]
r=json.loads((run/'receipt.json').read_text())
for req in r['requests']:
 if req['path']!='/v1/traces':continue
 b=(run/req['file']).read_bytes();assert hashlib.sha256(b).hexdigest()==req['sha256']
 for f,rs in fields(b):
  if f!=1:continue
  for f,ss in fields(rs):
   if f!=2:continue
   for f,sp in fields(ss):
    if f!=2:continue
    rec={};attrs={}
    for f,v in fields(sp):
     if f in (1,2,4):rec[{1:'trace_id',2:'span_id',4:'parent_span_id'}[f]]=v.hex()
     elif f==5:rec['name']=v.decode()
     elif f==9:
      kv=dict(fields(v));key=kv[1].decode()
      if key not in allowed:continue
      av=dict(fields(kv.get(2,b'')))
      if 1 in av:attrs[key]=av[1].decode()
      elif 3 in av:attrs[key]=av[3]
    rec['attributes']=attrs;spans.append(rec)
(run/'span-summary.json').write_text(json.dumps(spans,indent=2),encoding='utf-8')
print(json.dumps(spans,indent=2))
