#!/usr/bin/env python3
"""Five alternating default-size compiler walltime pairs and bounded Callgrind pairs.
No performance threshold: reports walltime separately from simulated program Ir.
The compiler walltime includes its assembler/linker children; Ir is PROGRAM Ir,
not compiler instructions or native cycles. Raw failures and receipts are kept.
"""
import argparse, hashlib, importlib.util, json, os, platform, shutil, statistics
import subprocess, sys, time
from pathlib import Path

p=argparse.ArgumentParser(description=__doc__)
p.add_argument('--repo',type=Path,required=True)
p.add_argument('--candidate',type=Path,required=True)
p.add_argument('--baseline',type=Path,required=True)
p.add_argument('--baseline-repo',type=Path,required=True)
p.add_argument('--out',type=Path,required=True)
p.add_argument('--compile-batch',type=int,default=5)
a=p.parse_args();a.repo=a.repo.resolve();a.out.mkdir(parents=True,exist_ok=False)
sys.path.insert(0,str(a.repo/'scripts'));import callgrind_ab as cg
sys.path.insert(0,str(a.repo/'tests/benchmark'));import run_benchmarks as bench
assert len(bench.BENCHMARKS)==45

def sha(path):
    with Path(path).open('rb') as stream:return hashlib.file_digest(stream,'sha256').hexdigest()
def git(repo,*args):return subprocess.check_output(['git','-C',str(repo),*args],text=True).strip()
def identity():
    tools={}
    for name in ('gcc-16','as','ld','mold','ld.lld','valgrind','rustc','objdump'):
        exe=Path(shutil.which(name)).resolve()
        tools[name]={'path':str(exe),'sha256':sha(exe),'version':subprocess.check_output([name,'--version'],text=True).splitlines()[0]}
    include=Path(subprocess.check_output(['gcc-16','-print-file-name=include'],text=True).strip())
    system=hashlib.sha256()
    for header in sorted(Path('/usr/include').rglob('*')):
        if header.is_file():system.update(str(header).encode()+b'\0'+sha(header).encode()+b'\n')
    rustc=Path(subprocess.check_output(['rustup','which','rustc'],text=True).strip())
    tools['rustc_actual']={'path':str(rustc),'sha256':sha(rustc)}
    return {'system_header_tree_sha256':system.hexdigest(),'candidate':{'source_commit':git(a.repo,'rev-parse','HEAD'),'source_tree':git(a.repo,'rev-parse','HEAD^{tree}'),'status':git(a.repo,'status','--porcelain'),'binary':str(a.candidate),'binary_sha256':sha(a.candidate)},
            'baseline':{'source_commit':git(a.baseline_repo,'rev-parse','HEAD'),'source_tree':git(a.baseline_repo,'rev-parse','HEAD^{tree}'),'status':git(a.baseline_repo,'status','--porcelain'),'binary':str(a.baseline),'binary_sha256':sha(a.baseline)},
            'controller_sha256':sha(__file__),'tools':tools,'builtin_headers':{str(f.relative_to(include)):sha(f) for f in sorted(include.rglob('*')) if f.is_file()},'platform':platform.platform()}
