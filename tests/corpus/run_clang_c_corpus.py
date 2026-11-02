#!/usr/bin/env python3
"""Strict, conservative per-invocation Clang C corpus runner.

Each actual Clang RUN is authoritative; EDG options/cases are inventory, never
zipped/broadcast to verifier prefixes. Only supported C frontend/preprocessor
translations execute. Every planned ID has an observation, including skips,
unsupported phases/architectures/capabilities and hard process failures.
XFAIL is a separate standard policy, never an inverted rejection expectation.
GCC comparisons are advisory, per-ID and only comparable clean observations.
--plan performs no compiler discovery/execution and reports zero coverage.
"""
from __future__ import annotations

import argparse
from collections import Counter
import concurrent.futures as cf
import enum
import hashlib
import math
from pathlib import Path
import re
import shlex
import shutil
import subprocess
import sys
import time

REPO=Path(__file__).resolve().parents[2];sys.path.insert(0,str(REPO))
from tools.corpus import diagnostics,process,publication,schema
from scripts.edg_corpus_mine import verify_index
CORPUS_INDEX=Path(__file__).resolve().parent/'clang-c/corpus-index.json'


class Outcome(str,enum.Enum):
    ACCEPT='ACCEPT';REJECT='REJECT';CRASH='CRASH';TIMEOUT='TIMEOUT';ERROR='ERROR'
    UNSUPPORTED='UNSUPPORTED';SKIP='SKIP'


def classify(returncode,stderr):return Outcome(diagnostics.classify(returncode,stderr))
def template_to_regex(msg):return diagnostics.template_regex(msg).pattern
parse_count=schema.count
spawn_process=process.run


def arch_name(value):
    if not value:return None
    first=value.lower().split('-')[0]
    if first in {'x86_64','amd64','x64'} or value.lower()=='x86-64':return 'x86_64'
    if first in {'i386','i486','i586','i686','x86'}:return 'x86'
    if first in {'arm64','aarch64'}:return 'aarch64'
    if first.startswith(('arm','thumb')):return 'arm'
    return first


def effective_arch(inv,base,default_arch):
    value=arch_name(default_arch);flags=base[1:]+inv['flags'];i=0
    while i<len(flags):
        token=flags[i];i+=1
        if token in {'-target','--target'} and i<len(flags):value=arch_name(flags[i]);i+=1
        elif token.startswith(('--target=','-target=')):value=arch_name(token.partition('=')[2])
        elif token in {'-m32','-m64'}:
            value=('x86' if token=='-m32' else 'x86_64') if value in {'x86','x86_64'} else None
    return value


def capabilities_for(family):
    # Static translation contracts, NOT proof of compiler feature correctness.
    # Exotic flags must be explicitly acknowledged. External inputs always
    # need separate resolution (unresolved %S/%t stays unsupported regardless).
    caps=set()
    if family=='clang':
        caps.add('clang-driver-target')
        from tools.corpus.directives import KNOWN_FLAGS
        caps.update('flag:'+f for f in KNOWN_FLAGS)
    return caps


def plan_invocations(rec,want_arch=None,include_cpp=False,*,compiler_family='custom',capabilities=()):
    if include_cpp:raise ValueError('C++ execution is outside this corpus contract')
    plans=[];caps=capabilities_for(compiler_family)|set(capabilities)
    for inv in rec['invocations']:
        schema.invocation(inv,rec['expected'])
        p=dict(inv);reasons=list(inv['unsupported'])
        if inv['language']!='c':reasons.append('C++ execution excluded' if inv['language']=='c++' else 'non-C language execution excluded')
        if inv['phase'] not in ('frontend','preprocess'):reasons.append('unsupported phase: '+inv['phase'])
        if inv['target'] and (not want_arch or arch_name(inv['target'])!=arch_name(want_arch)):
            reasons.append('architecture prerequisite: '+inv['target'])
        for c in inv['capabilities']:
            if c not in caps and not (compiler_family=='clang' and c.startswith(('flag:-W','flag:-fgnuc-version='))):
                reasons.append('compiler capability not established: '+c)
        standards=[f[5:] for f in inv['flags'] if f.startswith('-std=')]
        valid=re.compile(r'^(?:c|gnu)(?:89|90|99|11|17|18|2x|23)$|^iso9899:(?:1990|199409|1999|2011|2017)$')
        if standards and not valid.fullmatch(standards[-1]):reasons.append('unsupported C dialect: '+standards[-1])
        if standards and standards[-1] in {'c89','c90','gnu89','gnu90','iso9899:1990','iso9899:199409'}:
            reasons.append('C90 attribution comment effects require native verifier triage')
        if not standards and compiler_family!='clang' and 'clang-default-c-dialect' not in caps:
            reasons.append('implicit Clang C dialect not established')
        if compiler_family=='custom' and 'c-driver-contract' not in caps:
            reasons.append('custom compiler driver contract not established')
        p['unsupported']=list(dict.fromkeys(reasons));plans.append(p)
    return plans


