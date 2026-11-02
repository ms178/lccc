import pathlib,json,sys,subprocess,hashlib,random
repo=pathlib.Path('/home/user/lccc');sys.path.insert(0,str(repo));from tests.benchmark import run_benchmarks as b
r=pathlib.Path('/home/user/.cache/redteam94/performance97');out=r/'followup';out.mkdir(exist_ok=True);report=json.loads((r/'native45.json').read_text());qualified=json.loads((r/'native45-qualified.json').read_text());identity=[]
for row in report['benchmarks']:
 cmd=row['compile']['lccc']['command'].copy();cmd[0]='/home/user/.cache/redteam94/arms/integrated';binary=out/(row['name']+'-s96');cmd[cmd.index('-o')+1]=str(binary);p=subprocess.run(cmd,capture_output=True,text=True,timeout=120);oldhash=hashlib.sha256(binary.read_bytes()).hexdigest() if p.returncode==0 else None;current=pathlib.Path(row['runs']['lccc'][0]['command'][-1]);identity.append({'name':row['name'],'command':cmd,'exit':p.returncode,'stderr':p.stderr,'s96_sha256':oldhash,'current_sha256':hashlib.sha256(current.read_bytes()).hexdigest(),'identical':oldhash==hashlib.sha256(current.read_bytes()).hexdigest()})
(out/'identity.json').write_text(json.dumps(identity,indent=2));print('S96 byte-identical',sum(x['identical'] for x in identity),'/',len(identity),flush=True)
names=[x['name'] for x in qualified['rows'] if x['ratios']['main']['eligible'] and x['ratios']['main']['paired']['median']>1.01];names+=['nbody'];rows=[];env=b.controlled_environment();rng=random.Random(100178)
for name in names:
 row=next(x for x in report['benchmarks'] if x['name']==name);expected=row['runs']['gcc'][0]['stdout'];commands={k:row['runs'][k][0]['command'] for k in ['lccc','main']};item={'name':name,'rounds':[],'warmups':[]}
 for _ in range(2):
  for arm,cmd in commands.items():
   q=b.invoke(cmd,timeout_s=90,env=env);assert q['ok'] and q['stdout']==expected;q['arm']=arm;item['warmups'].append(q)
 for n in range(31):
  order=list(commands);rng.shuffle(order);runs={}
  for arm in order:
   q=b.invoke(commands[arm],timeout_s=90,env=env);assert q['ok'] and q['stdout']==expected;runs[arm]=q
  item['rounds'].append({'round':n,'order':order,'runs':runs})
 vals=[x['runs']['lccc']['wall_s']/x['runs']['main']['wall_s'] for x in item['rounds']];item['paired']=b.summarize(vals,100178);rows.append(item);(out/'pairs31.json').write_text(json.dumps({'complete':False,'rows':rows},indent=2));print(name,item['paired']['median'],item['paired']['median_ci95_bootstrap'],flush=True)
(out/'pairs31.json').write_text(json.dumps({'complete':True,'rows':rows},indent=2))
subprocess.run([sys.executable,'/home/user/.cache/redteam94/zstd-followup97.py'],check=True)
for arm in ['lccc','main']:
 binary=r/'native45/fannkuch'/arm/'benchmark.bin';cmd=['valgrind','--tool=callgrind','--error-exitcode=99','--log-file='+str(out/(arm+'-callgrind.txt')),'--callgrind-out-file='+str(out/(arm+'.callgrind')),str(binary)];p=subprocess.run(cmd,capture_output=True,timeout=180);(out/(arm+'-profile.json')).write_text(json.dumps({'command':cmd,'exit':p.returncode,'stdout':p.stdout.decode(),'stderr':p.stderr.decode()},indent=2));assert p.returncode==0
print('COMPLETE',flush=True)
