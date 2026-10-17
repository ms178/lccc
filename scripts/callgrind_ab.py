#!/usr/bin/env python3
"""Bounded, correctness-gated Callgrind A/B benchmark instrumentation.

Ir and pinned I1/D1/LL/branch counts are simulated instruction/cache-model
metrics, NOT Raptor Lake uops, cycles, PMU counters or measured speedups.
Equal-length aa/bb paths are retained. Matched ISA flags are the caller's job.
Software/library/startup differences can still matter despite pinned geometry.
Importing this module performs no compiler/tool discovery or subprocess work.

Usage: scripts/callgrind_ab.py MINE REF "OPT [OPT...]" [bench ...]
Optional: --out-root DIR --heavy --gcc-include-command gcc
"""
from __future__ import annotations

import argparse
import functools
import hashlib
import math
import os
from pathlib import Path
import platform
import re
import shlex
import shutil
import sys
import tempfile

REPO=Path(__file__).resolve().parents[1];sys.path.insert(0,str(REPO))
from tools.corpus import process,publication,schema
from scripts.perf_ab import split_driver_flags

CG_I1='32768,8,64';CG_D1='32768,8,64';CG_LL='33554432,16,64'
REQUIRED_EVENTS={'Ir','I1mr','D1mr','D1mw','ILmr','DLmr','DLmw','Bcm','Bim'}
DEFAULT_FAST=[
    'arith_loop','fib','matmul','sieve','tce_sum','spectral_norm','switch_dispatch','struct_copy',
    'loop_patterns','ackermann','constant_recursion','bitops','double_reduction','ascii_case_fold',
    'binary_search','ring_fifo','histogram','gzip_crc32','libm_round_family','tls_seg_access',
    'zlib_ng_adler32','expat_xml_scan','sqlite_varint','linux_find_bit','glibc_memcmp',
    'chacha20_block','sha256_transform','linux_rbtree','zstd_count','lz4_compress','qsort']
HEAVY={'nbody','mandelbrot','hash_table','strlen_bench','fannkuch','binary_trees','glibc_strstr'}


def _run(cmd,timeout):return process.normalize(process.run(cmd,timeout))


@functools.lru_cache(maxsize=8)
def gcc_include(command='gcc'):
    r=_run(shlex.split(command)+['-print-file-name=include'],15)
    if r['failure'] or r['returncode']!=0:raise ValueError('GCC include discovery failed')
    value=r['stdout'].decode('utf-8',errors='strict').strip()
    if not value or not Path(value).is_dir():raise ValueError('GCC include discovery did not return a directory')
    return '-I'+value


