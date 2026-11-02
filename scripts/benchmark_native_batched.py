#!/usr/bin/env python3
"""Calibrated fresh-state 45-workload entry timing, excluding fork/exec startup.

Compile each original source once to the measured object. By default this is
a direct `-c` compilation, exactly what users get: LCCC's integrated assembler
then resolves its exact-span tight-loop alignment, which a `-S` text cannot
carry to another assembler. Assembly is still saved for inspection and the
historical `--object-mode assembly` protocol remains replayable. Rename only
the main symbol in that object without recompiling. A strict ELF
comparison requires all code/data/relocations and resolved symbol entries to
remain identical (except that one global entry name). This avoids recompiling
main as an ordinary function, which genuinely changed GCC's matmul epilogue.
Link with one unchanged GCC driver; no cross-TU inlining or LTO. Check every
repeated stdout against the original oracle. A common count targets the fastest
arm but is capped by the slowest; attained durations are reported explicitly.

Each measured entry runs in a fresh forked child, preserving pristine globals.
This measures entry execution (including its page faults), not process startup. Retain the ordinary
corpus runner separately for startup and single-invocation correctness. Raw
randomized paired samples, compiler/source identities and artifacts survive.
"""
from __future__ import annotations
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import random
import re
import shutil
import statistics
import subprocess
import sys

ROOT=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT/'tests/benchmark'))
import run_benchmarks as bench
sys.path.insert(0,str(ROOT/'scripts'))
from benchmark_elf_identity import object_identity, rename_entry


def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()


def publish(path,data):
    tmp=Path(str(path)+'.tmp');tmp.write_text(json.dumps(data,indent=2)+'\n');os.replace(tmp,path)


def execute(command,log,timeout=180):
    p=subprocess.run([str(x) for x in command],text=True,capture_output=True,timeout=timeout,
                     env=bench.controlled_environment())
    row=dict(command=[str(x) for x in command],returncode=p.returncode,stdout=p.stdout,stderr=p.stderr)
    publish(log,row)
    if p.returncode:raise ValueError(f'command failed: {log}')
    return row


def measured_object_commands(common, cc, source, assembly, obj, mode):
    """Commands producing the saved assembly and the measured object.

    `direct` measures `cc FLAGS -c source`; `assembly` measures the historical
    `cc FLAGS -S source; cc -c source.s` round trip. Both always save assembly.
    """
    if mode not in ('direct', 'assembly'):
        raise ValueError('unknown object mode: '+str(mode))
    emit = [*common, '-S', source, '-o', assembly]
    build = [*common, '-c', source, '-o', obj] if mode == 'direct' else [cc, '-c', assembly, '-o', obj]
    return emit, build


def calibrated_count(calibration, seconds, max_seconds):
    """A common count, bounded by the slowest arm as well as the fastest.

    Constant-folded workloads can differ by orders of magnitude. Never turn a
    sub-microsecond fastest arm into hours of work for an unfolded reference.
    Reports retain the cap and per-arm attained durations; short arms are not
    evidence for small percentage changes.
    """
    per_iteration = [v['ns'] / v['iterations'] for v in calibration.values()]
    desired = math.ceil(seconds * 1e9 / min(per_iteration))
    bounded = math.floor(max_seconds * 1e9 / max(per_iteration))
    return max(1, min(1000000, desired, bounded))