def save(name,data):(a.out/name).write_text(json.dumps(data,indent=2)+'\n')
original=identity();assert not original['candidate']['status'] and not original['baseline']['status'];save('identities-before.json',original)
manifest={'complete':False,'cpu_affinity':sorted(os.sched_getaffinity(0)),'background_note':'Long baseline stress Callgrind was pinned to CPU0; this controller and children use CPU1. Shared-VM measurements, not isolated hardware timing.','pairs':5,'order':'candidate first on odd rounds, baseline first on even rounds','compiler_cost':f'default sizes; 5 alternating paired batches, {a.compile_batch} invocations per batch; median of batch means; native driver+assembler+linker walltime; all samples retained','program_Ir':'separate bounded sizes from checked-in callgrind_sizes.json; fixed cache geometry; NOT compiler Ir','compiler_Ir':'additional separate default-size compiler-parent Callgrind run; assembler/linker children NOT traced','compiler_flags':'-O2 -march=x86-64-v3 -fwrapv -Wl,--no-as-needed -lm (no size macros)','rows':[],'failures':[]}
save('costs.json',manifest)
include=cg.gcc_include('gcc-16')
for round_no in range(1,6):
    order=[('candidate',a.candidate),('baseline',a.baseline)]
    if round_no%2==0:order.reverse()
    print('DEFAULT COMPILATION ROUND',round_no,flush=True)
    for item in bench.BENCHMARKS:
        row={'round':round_no,'name':item.name,'arms':{}}
        for position,(name,compiler) in enumerate(order):
            directory=a.out/f'default-{round_no:02}'/item.name/('aa' if position==0 else 'bb');directory.mkdir(parents=True)
            flags='-O2 -march=x86-64-v3 -fwrapv -Wl,--no-as-needed -lm'
            records=[]
            for repetition in range(a.compile_batch):
                run_dir=directory/f'repeat-{repetition:02}';run_dir.mkdir()
                output=run_dir/'program'
                error=cg.compile_program(compiler,flags,bench.source_for(item),output,include=include)
                receipt=cg.process_receipt(str(output)+'.compile')
                record={'receipt':receipt,'error':error}
                if error:manifest['failures'].append({'phase':'compile','round':round_no,'name':item.name,'arm':name,'repetition':repetition,'error':error})
                else:record['elf_sha256']=sha(output)
                records.append(record)
            row['arms'][name]={'records':records,'batch_mean_seconds':statistics.mean(r['receipt']['process']['elapsed'] for r in records) if not any(r['error'] for r in records) else None}
        manifest['rows'].append(row);save('costs.json',manifest)
    print('BOUNDED PROGRAM CALLGRIND ROUND',round_no,flush=True)
    command=[sys.executable,str(a.repo/'scripts/callgrind_corpus.py'),'--mine',str(order[0][1]),'--ref',str(order[1][1]),'--gcc-include-command','gcc-16','--out',str(a.out/f'bounded-{round_no:02}')]
    rc=subprocess.run(command).returncode
    if rc:manifest['failures'].append({'phase':'corpus','round':round_no,'returncode':rc})
    save('costs.json',manifest)
print('DEFAULT-SIZE COMPILER-PARENT CALLGRIND',flush=True)
compiler_ir={}
for item in bench.BENCHMARKS:
    compiler_ir[item.name]={}
    for name,compiler in [('candidate',a.candidate),('baseline',a.baseline)]:
        directory=a.out/'compiler-Ir'/item.name/('aa' if name=='candidate' else 'bb');directory.mkdir(parents=True)
        output=directory/'program';counts=directory/'compiler.cg'
        command=['valgrind','--tool=callgrind','--quiet','--cache-sim=yes','--branch-sim=yes','--trace-children=no',
                 '--I1=32768,8,64','--D1=32768,8,64','--LL=33554432,16,64','--callgrind-out-file='+str(counts),
                 str(compiler),include,'-O2','-march=x86-64-v3','-fwrapv',str(bench.source_for(item)),'-Wl,--no-as-needed','-lm','-o',str(output)]
        result,info=cg.run_recorded(command,600,str(directory/'compiler-callgrind'))
        if result['returncode'] or result['failure']:raise RuntimeError(f'compiler instrumentation failed: {item.name} {name}')
        compiler_ir[item.name][name]={'events':cg.parse_summary(counts),'receipt':info,'counter_sha256':sha(counts),'elf_sha256':sha(output)}
    print('compiler Ir',item.name,flush=True);save('compiler-Ir.json',compiler_ir)
summary=[]
for item in bench.BENCHMARKS:
    rows=[r for r in manifest['rows'] if r['name']==item.name]
    times={arm:[r['arms'][arm]['batch_mean_seconds'] for r in rows if r['arms'][arm]['batch_mean_seconds'] is not None] for arm in ('candidate','baseline')}
    row={'name':item.name,'batch_mean_seconds':times,'median_seconds':{k:statistics.median(v) for k,v in times.items() if v},'program_Ir_ratios':[],'bounded_ELF_equal':[],
         'compiler_parent_Ir_ratio':compiler_ir[item.name]['candidate']['events']['Ir']/compiler_ir[item.name]['baseline']['events']['Ir']}
    if all(len(v)==5 for v in times.values()):row['compiler_median_ratio']=row['median_seconds']['candidate']/row['median_seconds']['baseline']
    for round_no in range(1,6):
        doc=json.loads((a.out/f'bounded-{round_no:02}'/item.name/'manifest.json').read_text())
        result=doc['results'][bench.source_for(item).stem]
        if result['status']=='COMPARABLE-SIMULATION':
            ratio=result['metrics']['ir_ratio'];row['program_Ir_ratios'].append(ratio if round_no%2 else 1/ratio)
            row['bounded_ELF_equal'].append(result['binary_sha256']['mine']==result['binary_sha256']['ref'])
    summary.append(row)
manifest['complete']=True;save('costs.json',manifest);save('summary.json',summary)
final=identity();save('identities-after.json',final)
assert final==original,'Source/tool/binary/controller identity changed during measurements'
print('DONE',len(summary),'workloads; failures:',len(manifest['failures']),flush=True)
sys.exit(bool(manifest['failures']))
