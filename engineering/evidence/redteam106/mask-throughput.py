import pathlib,subprocess,json,sys,random,hashlib
R=pathlib.Path('/home/user/.cache/redteam106');sys.path.insert(0,'/home/user/lccc');from tests.benchmark import run_benchmarks as b;from tools.corpus import publication
cc={'candidate':'/home/user/.cache/redteam104/candidate','main':'/home/user/.cache/redteam103/main2f47','gcc':'/usr/bin/gcc-16','clang':'/usr/bin/clang-23','icx':'/home/user/.cache/redteam94/intel/opt/intel/oneapi/compiler/2026.1/bin/icx'};report={'complete':False,'scope':'Focused outlined mask32/mask64 helper throughput; 40 million fixed iterations; not a broad application speedup claim','compiler_versions':{},'builds':{},'warmups':[],'rounds':[]};commands={}
for arm,compiler in cc.items():
 dest=R/('throughput-'+arm);cmd=[compiler,'-std=gnu17','-O2','-march=x86-64-v3','-fwrapv','-fno-fast-math','-ffp-contract=off','-Werror',str(R/'mask_throughput.c'),'-o',str(dest)];p=subprocess.run(cmd,capture_output=True,text=True,timeout=120);assert p.returncode==0 and not p.stderr,(cmd,p.stderr);report['builds'][arm]={'command':cmd,'exit':p.returncode,'stderr':p.stderr,'sha256':hashlib.sha256(dest.read_bytes()).hexdigest()};report['compiler_versions'][arm]=subprocess.check_output([compiler,'--version'],text=True);commands[arm]=['setarch','x86_64','-R','taskset','-c','0',str(dest),'106178']
env=b.controlled_environment();reference=b.invoke(commands['gcc'],timeout_s=90,env=env);assert reference['ok'];report['reference']=reference
for _ in range(2):
 for arm,cmd in commands.items():
  q=b.invoke(cmd,timeout_s=90,env=env);assert q['ok'] and q['stdout']==reference['stdout'];report['warmups'].append({'arm':arm,**q})
rng=random.Random(107178)
for n in range(31):
 order=list(commands);rng.shuffle(order);runs={}
 for arm in order:
  q=b.invoke(commands[arm],timeout_s=90,env=env);assert q['ok'] and q['stdout']==reference['stdout'];runs[arm]=q
 report['rounds'].append({'round':n,'order':order,'runs':runs});publication.atomic_bytes(R/'mask-throughput.json',(json.dumps(report,indent=2)+'\n').encode())
report['ratios']={ref:{metric:b.summarize([x['runs']['candidate'][metric]/x['runs'][ref][metric] for x in report['rounds']],107178) for metric in ['wall_s','child_cpu_s']} for ref in cc if ref!='candidate'};report['complete']=True;publication.atomic_bytes(R/'mask-throughput.json',(json.dumps(report,indent=2)+'\n').encode());print({ref:{k:(v['median'],v['median_ci95_bootstrap']) for k,v in m.items()} for ref,m in report['ratios'].items()},flush=True)