def check_diagnostics(expected,active_prefixes,stderr,*,source=Path('source.c'),ignore_unexpected=(),parsed=None):
    active=[dict(e,message=e['msg'],unsupported_reason='; '.join(e['unsupported']))
            for e in expected if e['prefix'] in active_prefixes]
    return diagnostics.verify(active,diagnostics.parse(stderr) if parsed is None else parsed,source=source,ignore_unexpected=ignore_unexpected)


def _row(rec,inv,*,planned=True):
    return dict(id=inv['id'],source_id=rec['id'],origin=rec['origin'],file=rec['file'],category=rec['category'],
                planned=planned,phase=inv['phase'],language=inv['language'],expect=inv['expect'],xfail=inv['xfail'],
                prefixes=inv['prefixes'],source_command=inv['command'],flags=inv['flags'],target=inv['target'],
                unsupported=inv['unsupported'],outcome='SKIP',status='SKIP',detail='',attempted=False,spawned=False,
                executed=False,diagnostics_verified=False,elapsed=0.0,returncode=None,cmd=[],translation_key=None,
                stdout_sha256=None,stderr_sha256=None,stdout_excerpt='',stderr_excerpt='',diagnostic_count=0,verification=None)


def _summary(rec,rows):
    schema.validate_results(rows)
    return dict(id=rec['id'],origin=rec['origin'],observations=rows,by_status=dict(Counter(r['status'] for r in rows)))


