"""Paired, output-checked real zstd API timings; shared GCC driver, differing libraries.
No speed claims from CLI fixed-time -b wall duration. Validate every iteration
outside its timed region, plus decode every emitted frame with reference CLI.
"""
import argparse, hashlib, json, math, os, random, statistics, subprocess, sys
from pathlib import Path
p=argparse.ArgumentParser(description=__doc__)
p.add_argument('--repo',type=Path,required=True);p.add_argument('--source',type=Path,required=True)
p.add_argument('--driver',action='append',required=True,help='name=executable; gcc required')
p.add_argument('--reference-cli',required=True);p.add_argument('--out',type=Path,required=True)
p.add_argument('--reps',type=int,default=9);p.add_argument('--size',type=int,default=1048576)
p.add_argument('--corpora');p.add_argument('--options');p.add_argument('--baseline')
a=p.parse_args();sys.path.insert(0,str(a.repo));from tests.benchmark import run_benchmarks as b
from tools.corpus import publication
if a.reps<3 or a.size<4096:p.error('at least three pairs and 4096 bytes required')
drivers=dict(v.split('=',1) for v in a.driver)
if len(drivers)!=len(a.driver) or 'gcc' not in drivers:p.error('unique arms and gcc required')
a.out.mkdir(parents=True,exist_ok=True);data=a.out/'inputs';data.mkdir(exist_ok=True)
rng=random.Random(178)
def scale(x):return (x*((a.size+len(x)-1)//len(x)))[:a.size]
sources=b'\n'.join(x.read_bytes() for x in sorted((a.source/'lib').rglob('*.c')))
rows=b'\n'.join(json.dumps({'id':i,'name':f'user{i%137}','value':rng.randrange(10000)}).encode() for i in range(20000))
inputs={'source':scale(sources),'json':scale(rows),'random':rng.randbytes(a.size),'rle':b'A'*a.size,'duplicates':scale(sources[:65536]+rng.randbytes(1024)),'elf':scale(Path('/usr/bin/bash').read_bytes())}
if a.corpora:
 selected=set(a.corpora.split(','))
 if not selected or selected-set(inputs):p.error('unknown corpus')
 inputs={k:v for k,v in inputs.items() if k in selected}
for name,value in inputs.items():(data/name).write_bytes(value)
options=[('fast5',-5,0,0,1),('level1',1,0,0,1),('level3',3,0,0,1),('level9',9,0,0,1),('level19',19,0,0,1),('long20',5,0,1,1),('threads2',3,2,0,1),('no_checksum',3,0,0,0)]
if a.options:
 selected=set(a.options.split(','))
 if not selected or selected-{x[0] for x in options}:p.error('unknown option')
 options=[x for x in options if x[0] in selected]
if a.baseline and a.baseline not in drivers:p.error('unknown baseline')
refs=list(dict.fromkeys(['gcc']+([a.baseline] if a.baseline else [])))
cpus=sorted(os.sched_getaffinity(0));report={'complete':False,'metric':'in-process real zstd API ns, shared GCC16 driver; not CLI startup throughput','reps':a.reps,'size':a.size,'cpus':cpus,'drivers':{k:{'path':v,'sha256':b.sha256_file(Path(v))} for k,v in drivers.items()},'inputs':{k:hashlib.sha256(v).hexdigest() for k,v in inputs.items()},'cases':[],'failures':[]}
def save():publication.atomic_bytes(a.out/'matrix.json',(json.dumps(report,indent=2)+'\n').encode())
def run(name,src,opt,count,decode,frame,out):
 _,level,workers,ldm,checksum=opt
 pin=','.join(map(str,cpus[:2] if workers else cpus[:1]))
 cmd=['taskset','-c',pin,drivers[name],str(src),str(level),str(workers),str(ldm),str(checksum),str(count),str(int(decode)),str(frame),str(out)]
 x=subprocess.run(cmd,capture_output=True,timeout=120)
 if x.returncode:raise RuntimeError(f'{name} rc={x.returncode}: {x.stderr[-1000:]!r}')
 values=list(map(int,x.stdout.split()))
 if len(values)!=4 or values[0]<=0 or values[1]!=count or values[2]!=a.size:raise ValueError('invalid driver result')
 return {'command':cmd,'ns':values[0],'iterations':count,'output_bytes':values[3]}
save()
for name in inputs:
 for opt in options:
  key=name+'-'+opt[0];wd=a.out/key;wd.mkdir(exist_ok=True);src=data/name;frame=wd/'gcc.zst'
  try:
   for arm in ['gcc',*[x for x in drivers if x!='gcc']]:
    dest=wd/(arm+'.zst');run(arm,src,opt,1,False,frame,dest)
    decoded=subprocess.run([a.reference_cli,'-q','-d','-c',str(dest)],capture_output=True,timeout=30)
    if decoded.returncode or decoded.stdout!=inputs[name]:raise ValueError('reference cross-decode mismatch '+arm)
   for decode in [False,True]:
    row={'case':key,'decode':decode,'frames':{arm:b.sha256_file(wd/(arm+'.zst')) for arm in drivers},'raw':[],'ratios':{}}
    pilot=[run(arm,src,opt,3,decode,frame,wd/(arm+'.zst'))['ns']/3 for arm in drivers]
    count=min(10000,max(1,math.ceil(80000000/min(pilot))))
    for arm in drivers:run(arm,src,opt,count,decode,frame,wd/(arm+'.zst'))
    for rep in range(a.reps):
     order=list(drivers);rng.shuffle(order);samples={}
     for arm in order:samples[arm]=run(arm,src,opt,count,decode,frame,wd/(arm+'.zst'))
     row['raw'].append({'round':rep,'order':order,'samples':samples})
    for arm in drivers:
     for ref in refs:
      if arm==ref:continue
      vals=[q['samples'][arm]['ns']/q['samples'][ref]['ns'] for q in row['raw']]
      row['ratios'][arm+'/'+ref]=b.summarize(vals,178)
     if not decode:
      decoded=subprocess.run([a.reference_cli,'-q','-d','-c',str(wd/(arm+'.zst'))],capture_output=True,timeout=30)
      if decoded.returncode or decoded.stdout!=inputs[name]:raise ValueError('post-timing cross-decode mismatch '+arm)
    row['below_20ms_floor']=any(q['samples'][arm]['ns']<20000000 for q in row['raw'] for arm in drivers)
    report['cases'].append(row);save();print(key,'decompress' if decode else 'compress', {k:round(v['median'],4) for k,v in row['ratios'].items()},flush=True)
  except Exception as e:
   report['failures'].append({'case':key,'error':str(e)});save();print('FAIL',key,str(e),flush=True)
report['aggregates']={}
for arm in drivers:
 for ref in refs:
  if arm==ref:continue
  key=arm+'/'+ref
  for decode in [False,True]:
   rows=[x for x in report['cases'] if x['decode']==decode and not x['below_20ms_floor']]
   vals=[x['ratios'][key]['median'] for x in rows]
   report['aggregates'][key+('/decompress' if decode else '/compress')]={'n':len(vals),'gm':math.exp(statistics.mean(map(math.log,vals))) if vals else None,'am':statistics.mean(vals) if vals else None,'best':min(vals) if vals else None,'worst':max(vals) if vals else None}
report['complete']=len(report['cases'])==len(inputs)*len(options)*2 and not report['failures'];save()
raise SystemExit(not report['complete'])
