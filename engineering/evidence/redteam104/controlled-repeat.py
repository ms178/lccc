import pathlib,json,subprocess,random,math,sys,hashlib
R=pathlib.Path('/home/user/.cache/redteam104');sys.path.insert(0,'/home/user/lccc');from tests.benchmark import run_benchmarks as b;from tools.corpus import publication
screen=json.loads((R/'zstd96/matrix.json').read_text());rng=random.Random(105178);report={'complete':False,'scope':'Independent 31-pair confirmation of preidentified source/L3 gain and random/long20 regression, with ASLR disabled and a byte-identical baseline control','target_ns':800000000,'reps':31,'rows':[]}
base=pathlib.Path('/home/user/.cache/redteam103/driver-main');control=pathlib.Path('/home/user/.cache/redteam103/driver-candidate');assert base.read_bytes()==control.read_bytes();report['identical_control_sha256']=hashlib.sha256(base.read_bytes()).hexdigest()
def save():publication.atomic_bytes(R/'controlled-repetitions31.json',(json.dumps(report,indent=2)+'\n').encode())
def run(cmd,n):
 cmd=cmd.copy();cmd[9]=str(n);cmd=['setarch','x86_64','-R',*cmd];p=subprocess.run(cmd,capture_output=True,text=True,timeout=120);assert p.returncode==0,(cmd,p.returncode,p.stderr);v=list(map(int,p.stdout.split()));assert len(v)==4 and v[0]>0 and v[1]==n and v[2]==1048576
 return {'command':cmd,'ns':v[0],'iterations':v[1],'input_bytes':v[2],'output_bytes':v[3],'stderr':p.stderr}
save()
for name in ['source-level3','random-long20']:
 old=next(x for x in screen['cases'] if x['case']==name and not x['decode']);commands={k:old['raw'][0]['samples'][k]['command'].copy() for k in ['candidate','main']};commands['control']=commands['main'].copy();commands['control'][3]=str(control)
 pilots={k:run(c,3) for k,c in commands.items()};count=min(10000,max(1,math.ceil(report['target_ns']/min(p['ns']/3 for p in pilots.values()))));row={'case':name,'decode':False,'pilots':pilots,'warmups':[],'rounds':[]};report['rows'].append(row)
 for _ in range(2):
  for arm,c in commands.items():row['warmups'].append({'arm':arm,**run(c,count)})
 for i in range(31):
  order=list(commands);rng.shuffle(order);runs={k:run(commands[k],count) for k in order};row['rounds'].append({'round':i,'order':order,'runs':runs});save()
 row['ratios']={arm+'/main':b.summarize([x['runs'][arm]['ns']/x['runs']['main']['ns'] for x in row['rounds']],105178) for arm in ['candidate','control']};row['frame_hashes_unchanged']={k:hashlib.sha256(pathlib.Path(c[-1]).read_bytes()).hexdigest()==old['frames']['main' if k=='control' else k] for k,c in commands.items()};assert all(row['frame_hashes_unchanged'].values());save();print(name,{k:(v['median'],v['median_ci95_bootstrap']) for k,v in row['ratios'].items()},flush=True)
report['complete']=True;save();print('CONTROLLED COMPLETE',flush=True)