def run_one(rec,corpus_root,cc,timeout=120,syntax_only=True,run_reject=True,mode='diagnostics',
            want_arch=None,spawn=None,*,compiler_family='custom',capabilities=(),plan_only=False,default_arch=None):
    """All planned IDs survive; never summarize several outcomes as the last one."""
    if not syntax_only:raise ValueError('object generation has no corpus oracle; use supported frontend phases')
    if mode not in {'diagnostics','outcome-only'}:raise ValueError('unknown verification mode')
    spawn=spawn or spawn_process
    if not rec['file']:
        inv=dict(id=rec['id']+'#unplanned-source',phase='unsupported',language='c',expect='accept',xfail=False,
                 prefixes=[],command='(uncopied inventory source)',flags=[],target=None,unsupported=[])
        row=_row(rec,inv,planned=False);row['detail']=rec['skip_reason'];return _summary(rec,[row])
    plans=plan_invocations(rec,want_arch,compiler_family=compiler_family,capabilities=capabilities)
    rows=[];test_file=corpus_root/rec['file']
    for inv in plans:
        row=_row(rec,inv);rows.append(row)
        if not run_reject and inv['expect']=='reject':row['detail']='--no-reject';continue
        if inv['unsupported']:
            row.update(status='UNSUPPORTED',outcome='UNSUPPORTED',detail='; '.join(inv['unsupported']));continue
        if plan_only:row['detail']='plan-only: no compiler process executed';continue
        # Validate source identity immediately before each attempted spawn.
        try:
            schema.relative(rec['file'])
            schema.require(test_file.is_file() and not test_file.is_symlink() and corpus_root.resolve() in test_file.resolve().parents,'missing/escaping/symlink source')
            schema.require(schema.file_hash(test_file)==rec['sha256'],'fixture SHA-256 mismatch')
        except (OSError,ValueError) as exc:
            row.update(status='ERROR',outcome='ERROR',detail=str(exc));continue
        try:
            base=shlex.split(cc);schema.require(bool(base),'empty compiler command')
            cmd=base+['-x','c']+inv['flags']+(['-E'] if inv['phase']=='preprocess' else ['-fsyntax-only'])+[str(test_file)]
        except ValueError as exc:
            row.update(status='ERROR',outcome='ERROR',detail=str(exc));continue
        row['cmd']=cmd
        # Executable name may differ. ALL launcher/compiler arguments, source,
        # phase, target and an established architecture contract must agree.
        translated=dict(id=inv['id'],sha256=rec['sha256'],phase=inv['phase'],language=inv['language'],
                        target=inv['target'],architecture=effective_arch(inv,base,default_arch),
                        flags=base[1:]+['-x','c']+inv['flags'])
        row['translation_key']=hashlib.sha256(schema.dumps(translated).encode()).hexdigest() if translated['architecture'] else None
        row['attempted']=True
        try:
            try:
                result=process.normalize(spawn(cmd,timeout));row['spawned']=True
            except subprocess.TimeoutExpired as exc:
                # Adapter for injected/legacy subprocess mocks; real process.run
                # preserves bounded partial output and its raw termination code.
                result=process.normalize((None,exc.output or b'',exc.stderr or b''));result['failure']='timeout';row['spawned']=True
            except (OSError,ValueError) as exc:
                row.update(status='ERROR',outcome='ERROR',detail='spawn error: '+str(exc));continue
            stderr=result['stderr'].decode('utf-8',errors='replace');stdout=result['stdout'].decode('utf-8',errors='replace')
            row.update(elapsed=result['elapsed'], returncode=result['returncode'], executed=True,
                       stdout_sha256=result['stdout_sha256'], stderr_sha256=result['stderr_sha256'],
                       stdout_excerpt=stdout[:2048], stderr_excerpt=stderr[:4096])
            parsed = diagnostics.parse(stderr, compiler_family=compiler_family, returncode=result['returncode'])
            outcome=diagnostics.classify(result['returncode'],stderr,failure=result.get('failure'),compiler_family=compiler_family,parsed=parsed)
            try:
                schema.require(test_file.is_file() and not test_file.is_symlink() and corpus_root.resolve() in test_file.resolve().parents,'source changed/escaped during process')
                schema.require(schema.file_hash(test_file)==rec['sha256'],'fixture changed during process')
            except (OSError,ValueError) as exc:
                outcome='ERROR';row['detail']=str(exc)
            row.update(outcome=outcome,elapsed=result['elapsed'],returncode=result['returncode'],executed=True,
                       stdout_sha256=result['stdout_sha256'],stderr_sha256=result['stderr_sha256'],
                       stdout_excerpt=stdout[:2048],stderr_excerpt=stderr[:4096],diagnostic_count=len(parsed))
            ok=True
            if outcome in {'ACCEPT','REJECT'} and mode=='diagnostics' and inv['verify']:
                verification=check_diagnostics(rec['expected'],inv['prefixes'],stderr,source=test_file,ignore_unexpected=inv['ignore_unexpected'],parsed=parsed)
                ok=verification['ok'];row['diagnostics_verified']=ok
                row['verification']=dict(ok=ok,detail=verification['detail'],match_count=len(verification['matches']),
                        missing_count=len(verification['missing']),unexpected_count=len(verification['unexpected']),
                        matches=verification['matches'][:32],missing=verification['missing'][:32],unexpected=verification['unexpected'][:32])
                if 'resource budget' in verification['detail']:
                    outcome='ERROR';row['outcome']='ERROR';row['diagnostics_verified']=False
                row['detail']=verification['detail']
            elif outcome in schema.TERMINAL:
                row['detail']=row['detail'] or result.get('failure') or f'{outcome}: process returncode={result["returncode"]}'
            if not row['detail'] and outcome!=inv['expect'].upper():row['detail']='expected '+inv['expect']+', observed '+outcome
            row['status']=schema.verdict(outcome,inv['expect'],ok,inv['xfail'])
        except Exception as exc:
            # A programming/adapter/worker failure is not a C rejection or an
            # XFAIL. Retain completed invocations and any captured raw process
            # evidence; keep this ID terminal instead of aborting the report.
            row.update(outcome='ERROR', status='ERROR', diagnostics_verified=False,
                       executed=row['spawned'],
                       detail=f'worker {type(exc).__name__}: {exc}')

    return _summary(rec,rows)


