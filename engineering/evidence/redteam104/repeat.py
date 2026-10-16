import pathlib,json,subprocess,random,math,sys,hashlib
R=pathlib.Path('/home/user/.cache/redteam104');sys.path.insert(0,'/home/user/lccc');from tests.benchmark import run_benchmarks as b;from tools.corpus import publication
screen=json.loads((R/'zstd96/matrix.json').read_text());special={('source-level3',False),('source-level9',False),('source-no_checksum',False),('duplicates-level9',False),('elf-level9',False)}
selected=[r for r in screen['cases'] if r['ratios']['candidate/main']['median']>1.01 or (r['case'],r['decode']) in special];report={'complete':False,'scope':'All 14 main-relative screen regressions >1%, three source compression hotspots and two apparent wins; selected follow-up, not a new full-matrix aggregate','target_ns':400000000,'reps':31,'rows':[]};rng=random.Random(104178)
def save():publication.atomic_bytes(R/'repetitions31.json',(json.dumps(report,indent=2)+'\n').encode())
def run(cmd,n):
 cmd=cmd.copy();cmd[9]=str(n);p=subprocess.run(cmd,capture_output=True,text=True,timeout=120);assert p.returncode==0,(cmd,p.returncode,p.stderr);v=list(map(int,p.stdout.split()));assert len(v)==4 and v[0]>0 and v[1]==n and v[2]==1048576
 return {'command':cmd,'ns':v[0],'iterations':v[1],'input_bytes':v[2],'output_bytes':v[3],'stderr':p.stderr}
save()
for old in selected:
 commands={k:old['raw'][0]['samples'][k]['command'] for k in ['candidate','main']};pilots={k:run(c,3) for k,c in commands.items()};count=min(10000,max(1,math.ceil(report['target_ns']/min(p['ns']/3 for p in pilots.values()))));row={'case':old['case'],'decode':old['decode'],'screen_ratio':old['ratios']['candidate/main'],'pilots':pilots,'warmups':[],'rounds':[]};report['rows'].append(row)
 for _ in range(2):
  for arm,c in commands.items():row['warmups'].append({'arm':arm,**run(c,count)})
 for i in range(31):
  order=list(commands);rng.shuffle(order);runs={k:run(commands[k],count) for k in order};row['rounds'].append({'round':i,'order':order,'runs':runs});save()
 row['paired']=b.summarize([x['runs']['candidate']['ns']/x['runs']['main']['ns'] for x in row['rounds']],104178)
 row['frame_hashes_unchanged']={k:hashlib.sha256(pathlib.Path(c[-1]).read_bytes()).hexdigest()==old['frames'][k] for k,c in commands.items()};assert all(row['frame_hashes_unchanged'].values());save();print(row['case'],row['decode'],row['paired']['median'],row['paired']['median_ci95_bootstrap'],flush=True)
report['complete']=True;save();print('COMPLETE',len(selected),flush=True)
