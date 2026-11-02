import subprocess,json,pathlib,sys,hashlib
r=pathlib.Path('/home/user/.cache/redteam106');sys.path.insert(0,'/home/user/lccc');from tests.benchmark import run_benchmarks as b;from tools.corpus import publication
report={'complete':False,'scope':'Within-process diagnostic: symbol-isolated unchanged compiler-generated libraries; same GCC driver/input/output buffers, randomized arm order, excluded per-switch warm-up, independent GCC decode every call; base and control execute the same baseline code with separate contexts. Not pooled with separate-process matrix timings.','binary_sha256':hashlib.sha256((r/'within-process').read_bytes()).hexdigest(),'source_sha256':hashlib.sha256((r/'within_process.c').read_bytes()).hexdigest(),'rows':[]}
for case,level,ldm in [('source',3,0),('random',5,1)]:
 cmd=['setarch','x86_64','-R','taskset','-c','0',str(r/'within-process'),f'/home/user/.cache/redteam104/zstd96/inputs/{case}',str(level),str(ldm),'31','106178'];p=subprocess.run(cmd,capture_output=True,text=True,timeout=300);(r/(case+'.stdout')).write_text(p.stdout);assert p.returncode==0,(cmd,p.returncode,p.stderr)
 row={'case':case,'level':level,'long20':bool(ldm),'command':cmd,'exit':p.returncode,'stderr':p.stderr,'rounds':[]};report['rows'].append(row)
 for i,line in enumerate(p.stdout.splitlines()):
  a=list(map(int,line.split()));assert len(a)==11 and a[0]==i and sorted(a[1:4])==[0,1,2] and a[4]>0 and min(a[5:])>0
  row['rounds'].append({'round':i,'order':a[1:4],'iterations':a[4],'main_ns':a[5],'candidate_ns':a[6],'control_ns':a[7],'output_bytes':a[8:]})
 assert len(row['rounds'])==31
 row['ratios']={arm+'/main':b.summarize([x[arm+'_ns']/x['main_ns'] for x in row['rounds']],106178) for arm in ['candidate','control']};publication.atomic_bytes(r/'within-process.json',(json.dumps(report,indent=2)+'\n').encode());print(case,{k:(v['median'],v['median_ci95_bootstrap']) for k,v in row['ratios'].items()},flush=True)
report['complete']=True;publication.atomic_bytes(r/'within-process.json',(json.dumps(report,indent=2)+'\n').encode())