def is_divergence(candidate,reference):
    return (candidate['id']==reference['id'] and candidate['executed'] and reference['executed']
            and candidate['outcome'] in {'ACCEPT','REJECT'} and reference['outcome'] in {'ACCEPT','REJECT'}
            and candidate['translation_key'] is not None and candidate['translation_key']==reference['translation_key']
            and candidate['outcome']!=reference['outcome'])


def pairwise(candidate,reference):
    c={r['id']:r for r in candidate};ref={r['id']:r for r in reference};pairs=[]
    for id in sorted(c.keys()|ref.keys()):
        a=c.get(id);b=ref.get(id);state='NONCOMPARABLE';reason='missing observation'
        if a and b:
            if a['outcome'] in schema.TERMINAL or b['outcome'] in schema.TERMINAL:reason='terminal process observation'
            elif not a['executed'] or not b['executed']:reason='not executed by both engines'
            elif a['translation_key'] is None or a['translation_key']!=b['translation_key']:reason='translation/architecture contracts differ or are unestablished'
            else:state='DIVERGENCE' if is_divergence(a,b) else 'AGREEMENT';reason='clean per-invocation outcome comparison (advisory)'
        pairs.append(dict(id=id,state=state,reason=reason,candidate=a['outcome'] if a else None,reference=b['outcome'] if b else None))
    return pairs


def compiler_identity(cc,family='auto',*,spawn=None):
    spawn=spawn or spawn_process;argv=shlex.split(cc)
    schema.require(bool(argv),'empty compiler command');executable=shutil.which(argv[0])
    schema.require(executable is not None,'compiler executable unavailable: '+argv[0])
    result=process.normalize(spawn(argv+['--version'],15))
    schema.require(result['returncode']==0 and not result.get('failure'),'compiler version probe failed')
    text=(result['stdout']+result['stderr']).decode('utf-8',errors='replace').strip()
    schema.require(bool(text) and '\ufffd' not in text,'compiler version output missing/invalid')
    auto='clang' if 'clang' in text.lower() else 'lccc' if 'lccc' in text.lower() else 'gcc' if re.search(r'\bgcc\b|Free Software Foundation',text,re.I) else 'custom'
    schema.require(family in {'auto','custom',auto},'declared compiler family conflicts with version evidence')
    default_arch=None;target=re.search(r'^Target:\s*(\S+)',text,re.M)
    if target:default_arch=arch_name(target[1])
    # GCC --version lacks its target. A metadata query is not a compilation.
    if auto=='gcc':
        machine=process.normalize(spawn(argv+['-dumpmachine'],15))
        if machine['returncode']==0 and not machine.get('failure'):
            value=machine['stdout'].decode('ascii',errors='replace').strip()
            if re.fullmatch(r'[A-Za-z0-9_.+-]+',value):default_arch=arch_name(value)
    return dict(command=argv,family=auto if family=='auto' else family,verified=True,
                executable=str(Path(executable).resolve()),executable_sha256=schema.file_hash(Path(executable)),
                command_file_hashes={arg:schema.file_hash(Path(arg)) for arg in argv[1:] if Path(arg).is_file()},
                version=text[:4096],default_arch=default_arch)