def validate_resume(old, identity, names, reference, rounds, seconds, max_seconds, pinning):
    """Accept only complete paired rows of the exact unchanged experiment.

    Older reports did not name their reference. Recompute every ratio from
    their raw samples instead of guessing a denominator from a label/median.
    Binary identities are independently rechecked by --reuse-build.
    """
    if (old.get('identity') != json.loads(json.dumps(identity))
            or old.get('rounds') != rounds or old.get('pinning') != pinning):
        raise ValueError('resume identity, rounds or CPU binding changed')
    rows = old.get('rows', [])
    if not isinstance(rows, list) or [r.get('name') for r in rows] != names[:len(rows)]:
        raise ValueError('resume requires an ordered completed prefix')
    keys = set(identity['compilers'])
    for row in rows:
        if (row.get('target_seconds') != seconds or row.get('max_batch_seconds') != max_seconds
                or len(row.get('samples', [])) != rounds
                or set(row.get('comparisons', {})) != keys):
            raise ValueError('resume timing policy or completed sample count changed')
        if type(row.get('iterations')) is not int or row['iterations'] <= 0:
            raise ValueError('invalid resumed iteration count')
        for sample in row['samples']:
            if set(sample.get('values', {})) != keys:
                raise ValueError('resume sample compiler set changed')
            for value in sample['values'].values():
                if (type(value.get('iterations')) is not int or value['iterations'] != row['iterations']
                        or type(value.get('ns')) is not int or value['ns'] <= 0):
                    raise ValueError('invalid resumed sample')
        for key in keys:
            ratios = [sample['values'][key]['ns'] / sample['values'][reference]['ns']
                      for sample in row['samples']]
            comparison = row['comparisons'][key]
            if comparison.get('paired_ratios') != ratios or comparison.get('median') != statistics.median(ratios):
                raise ValueError('resume reference or recorded ratios changed')
    return rows


