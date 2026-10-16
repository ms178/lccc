import pathlib,json,subprocess,random,sys
repo=pathlib.Path('/home/user/lccc');sys.path.insert(0,str(repo));from tests.benchmark import run_benchmarks as b
r=pathlib.Path('/home/user/.cache/redteam94/performance97');out=r/'followup';out.mkdir(exist_ok=True);data=json.loads((r/'zstd96/matrix.json').read_text());ranked=sorted(data['cases'],key=lambda x:x['ratios']['candidate/main']['median'],reverse=True);selected=ranked[:6]+ranked[-1:];records=[];rng=random.Random(101178);cli='/home/user/.cache/redteam94/zstd97/build-gcc/programs/zstd'
for case in selected:
 key=case['case']+('-D' if case['decode'] else '-C');row={'case':case['case'],'decode':case['decode'],'raw':[],'warmups':[]};commands={arm:case['raw'][0]['samples'][arm]['command'].copy() for arm in ['candidate','main']}
 for arm,cmd in commands.items():cmd[-1]=str(out/(key+'-'+arm+'.zst'))
 def run(arm):
  cmd=commands[arm];p=subprocess.run(cmd,capture_output=True,timeout=120);assert p.returncode==0 and not p.stderr,(cmd,p.stderr);v=list(map(int,p.stdout.split()));assert len(v)==4 and v[0]>0 and v[1]==int(cmd[9]) and v[2]==data['size'];return {'command':cmd,'ns':v[0],'iterations':v[1],'output_bytes':v[3]}
 for _ in range(2):
  for arm in commands:row['warmups'].append({'arm':arm,**run(arm)})
 for n in range(31):
  order=list(commands);rng.shuffle(order);samples={arm:run(arm) for arm in order};row['raw'].append({'round':n,'order':order,'samples':samples})
 if not case['decode']:
  for arm,cmd in commands.items():
   p=subprocess.run([cli,'-q','-d','-c',cmd[-1]],capture_output=True,timeout=30);assert p.returncode==0 and p.stdout==pathlib.Path(cmd[4]).read_bytes()
 vals=[x['samples']['candidate']['ns']/x['samples']['main']['ns'] for x in row['raw']];row['paired']=b.summarize(vals,101178);records.append(row);(out/'zstd-pairs31.json').write_text(json.dumps({'complete':False,'rows':records},indent=2));print(key,row['paired']['median'],row['paired']['median_ci95_bootstrap'],flush=True)
(out/'zstd-pairs31.json').write_text(json.dumps({'complete':True,'rows':records},indent=2))