def coverage(doc,files,rows):
    planned=[r for r in rows if r['planned']];unplanned=[r for r in rows if not r['planned']]
    statuses=Counter(r['status'] for r in planned);reasons=Counter(reason for r in planned for reason in r['unsupported'])
    schema.require(len(planned)==sum(len(r['invocations']) for r in files),'lost planned observations')
    schema.require(len(unplanned)==sum(r['file'] is None for r in files),'lost unplanned source skips')
    schema.require(sum(statuses.values())==len(planned),'status partition mismatch')
    return dict(indexed_files=doc['count'],copied_files=doc['copied'],unplanned_files=doc['skipped'],
                total_planned_invocations=sum(len(r['invocations']) for r in doc['files']),selected_files=len(files),
                selected_planned_invocations=len(planned),selected_unplanned_source_skips=len(unplanned),
                executed_invocations=sum(r['executed'] for r in planned),spawn_attempts=sum(r['attempted'] for r in planned),
                executed_files=len({r['source_id'] for r in planned if r['executed']}),
                clean_semantic_observations=sum(r['executed'] and r['outcome'] in {'ACCEPT','REJECT'} for r in planned),
                diagnostic_verified_invocations=sum(r['diagnostics_verified'] for r in planned),
                plan_eligible_invocations=sum(r['detail']=='plan-only: no compiler process executed' for r in planned),
                by_status=dict(statuses),by_phase=dict(Counter(r['phase'] for r in planned)),
                unsupported_reason_incidence=dict(reasons),cxx_execution=0,gnu_execution=0,
                interpretation='eligibility is not executed coverage; reason incidences overlap; GNU facts are inventory only')