def geometry():
    defaults={'I1':CG_I1,'D1':CG_D1,'LL':CG_LL};values={};overrides={}
    for key,default in defaults.items():
        env='LCCC_CG_'+key;value=os.environ.get(env,default)
        if env in os.environ:overrides[env]=value
        if not re.fullmatch(r'[1-9][0-9]*,[1-9][0-9]*,[1-9][0-9]*',value):raise ValueError('invalid '+key+' cache geometry')
        size,assoc,line=map(int,value.split(','))
        if (line&(line-1) or size<assoc*line or size%(assoc*line) or
                ((size//(assoc*line))&(size//(assoc*line)-1)) or size>2**40):
            raise ValueError('unsupported '+key+' cache geometry')
        values[key]=value
    return values,overrides


def validate_events(events):
    if not isinstance(events,dict) or not REQUIRED_EVENTS<=events.keys():raise ValueError('missing required Callgrind event(s)')
    if any(type(v) is not int or not 0<=v<=2**64-1 for v in events.values()):raise ValueError('invalid Callgrind event total')
    if events['Ir']<=0:raise ValueError('zero Ir is not comparable performance evidence')
    return events


def parse_summary(path):
    path=Path(path)
    if not path.is_file():raise ValueError('Callgrind output missing')
    if path.stat().st_size>64*1024*1024:raise ValueError('Callgrind file exceeds reader budget')
    events=None;totals=None
    with path.open(encoding='utf-8',errors='strict') as f:
        for line in f:
            if line.startswith('events:'):
                if events is not None:raise ValueError('ambiguous duplicate events header')
                events=line.split()[1:]
            elif line.startswith('summary:'):
                if totals is not None:raise ValueError('ambiguous duplicate summary')
                try:totals=[int(v) for v in line.split()[1:]]
                except ValueError as exc:raise ValueError('invalid Callgrind summary token') from exc
    if not events or totals is None or len(events)!=len(totals) or len(set(events))!=len(events):
        raise ValueError('malformed Callgrind events/summary cardinality')
    return validate_events(dict(zip(events,totals)))


def parse_region_totals(path):
    """Read exclusive file totals for a client-controlled instrumentation region.

    Callgrind 3.27.1 resets global counters on STOP_INSTRUMENTATION, so its
    termination `summary` may be zero or unsigned-underflowed after ZERO_STATS.
    The cost records and their final `totals` remain intact. This is a separate
    parser, not a relaxation of the whole-process summary evidence contract.
    Callgrind cost lists may elide trailing zero events; never missing headers,
    duplicate fields, extra values, negative counts, or an empty/zero-Ir region.
    """
    path = Path(path)
    if not path.is_file() or path.stat().st_size > 64 * 1024 * 1024:
        raise ValueError('missing/oversized Callgrind region file')
    events = totals = None
    with path.open(encoding='utf-8', errors='strict') as stream:
        for line in stream:
            if line.startswith('events:'):
                if events is not None:
                    raise ValueError('duplicate Callgrind region events')
                events = line.split()[1:]
            elif line.startswith('totals:'):
                if totals is not None:
                    raise ValueError('duplicate Callgrind region totals')
                totals = [int(v) for v in line.split()[1:]]
    if (not events or not totals or len(totals) > len(events)
            or len(set(events)) != len(events)):
        raise ValueError('malformed Callgrind region events/totals')
    return validate_events(dict(zip(events, totals + [0] * (len(events) - len(totals)))))


def clear_process_record(prefix):
    paths = {suffix: Path(str(prefix) + '.' + suffix) for suffix in ('stdout', 'stderr', 'json')}
    for path in paths.values():
        path.unlink(missing_ok=True)
    return paths


def run_recorded(command, timeout, prefix):
    """Keep captured stdout/stderr and exact argv, including unsuccessful arms.

    JSON hashes alone are not the raw evidence. Clear a prior record BEFORE
    execution so an infrastructure exception cannot expose stale output.
    """
    paths = clear_process_record(prefix)
    try:
        result = _run(command, timeout)
    except (OSError, ValueError) as error:
        publication.atomic_bytes(paths['json'], schema.dumps(dict(command=command, infrastructure_error=str(error))).encode())
        raise
    info = dict(command=command, process=evidence(result),
                raw_outputs={stream: str(paths[stream]) for stream in ('stdout', 'stderr')})
    for stream in ('stdout', 'stderr'):
        publication.atomic_bytes(paths[stream], result[stream])
    publication.atomic_bytes(paths['json'], schema.dumps(info).encode())
    return result, info


def process_receipt(prefix):
    """Link the exact sidecar and its raw streams into the top-level manifest.

    Infrastructure errors have no captured streams; they still have a receipt.
    Missing/malformed sidecars are errors, never evidence of successful work.
    """
    path=Path(str(prefix)+'.json')
    info=schema.loads(path.read_text(encoding='utf-8'))
    if not isinstance(info,dict):raise ValueError('process receipt must be a JSON object')
    info.setdefault('raw_outputs',{})
    info['receipt']=dict(path=str(path),sha256=schema.file_hash(path))
    return info


def retain_process_receipt(prefix, *, primary_error=None):
    """Keep receipt failures secondary to an already failed process.

    An unreadable receipt never supplies a successful hash or process record.
    Without a primary failure it remains fatal, rather than certifying execution
    with missing evidence. Compilation and native execution share this rule.
    """
    try:return process_receipt(prefix)
    except (OSError,ValueError) as error:
        if primary_error is None:raise
        return dict(receipt_error=str(error),receipt_path=str(prefix)+'.json')


def compile_program(compiler,opt,source,out,*,include=None):
    """No builtin-shadowing compile(), no eager GCC discovery, no stale binary."""
    out=Path(out);out.unlink(missing_ok=True)
    paths=clear_process_record(str(out)+'.compile')
    try:
        opts=shlex.split(opt) if isinstance(opt,str) else list(opt)
        include=include or gcc_include()
        frontend,linker=split_driver_flags(opts)
        # Use the native harness's ordering authority: --as-needed must see
        # the object before its libraries, in both measurement arms.
        r,_=run_recorded(shlex.split(str(compiler))+[include]+frontend+[str(source)]+linker+['-o',str(out)],180,str(out)+'.compile')
    except (OSError,ValueError) as exc:
        if not paths['json'].exists():
            publication.atomic_bytes(paths['json'],schema.dumps(dict(compiler=str(compiler),options=opt,infrastructure_error=str(exc))).encode())
        return 'compile infrastructure: '+str(exc)
    if r['failure'] or r['returncode']!=0:return f"compile rc={r['returncode']} failure={r['failure']}: "+r['stderr'].decode('utf-8',errors='replace')[-400:]
    if not out.is_file() or not os.access(out,os.X_OK):return 'compiler reported success but produced no executable'
    return None


def same_output(candidate,reference):
    if candidate.get('failure') or reference.get('failure'):return 'process resource/timeout failure'
    if candidate['returncode']!=0 or reference['returncode']!=0:return f"nonzero exit {candidate['returncode']}/{reference['returncode']}"
    if candidate['stdout']!=reference['stdout']:return 'stdout mismatch'
    return None


def evidence(r):
    return dict(returncode=r['returncode'],failure=r.get('failure'),elapsed=r['elapsed'],
                stdout_sha256=r['stdout_sha256'],stderr_sha256=r['stderr_sha256'],stderr_excerpt=r['stderr'].decode('utf-8',errors='replace')[-400:])


def callgrind(binary,outdir,*,cache_geometry=None,expected_stdout=None,timeout=1200):
    cg=Path(outdir)/(Path(binary).name+'.cg');cg.unlink(missing_ok=True)
    model=cache_geometry or geometry()[0]
    cmd=['valgrind','--tool=callgrind','--cache-sim=yes','--branch-sim=yes','--quiet',
         f'--I1={model["I1"]}',f'--D1={model["D1"]}',f'--LL={model["LL"]}',f'--callgrind-out-file={cg}',str(binary)]
    try:r,info=run_recorded(cmd,timeout,str(cg))
    except (OSError,ValueError) as exc:return None,'Callgrind infrastructure: '+str(exc),dict(command=cmd)
    if r['failure'] or r['returncode']!=0:return None,f"Callgrind rc={r['returncode']} failure={r['failure']}",info
    if expected_stdout is not None and r['stdout']!=expected_stdout:return None,'instrumented stdout mismatch',info
    try:events=parse_summary(cg)
    except (OSError,ValueError) as exc:return None,str(exc),info
    info['counter_file_sha256']=schema.file_hash(cg)
    return events,None,info


def compare_events(reference,candidate):
    r=validate_events(reference);m=validate_events(candidate)
    ratio=m['Ir']/r['Ir']  # both are strictly positive bounded integer counts
    if not math.isfinite(ratio) or ratio<=0:raise ValueError('invalid Ir ratio')
    return dict(ir_ref=r['Ir'],ir_mine=m['Ir'],ir_ratio=ratio,
                i1_misses=[m['I1mr'],r['I1mr']],d1_misses=[m['D1mr']+m['D1mw'],r['D1mr']+r['D1mw']],
                ll_misses=[m['ILmr']+m['DLmr']+m['DLmw'],r['ILmr']+r['DLmr']+r['DLmw']],
                conditional_mispredicts=[m['Bcm'],r['Bcm']],indirect_mispredicts=[m['Bim'],r['Bim']])


def tool_identity(command,option='--version'):
    argv=shlex.split(str(command));executable=shutil.which(argv[0]) if argv else None
    if not executable:raise ValueError('tool unavailable: '+str(command))
    r=_run(argv+[option],15)
    text=(r['stdout']+r['stderr']).decode('utf-8',errors='replace').strip()
    if r['failure'] or r['returncode']!=0 or not text:raise ValueError('tool version unavailable: '+str(command))
    return dict(command=argv,executable=str(Path(executable).resolve()),sha256=schema.file_hash(Path(executable)),version=text[:4096])


def _measure(args,benches,outroot,manifest,cache):
    mdir=outroot/'aa';rdir=outroot/'bb';mdir.mkdir(exist_ok=True);rdir.mkdir(exist_ok=True)
    if len(os.fsencode(mdir))!=len(os.fsencode(rdir)):raise ValueError('unequal A/B path byte lengths')
    manifest['paths']=dict(mine=str(mdir),ref=str(rdir),equal_byte_length=True)
    manifest['include']=gcc_include(args.gcc_include_command)
    manifest['tools']={name:tool_identity(cmd) for name,cmd in [('mine',args.mine),('ref',args.ref),('valgrind','valgrind'),('include_discovery',args.gcc_include_command)]}
    rows=[];failures=manifest['failures']
    for bench in benches:
        source=REPO/'tests/benchmark/programs'/f'{bench}.c';entry=dict(status='ERROR');manifest['results'][bench]=entry
        if not source.is_file():entry['detail']='missing-source';failures[bench]='missing-source';continue
        entry['source_sha256']=schema.file_hash(source);mbin=mdir/bench;rbin=rdir/bench
        entry['compilation']={}
        err=None
        for side,compiler,binary in (('mine',args.mine,mbin),('ref',args.ref,rbin)):
            err=compile_program(compiler,args.opt,source,binary,include=manifest['include'])
            entry['compilation'][side]=retain_process_receipt(str(binary)+'.compile',primary_error=err)
            if err:break
        if err:entry['detail']=err;failures[bench]=err;continue
        entry['binary_sha256']=dict(mine=schema.file_hash(mbin),ref=schema.file_hash(rbin))
        entry['native']={}
        native={}
        try:
            for side,binary in (('mine',mbin),('ref',rbin)):
                prefix=str(binary)+'.native'
                try:
                    native[side],_=run_recorded([str(binary)],120,prefix)
                except (OSError,ValueError) as error:
                    entry['native'][side]=retain_process_receipt(prefix,primary_error=error)
                    raise
                else:
                    entry['native'][side]=retain_process_receipt(prefix)
        except (OSError,ValueError) as exc:entry['detail']=str(exc);failures[bench]=str(exc);continue
        pm,pr=native['mine'],native['ref']
        entry['correctness']=dict(mine=evidence(pm),ref=evidence(pr))
        err=same_output(pm,pr)
        if err:entry['detail']=err;failures[bench]=err;continue
        m_events,me,mi=callgrind(mbin,mdir,cache_geometry=cache,expected_stdout=pm['stdout'],timeout=args.instrument_timeout)
        r_events,re,ri=callgrind(rbin,rdir,cache_geometry=cache,expected_stdout=pr['stdout'],timeout=args.instrument_timeout)
        entry['instrumentation']=dict(mine=mi,ref=ri)
        if me or re:entry['detail']=me or re;failures[bench]=me or re;continue
        try:metrics=compare_events(r_events,m_events)
        except ValueError as exc:entry['detail']=str(exc);failures[bench]=str(exc);continue
        entry.update(status='COMPARABLE-SIMULATION',ref=r_events,mine=m_events,metrics=metrics);rows.append((bench,metrics))
    if rows:
        geomean=math.exp(math.fsum(math.log(m['ir_ratio']) for _,m in rows)/len(rows))
        if not math.isfinite(geomean) or geomean<=0:raise ValueError('invalid aggregate Ir metric')
        manifest['geomean_ir_mine_over_ref']=geomean
    manifest['comparable']=len(rows);manifest['selected']=len(benches)
    print('| benchmark | Ir ref | Ir mine | Ir m/r | I1miss m/r | D1miss r+w m/r | LLmiss r+w m/r | Bcm m/r | Bim m/r |')
    print('| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |')
    for bench,m in rows:
        pair=lambda key:'/'.join(str(v) for v in m[key])
        print(f'| {bench} | {m["ir_ref"]} | {m["ir_mine"]} | {m["ir_ratio"]:.5f} | {pair("i1_misses")} | {pair("d1_misses")} | {pair("ll_misses")} | {pair("conditional_mispredicts")} | {pair("indirect_mispredicts")} |')
    if rows:print('geomean simulated Ir mine/ref =',manifest['geomean_ir_mine_over_ref'],'(not measured speedup)')
    return 2 if not rows else 1 if failures else 0


def build_parser():
    """The one parser `main` parses with, factored out so its option set can be
    read off it rather than restated beside it."""
    ap=argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument('mine');ap.add_argument('ref')
    ap.add_argument('--opt',required=True);ap.add_argument('bench',nargs='*')
    ap.add_argument('--instrument-timeout',type=int,default=1200,help='seconds per instrumented arm')
    ap.add_argument('--heavy',action='store_true')
    ap.add_argument('--out-root',type=Path)
    ap.add_argument('--gcc-include-command',default='gcc')
    return ap


def long_options(ap):
    """Every long option `ap` defines, derived from the parser itself.

    `_OWN_OPTIONS` used to be a hand-written frozenset declared next to main --
    a second source of truth.  Add an option to the parser and forget the set,
    and the rewrite in main silently swallows the new flag into `--opt=`, which
    turns "missing required --opt" into a bogus value and hides the real error.
    Deriving it makes that drift impossible rather than merely tested for.
    argparse exposes no public enumeration of a parser's options, so this reads
    `_actions`, which has been stable since 3.4 and is what `format_usage`
    itself walks.
    """
    return frozenset(opt.split('=',1)[0] for action in ap._actions
                     for opt in action.option_strings if opt.startswith('--'))


# Every long option the parser defines, so the option-string rewrite below never
# swallows one of the script's own flags into `--opt=`.
_OWN_OPTIONS=long_options(build_parser())


def main(argv=None):
    argv=list(sys.argv[1:] if argv is None else argv)
    ap=build_parser()
    # Preserve the original third positional quoted option-string, including
    # a single '-O2' (argparse would mistake that positional for an option).
    # Anything spelling one of this parser's own long options is left alone:
    # rewriting `mine ref --out-root X` into `--opt=--out-root` would hide the
    # missing required `--opt` behind a bogus value instead of reporting it.
    # An explicit --opt anywhere before the separator makes the third token
    # a benchmark, not legacy flags. Consuming it used to silently select the
    # entire default/heavy corpus instead of the requested bounded workload.
    options = argv[:argv.index('--')] if '--' in argv else argv
    explicit_opt = any(arg == '--opt' or arg.startswith('--opt=') for arg in options)
    if not explicit_opt and len(argv)>=3 and argv[2].split('=',1)[0] not in _OWN_OPTIONS:
        argv=argv[:2]+['--opt='+argv[2]]+argv[3:]
    # parse_INTERMIXED_args, not parse_args.  The rewrite above puts an optional
    # in front of the `bench` positional, and plain parse_args consumes
    # positionals in the contiguous groups the optionals split them into -- a
    # group that `nargs='*'` is satisfied by with NOTHING in it.  CPython 3.12.3,
    # which is what Ubuntu 24.04 ships and therefore what hosted CI runs, binds
    # `bench` to empty and then rejects every bench name as `unrecognized
    # arguments: fib`; later 3.12 patches and 3.13 group them correctly.  That
    # is the whole of PR #730's red "Verify remaining fast local contracts"
    # step: green on every developer host here (3.12.15, 3.13.14), 4 errors on
    # the runner.  parse_intermixed_args is the stdlib answer to exactly this,
    # and unlike hand-rolled argv reordering it keeps a bare `--` meaningful.
    args=ap.parse_intermixed_args(argv);benches=args.bench or DEFAULT_FAST+(sorted(HEAVY) if args.heavy else [])
    if args.instrument_timeout < 1:ap.error('instrumentation timeout must be positive')
    if len(set(benches))!=len(benches) or any(not re.fullmatch(r'[A-Za-z0-9_][A-Za-z0-9_.-]*',b) for b in benches):
        ap.error('duplicate/unsafe benchmark name')
    # A unique common prefix avoids stale/concurrent default outputs. Explicit
    # out-root runs serialize the entire measurement under a same-root flock.
    outroot=args.out_root or Path(tempfile.mkdtemp(prefix='cg-ab-'));outroot.mkdir(parents=True,exist_ok=True)
    manifest=dict(schema='lccc-callgrind-ab-v2',model='simulated instructions/I1/D1/LL/branch predictor; not PMU/uops/cycles/speedup',
                  opt=args.opt,opt_tokens=shlex.split(args.opt),instrument_timeout_s=args.instrument_timeout,heavy_opt_in=bool(set(benches)&HEAVY),
                  host=dict(platform=platform.platform(),machine=platform.machine(),libc=platform.libc_ver()),
                  compiler_environment={k:v for k,v in sorted(os.environ.items()) if k.startswith(('CCC_','LCCC_')) and not any(s in k.upper() for s in ('TOKEN','SECRET','PASSWORD','KEY'))},
                  failures={},results={},comparable=0,selected=len(benches))
    rc=2
    with publication.lock(outroot/'.callgrind.lock'):
        try:
            cache,overrides=geometry();manifest.update(geometry=cache,geometry_env_overrides=overrides)
            rc=_measure(args,benches,outroot,manifest,cache)
        except (OSError,ValueError) as exc:manifest['failures']['setup']=str(exc);print('Callgrind setup failed:',exc,file=sys.stderr)
        publication.atomic_bytes(outroot/'manifest.json',schema.dumps(manifest).encode())
    print('manifest ->',outroot/'manifest.json')
    return rc


if __name__=='__main__':raise SystemExit(main())
