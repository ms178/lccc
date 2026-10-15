import pathlib,json,subprocess,hashlib,sys
R=pathlib.Path('/home/user/.cache/redteam103'); old=json.loads(pathlib.Path('/home/user/.cache/redteam94/performance97/native45.json').read_text()); rows=[]
sha=lambda p:hashlib.sha256(pathlib.Path(p).read_bytes()).hexdigest()
report={'complete':False,'metric':'whole-ELF identity and native output correctness; NOT new timing measurements','base':'2f477ce01749824ccab73ead7ccac90abd77fa08','candidate':subprocess.check_output(['git','-C','/home/user/lccc','rev-parse','HEAD'],text=True).strip(),'added_common_flag':'-std=gnu17','rows':rows}
for row in old['benchmarks']:
 item={'name':row['name'],'arms':{}};rows.append(item)
 for arm in ['lccc','main','gcc','clang','icx']:
  cmd=row['compile']['lccc' if arm=='main' else arm]['command'].copy()
  if arm in ['lccc','main']:cmd[0]=str(R/('candidate' if arm=='lccc' else 'main2f47'))
  cmd.insert(1,'-std=gnu17'); dest=R/'native45'/row['name']/arm/'benchmark.bin';dest.parent.mkdir(parents=True,exist_ok=True);cmd[cmd.index('-o')+1]=str(dest)
  p=subprocess.run(cmd,capture_output=True,text=True,timeout=180)
  rec={'command':cmd,'exit':p.returncode,'stdout':p.stdout,'stderr':p.stderr};item['arms'][arm]=rec
  assert p.returncode==0,rec
  rec['sha256']=sha(dest);rec['prior_timed_sha256']=row['compile']['lccc' if arm=='main' else arm]['binary_sha256'];rec['identical_to_prior_timed_candidate' if arm=='main' else 'identical_to_prior_timed_arm']=rec['sha256']==rec['prior_timed_sha256']
  run=subprocess.run([str(dest)],capture_output=True,text=True,timeout=180);rec['run']={'exit':run.returncode,'stdout':run.stdout,'stderr':run.stderr};rec['correct']=run.returncode==0 and run.stdout.strip()==row['runs']['gcc'][0]['stdout'].strip();assert rec['correct'],rec
 item['candidate_main_identical']=item['arms']['lccc']['sha256']==item['arms']['main']['sha256']
 (R/'identity.json').write_text(json.dumps(report,indent=2));print(row['name'],'candidate/main identical:',item['candidate_main_identical'],'prior identities:',{k:next(v for key,v in a.items() if key.startswith('identical_to')) for k,a in item['arms'].items()},flush=True)
report['complete']=True;report['summary']={'candidate_main_identical':sum(x['candidate_main_identical'] for x in rows),'correct_arms':sum(a['correct'] for x in rows for a in x['arms'].values()),'prior_identity_counts':{k:sum(next(v for key,v in x['arms'][k].items() if key.startswith('identical_to')) for x in rows) for k in ['lccc','main','gcc','clang','icx']}}
(R/'identity.json').write_text(json.dumps(report,indent=2));print(report['summary'],flush=True)