def main(argv=None):
    ap=argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument('--index',type=Path,default=CORPUS_INDEX);ap.add_argument('--cc',default='lccc')
    ap.add_argument('--gcc-cmd',help='advisory reference command; no authoritative GCC verdict')
    ap.add_argument('--compiler-family',choices=['auto','lccc','gcc','clang','custom'],default='auto')
    ap.add_argument('--reference-family',choices=['auto','lccc','gcc','clang','custom'],default='auto')
    ap.add_argument('--capability',action='append',default=[]);ap.add_argument('--reference-capability',action='append',default=[])
    ap.add_argument('--arch',help='architecture prerequisite filter; does not retarget unqualified RUNs or establish the compiler default target')
    ap.add_argument('--category');ap.add_argument('--limit',type=int,default=0);ap.add_argument('--timeout',type=float,default=120)
    ap.add_argument('-j','--jobs',type=int,default=2);ap.add_argument('--syntax-only',action='store_true',help='retained compatibility; frontend phases already use syntax-only')
    ap.add_argument('--no-reject',action='store_true');ap.add_argument('--plan',action='store_true',help='no compiler probes or execution; coverage stays zero')
    ap.add_argument('--mode',choices=['diagnostics','outcome-only'],default='diagnostics');ap.add_argument('--report',type=Path)
    args=ap.parse_args(argv)
    try:
        schema.require(args.jobs>0 and args.limit>=0 and math.isfinite(args.timeout) and args.timeout>0,'invalid jobs/limit/timeout budget')
        doc=verify_index(args.index);files=doc['files'];index_sha256=schema.file_hash(args.index)
        if args.category:
            known=sorted({r['category'] for r in files});schema.require(args.category in known,'unknown category '+repr(args.category)+'; known: '+', '.join(known))
            files=[r for r in files if r['category']==args.category]
        if args.limit:files=files[:args.limit]
        schema.require(bool(files),'empty selection is not a corpus pass')
        if args.report:
            schema.require(args.index.parent.resolve() not in (args.report.resolve(),*args.report.resolve().parents),'report path must not overwrite corpus inputs')
        schema.strings(args.capability);schema.strings(args.reference_capability)
    except (OSError,ValueError,TypeError) as exc:
        print('corpus usage/preflight error: '+str(exc),file=sys.stderr);return 2
    started=time.monotonic();engines={};probe_errors={};observations={}
    commands=[('candidate',args.cc,args.compiler_family,args.capability)]
    if args.gcc_cmd:commands.append(('reference',args.gcc_cmd,args.reference_family,args.reference_capability))
    for name,cc,family,caps in commands:
        declared='lccc' if family=='auto' else family
        identity=dict(command=[cc],family=declared,verified=False,default_arch=None,version=None,executable_sha256=None)
        # Superset preflight: don't even probe an engine if policy/source
        # constraints leave no potentially executable plan.
        possible=any(not p['unsupported'] and (not args.no_reject or p['expect']!='reject')
                     for r in files for p in plan_invocations(r,args.arch,compiler_family='clang',capabilities=caps))
        if not args.plan and possible:
            try:identity=compiler_identity(cc,family)
            except (OSError,ValueError) as exc:probe_errors[name]=str(exc)
        engines[name]=identity
        if name in probe_errors:
            # No compile was attempted. Keep every planned ID as ERROR and
            # unplanned files as SKIP, so a missing tool can't disappear.
            rows=[]
            for r in files:
                result=run_one(r,args.index.parent.parent,cc,args.timeout,True,not args.no_reject,args.mode,args.arch,
                               compiler_family=declared,capabilities=caps,plan_only=True)
                for row in result['observations']:
                    if row['planned'] and row['detail']=='plan-only: no compiler process executed':
                        row.update(outcome='ERROR',status='ERROR',detail='compiler identity probe: '+probe_errors[name])
                    rows.append(row)
            observations[name]=rows;continue
        def execute(r):
            return run_one(r,args.index.parent.parent,cc,args.timeout,True,not args.no_reject,args.mode,args.arch,
                           compiler_family=identity['family'],capabilities=caps,plan_only=args.plan,default_arch=identity['default_arch'])
        with cf.ThreadPoolExecutor(max_workers=args.jobs) as pool:
            observations[name]=sorted((row for result in pool.map(execute,files) for row in result['observations']),key=lambda r:r['id'])
        schema.validate_results(observations[name])
    metrics={name:coverage(doc,files,rows) for name,rows in observations.items()}
    pairs=pairwise(observations['candidate'],observations['reference']) if args.gcc_cmd else []
    gnu_summary=REPO/'tests/corpus/gnu-torture-manifest-summary.json'
    gnu_count=schema.loads(gnu_summary.read_text())['count'] if gnu_summary.is_file() else None
    report=dict(schema=schema.REPORT_VERSION,index_schema=doc['schema'],index_sha256=index_sha256,source=doc['source'],
                plan_only=args.plan,mode=args.mode,architecture_filter=args.arch,engines=engines,probe_errors=probe_errors,
                wall_s=time.monotonic()-started,coverage=metrics,observations=observations,pairwise=pairs,
                pairwise_counts=dict(Counter(p['state'] for p in pairs)),inventory_only=dict(gnu_c_records=gnu_count,gnu_execution=0,cxx_execution=0))
    index_error = None
    try:
        changed = schema.file_hash(args.index) != index_sha256
    except OSError as exc:
        changed = True
        index_error = str(exc)
    input_errors=[]
    for r in files:
        if not r['file']:continue
        try:
            path=args.index.parent.parent/r['file']
            schema.require(path.is_file() and not path.is_symlink() and schema.file_hash(path)==r['sha256'],'fixture changed/missing')
        except (OSError,ValueError) as exc:input_errors.append(dict(id=r['id'],detail=str(exc)))
    if changed or input_errors:report['input_changed_during_run']=dict(index_changed=changed,index_error=index_error,fixture_errors=input_errors)
    if args.report:
        try:
            args.report.parent.mkdir(parents=True,exist_ok=True);encoded=schema.dumps(report).encode()
            schema.require(len(encoded)<=64*1024*1024,'report exceeds bounded publication budget')
            publication.atomic_bytes(args.report,encoded)
        except (OSError,ValueError) as exc:
            print('report publication failed: '+str(exc),file=sys.stderr);return 1
    for name,m in metrics.items():
        print(f"{name}: {m['selected_files']} selected files; {m['selected_planned_invocations']} planned invocations; "
              f"{m['selected_unplanned_source_skips']} unplanned sources; {m['executed_invocations']} executed; "
              f"{m['plan_eligible_invocations']} eligible plan-only; statuses={m['by_status']}")
    if pairs:print('advisory per-ID comparisons:',report['pairwise_counts'])
    if probe_errors:print('compiler identity failures:',probe_errors,file=sys.stderr)
    hard=changed or bool(input_errors) or bool(probe_errors) or any(r['status'] in schema.TERMINAL|{'FAIL','XPASS'} for r in observations['candidate'])
    if args.gcc_cmd:hard=hard or any(r['status'] in schema.TERMINAL for r in observations['reference'])
    return 1 if hard else 0


if __name__=='__main__':raise SystemExit(main())