def main():
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--compiler',action='append',required=True,help='label=executable')
    ap.add_argument('--reference',default='gcc')
    ap.add_argument('--gcc',default='gcc-16',help='neutral driver compiler and builtin-header provider')
    ap.add_argument('--out',type=Path,required=True)
    ap.add_argument('--only',help='comma-separated registry IDs')
    ap.add_argument('--rounds',type=int,default=7)
    ap.add_argument('--seconds',type=float,default=.2)
    ap.add_argument('--max-batch-seconds',type=float,default=3.)
    ap.add_argument('--cpu',default='auto')
    ap.add_argument('--flags',default='-O2 -march=x86-64-v3 -fwrapv -ffp-contract=off')
    ap.add_argument('--build-only',action='store_true')
    ap.add_argument('--reuse-build',action='store_true')
    ap.add_argument('--resume',action='store_true',help='resume an exactly matching completed prefix; implies --reuse-build')
    ap.add_argument('--publish-to',type=Path,help='atomically mirror each completed report row to persistent storage')
    ap.add_argument('--relink-driver',action='store_true',help='verify cached objects, rebuild only changed common driver and links')
    ap.add_argument('--object-mode',choices=('direct','assembly'),default='direct',
                    help='measured object: direct -c compilation (default, production path) or -S then assemble (historical)')
    args=ap.parse_args()
    if not __debug__:ap.error('assertions required')
    if args.resume and (args.relink_driver or args.build_only):ap.error('resume cannot relink or be build-only')
    if args.resume:args.reuse_build=True
    if args.rounds<3 or not math.isfinite(args.seconds) or args.seconds<=0:ap.error('invalid measurement settings')
    if not math.isfinite(args.max_batch_seconds) or args.max_batch_seconds < args.seconds:
        ap.error('maximum batch duration must be finite and at least the target duration')
    compilers={}
    for value in args.compiler:
        label,path=value.split('=',1)
        if not re.fullmatch(r'[A-Za-z0-9_-]+',label) or label in compilers:ap.error('invalid/duplicate compiler label')
        resolved=shutil.which(path)
        if resolved is None:ap.error('missing compiler '+path)
        compilers[label]=str(Path(resolved).resolve())
    if args.reference not in compilers:ap.error('reference compiler missing')
    known={b.name:b for b in bench.BENCHMARKS}
    names=args.only.split(',') if args.only else list(known)
    if not names or len(names)!=len(set(names)) or set(names)-set(known):ap.error('invalid corpus selection')
    import shlex
    flags=shlex.split(args.flags)
    include=subprocess.check_output([args.gcc,'-print-file-name=include'],text=True).strip()
    out=args.out.resolve();out.mkdir(parents=True,exist_ok=True)
    driver=ROOT/'tests/benchmark/batched_driver.c'
    identity=dict(compilers={k:dict(path=v,sha256=sha(v),version=bench.command_version(v)) for k,v in compilers.items()},
                  flags=flags,object_mode=args.object_mode,driver_sha256=sha(driver),object_protocol_sha256=sha(ROOT/'scripts/benchmark_elf_identity.py'),header_provider=bench.command_version(args.gcc),
                  sources={name:dict(sha256=sha(bench.source_for(known[name])),flags=known[name].extra_flags) for name in names})
    if args.relink_driver:
        prior=json.loads((out/'build.json').read_text())
        if not prior['complete']:
            raise ValueError('cannot relink an incomplete build')
        old=dict(prior['identity']);new=json.loads(json.dumps(identity))
        for permitted in ('driver_sha256','object_protocol_sha256'):
            old.pop(permitted);new.pop(permitted)
        if old!=new:
            raise ValueError('relink permits driver/validator changes only; objects are revalidated')
        for name in names:
            for key in compilers:
                directory=out/name/key;check=prior['body_checks'][name][key]
                if (sha(directory/'program')!=prior['binaries'][name][key]
                        or sha(directory/'original.o')!=check['original_object']
                        or sha(directory/'work.o')!=check['renamed_object']
                        or object_identity((directory/'original.o').read_bytes(),'main')!=object_identity((directory/'work.o').read_bytes(),'benchmark_entry')):
                    raise ValueError('cached object/binary drift')
        prior['complete']=False;publish(out/'build.json',prior)
        execute([args.gcc,'-O2','-fno-lto','-c',driver,'-o',out/'driver.o'],out/'driver.json')
        for name in names:
            for key in compilers:
                directory=out/name/key
                execute([args.gcc,'-no-pie',out/'driver.o',directory/'work.o','-lm','-o',directory/'program'],directory/'link.json')
                prior['binaries'][name][key]=sha(directory/'program')
        prior.update(complete=True,identity=identity);publish(out/'build.json',prior)
        args.reuse_build=True
    if args.reuse_build:
        prior=json.loads((out/'build.json').read_text());assert prior['complete'] and prior['identity']==json.loads(json.dumps(identity))
        for name in names:
            for key in compilers:assert sha(out/name/key/'program')==prior['binaries'][name][key]
    else:
        build=dict(complete=False,identity=identity,binaries={},body_checks={})
        publish(out/'build.json',build)
        execute([args.gcc,'-O2','-fno-lto','-c',driver,'-o',out/'driver.o'],out/'driver.json')
        for name in names:
            source=bench.source_for(known[name]);text=source.read_text()
            if not re.search(r'\bint\s+main\s*\(\s*(?:void)?\s*\)',text):raise ValueError('unsupported entry prototype: '+name)
            build['binaries'][name]={};build['body_checks'][name]={}
            for key,cc in compilers.items():
                directory=out/name/key;directory.mkdir(parents=True,exist_ok=True)
                common=[cc,*flags,*known[name].extra_flags,'-I'+include]
                plain=directory/'original.s'
                emit,build_object=measured_object_commands(common,cc,source,plain,directory/'original.o',args.object_mode)
                execute(emit,directory/'original.json')
                execute(build_object,directory/'assemble.json')
                original_object=(directory/'original.o').read_bytes()
                renamed_object=rename_entry(original_object)
                (directory/'work.o').write_bytes(renamed_object)
                if object_identity(original_object,'main') != object_identity(renamed_object,'benchmark_entry'):
                    raise ValueError(f'object rename changed code/data/relocations: {name}/{key}')
                build['body_checks'][name][key]=dict(original_object=sha(directory/'original.o'),renamed_object=sha(directory/'work.o'),whole_object_identity=True)
                execute([args.gcc,'-no-pie',out/'driver.o',directory/'work.o','-lm','-o',directory/'program'],directory/'link.json')
                # The single-run oracle links the exact measured object.
                oracle_input=directory/'original.o' if args.object_mode=='direct' else plain
                execute([cc,oracle_input,'-lm','-o',directory/'original'],directory/'original-link.json')
                original=execute([directory/'original'],directory/'original-run.json')
                build['binaries'][name][key]=sha(directory/'program')
                (directory/'expected.stdout').write_text(original['stdout'])
            publish(out/'build.json',build)
        build['complete']=True;publish(out/'build.json',build)
    if args.build_only:return 0
    affinity,pinning,warnings=bench.choose_affinity(args.cpu)
    report=dict(complete=False,identity=identity,pinning=pinning,warnings=warnings,rows=[],rounds=args.rounds,
                measurement='fresh fork child per entry; fork/exec/wait excluded; entry page faults included; whole-object identity and every stdout checked; identical binaries explicitly share samples',harness_sha256=sha(__file__))
    report['reference']=args.reference
    if args.resume:
        previous=json.loads((out/'results.json').read_text())
        if previous.get('measurement') != report['measurement']:
            raise ValueError('resume measurement protocol changed')
        report['rows']=validate_resume(previous,identity,names,args.reference,args.rounds,args.seconds,args.max_batch_seconds,pinning)
        report['resume']=dict(previous_report_sha256=sha(out/'results.json'),
                              previous_harness_sha256=previous.get('harness_sha256'),
                              reused_completed_rows=len(report['rows']),
                              note='Separated measurement sessions; each retained row has complete paired rounds and unchanged identities.')
    def save_report():
        publish(out/'results.json',report)
        if args.publish_to:
            args.publish_to.parent.mkdir(parents=True,exist_ok=True)
            publish(args.publish_to,report)
    save_report()
    completed=len(report['rows'])
    for index,name in enumerate(names):
        if index < completed:continue
        base=out/name;expected=(base/args.reference/'expected.stdout').read_text()
        def sample(key,n,label):
            assert (base/key/'expected.stdout').read_text()==expected, 'single-run oracle disagreement'
            r=execute([*affinity,base/key/'program',str(n)],base/key/(label+'.json'),timeout=max(180,known[name].timeout_s))
            if r['stdout']!=expected*n:raise ValueError('non-repeatable/incorrect batch: '+name+'/'+key)
            match=re.fullmatch(r'BATCH_RESULT (\{.*\})\n?',r['stderr'])
            if not match:raise ValueError('missing/unexpected batch diagnostic')
            value=json.loads(match[1]);assert value['iterations']==n and value['ns']>0
            return value
        representatives={}; aliases={}
        for key in compilers:
            assert (base/key/'expected.stdout').read_text()==expected, 'single-run oracle disagreement'
            binary_sha=sha(base/key/'program')
            aliases[key]=representatives.setdefault(binary_sha,key)
        measured=list(representatives.values())
        calibration={k:sample(k,1,'calibration') for k in measured}
        for key in measured:
            if calibration[key]['ns'] < 20000000:
                probe_count=min(1000000,max(2,math.ceil(20000000/calibration[key]['ns'])))
                calibration[key]=sample(key,probe_count,'warm-calibration')
        calibration={k:calibration[aliases[k]] for k in compilers}
        count=calibrated_count(calibration,args.seconds,args.max_batch_seconds)
        row=dict(name=name,measured_as=aliases,tags=known[name].tags,iterations=count,calibration=calibration,target_seconds=args.seconds,max_batch_seconds=args.max_batch_seconds,samples=[],comparisons={})
        rng=random.Random(833+index)
        for round_id in range(-1,args.rounds):
            order=list(measured);rng.shuffle(order)
            measured_values={k:sample(k,count,f'round-{round_id}') for k in order}
            values={k:measured_values[aliases[k]] for k in compilers}
            if round_id>=0:row['samples'].append(dict(order=order,values=values))
        for key in compilers:
            ratios=[r['values'][key]['ns']/r['values'][args.reference]['ns'] for r in row['samples']]
            row['comparisons'][key]=dict(identical_binary_to_reference=aliases[key]==aliases[args.reference],minimum_sample_seconds=min(r['values'][key]['ns'] for r in row['samples'])/1e9,median=statistics.median(ratios),paired_ratios=ratios,
                exploratory_ci95=None if aliases[key]==aliases[args.reference] else bench.bootstrap_median_ci(ratios,833+index))
        report['rows'].append(row);save_report()
        print(name,{k:v['median'] for k,v in row['comparisons'].items()},flush=True)
    report['complete']=True;save_report()
    return 0


if __name__=='__main__':raise SystemExit(main())
