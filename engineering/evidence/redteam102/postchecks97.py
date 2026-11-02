import pathlib,json,subprocess,hashlib,sys,random
repo=pathlib.Path('/home/user/lccc');r=pathlib.Path('/home/user/.cache/redteam94');out=r/'performance97/followup';sys.path.insert(0,str(repo));from tests.benchmark import run_benchmarks as b
cc=r/'arms/s98';rows=[]
for case in json.loads((r/'semantic65/index.json').read_text())['cases']:
 cmd=case['arms']['candidate']['build']['command'].copy();cmd[0]=str(cc);binary=out/(case['id'].replace('/','-')+'-current');cmd[cmd.index('-o')+1]=str(binary);p=subprocess.run(cmd,capture_output=True,text=True,timeout=120);row={'id':case['id'],'command':cmd,'build_exit':p.returncode,'stderr':p.stderr}
 if p.returncode==0:
  q=subprocess.run([str(binary),*case['arguments']],capture_output=True,timeout=30);row.update(run_exit=q.returncode,stdout_sha256=hashlib.sha256(q.stdout).hexdigest());row['pass']=q.returncode==0 and row['stdout_sha256']==case['arms']['gcc']['run']['stdout_sha256'] and not p.stderr
 else:row['pass']=False
 rows.append(row)
(out/'current-semantic65.json').write_text(json.dumps(rows,indent=2));print('current semantic',sum(x['pass'] for x in rows),len(rows),flush=True);assert all(x['pass'] for x in rows)
for arm in ['candidate','main']:
 cmd=['valgrind','--tool=callgrind','--error-exitcode=99','--log-file='+str(out/('rle-'+arm+'.txt')),'--callgrind-out-file='+str(out/('rle-'+arm+'.callgrind')),str(r/f'zstd97/driver-{arm}'),str(r/'performance97/zstd96/inputs/rle'),'5','0','1','1','4','0','/dev/null',str(out/('rle-'+arm+'.zst'))];p=subprocess.run(cmd,capture_output=True,timeout=180);assert p.returncode==0;(out/('rle-'+arm+'-profile.json')).write_text(json.dumps({'command':cmd,'exit':p.returncode,'stdout':p.stdout.decode(),'stderr':p.stderr.decode()},indent=2))
data=json.loads((r/'performance97/native45.json').read_text());case=next(x for x in data['benchmarks'] if x['name']=='fannkuch');cmd=case['compile']['lccc']['command'].copy();cmd=['-fno-wrapv' if x=='-fwrapv' else x for x in cmd];binary=out/'fannkuch-no-wrapv';cmd[cmd.index('-o')+1]=str(binary);p=subprocess.run(cmd,capture_output=True,text=True);assert p.returncode==0 and not p.stderr;commands={'wrapv':case['runs']['lccc'][0]['command'],'no_wrapv':[*case['runs']['lccc'][0]['command'][:-1],str(binary)]};expected=case['runs']['gcc'][0]['stdout'];samples=[];rng=random.Random(102178);env=b.controlled_environment()
for n in range(-2,9):
 order=list(commands);rng.shuffle(order);runs={}
 for arm in order:
  q=b.invoke(commands[arm],timeout_s=90,env=env);assert q['ok'] and q['stdout']==expected;runs[arm]=q
 samples.append({'round':n,'order':order,'runs':runs})
summary=b.summarize([x['runs']['no_wrapv']['wall_s']/x['runs']['wrapv']['wall_s'] for x in samples if x['round']>=0],102178)
(out/'fannkuch-flag-ablation.json').write_text(json.dumps({'scope':'Diagnostic flag ablation; changed arithmetic contract, NOT an accepted optimization or canonical performance comparison','command':cmd,'sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'rounds':samples,'paired_no_wrapv_over_wrapv':summary},indent=2));print('fann no-wrapv/wrapv',summary['median'],summary['median_ci95_bootstrap'],flush=True)
