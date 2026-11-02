"""Strict, complete versioned contracts. No tool discovery at import time."""
from __future__ import annotations

import hashlib
import json
import math
from pathlib import Path,PurePosixPath
import re

VERSION='lccc-c-corpus-v2'
REPORT_VERSION='lccc-c-corpus-report-v2'
TERMINAL={'CRASH','TIMEOUT','ERROR'}
NONEXECUTED={'UNSUPPORTED','SKIP'}
OUTCOMES=TERMINAL|NONEXECUTED|{'ACCEPT','REJECT'}
VERDICTS=TERMINAL|NONEXECUTED|{'PASS','FAIL','XFAIL','XPASS'}
SHA256=re.compile(r'^[0-9a-f]{64}$');SHA1=re.compile(r'^[0-9a-f]{40}$')
KINDS={'error','warning','note','remark','no-diagnostics'}


class SchemaError(ValueError):pass


def file_hash(path: Path):
    h=hashlib.sha256()
    with path.open('rb') as f:
        for block in iter(lambda:f.read(1048576),b''):h.update(block)
    return h.hexdigest()


def _pairs(pairs):
    result={}
    for k,v in pairs:
        if k in result:raise SchemaError(f'duplicate JSON key {k!r}')
        result[k]=v
    return result


def loads(text):
    def invalid(value):raise SchemaError('nonfinite JSON value: '+value)
    def finite_float(value):
        result = float(value)
        if not math.isfinite(result):
            raise SchemaError('nonfinite JSON number: ' + value)
        return result
    return json.loads(text, parse_constant=invalid, parse_float=finite_float,
                      object_pairs_hook=_pairs)


def dumps(value):
    return json.dumps(value,sort_keys=True,separators=(',',':'),ensure_ascii=True,allow_nan=False)+'\n'


def count(value):
    if value is None or value == '':return 1,1
    require(isinstance(value, str), 'occurrence count must be text')
    if not re.fullmatch(r'\d+(?:-\d+|\+)?',value):raise SchemaError('bad occurrence count: '+value)
    if value.endswith('+'):return int(value[:-1]),None
    a,b=(value.split('-') if '-' in value else (value,value));a,b=int(a),int(b)
    if b<a:raise SchemaError('inverted occurrence count')
    return a,b


def require(ok,msg):
    if not ok:raise SchemaError(msg)


def keys(obj,required,optional=()):
    require(isinstance(obj,dict),'object required')
    require(not set(required)-obj.keys(),'missing fields: '+str(sorted(set(required)-obj.keys())))
    require(not obj.keys()-set(required)-set(optional),'unknown control fields: '+str(sorted(obj.keys()-set(required)-set(optional))))


def integer(x,*,minimum=0,nullable=False):
    require((nullable and x is None) or (type(x) is int and x>=minimum),'invalid integer/count')


def strings(x):
    require(isinstance(x,list) and all(isinstance(v,str) and v and '\0' not in v for v in x),'string array required')


def relative(x):
    require(isinstance(x,str) and bool(x),'relative path required')
    p=PurePosixPath(x)
    require(bool(p.parts) and not p.is_absolute() and '..' not in p.parts and '\\' not in x and not re.search(r'[\x00-\x1f\x7f]',x) and str(p)==x,'unsafe/noncanonical path')


def expectation(e):
    keys(e,{'id','prefix','kind','msg','regex_form','count_min','count_max','annotation_line','location','unsupported'})
    require(isinstance(e['id'],str) and e['id'],'expectation ID required')
    require(isinstance(e['prefix'],str) and e['prefix'],'prefix required')
    require(e['kind'] in KINDS and isinstance(e['msg'],str),'invalid diagnostic declaration')
    require(type(e['regex_form']) is bool,'regex_form must be boolean')
    integer(e['count_min']);integer(e['count_max'],nullable=True)
    require(e['count_max'] is None or e['count_max']>=e['count_min'],'inverted count range')
    integer(e['annotation_line'],minimum=1);strings(e['unsupported'])
    keys(e['location'],{'file','line','any_file'})
    require(isinstance(e['location']['file'],str) and '\0' not in e['location']['file'],'diagnostic filename required')
    integer(e['location']['line'],minimum=0,nullable=True)
    require(type(e['location']['any_file']) is bool,'any_file must be boolean')


def invocation(inv,expected):
    keys(inv,{'id','model','command','language','phase','flags','target','prefixes','verify',
              'expect','xfail','expected_ids','ignore_unexpected','unsupported','capabilities'})
    require(isinstance(inv['id'],str) and inv['id'],'invocation ID required')
    require(inv['model'] in {'clang-run','edg-options'},'unknown execution model')
    require(isinstance(inv['command'],str) and inv['command'],'source command required')
    require(inv['language'] in {'c','c++','unsupported'},'unknown language')
    require(inv['phase'] in {'frontend','preprocess','codegen','filecheck','execute','link','unsupported'},'unknown phase')
    require(inv['expect'] in {'accept','reject'},'expected outcome must not encode XFAIL policy')
    for f in ('flags','prefixes','expected_ids','ignore_unexpected','unsupported','capabilities'):strings(inv[f])
    for f in ('prefixes','expected_ids','ignore_unexpected','unsupported','capabilities'):
        require(len(set(inv[f]))==len(inv[f]),'duplicate '+f)
    require(type(inv['verify']) is bool and type(inv['xfail']) is bool,'boolean policy required')
    require(inv['target'] is None or isinstance(inv['target'],str) and inv['target'],'invalid target')
    require(set(inv['ignore_unexpected'])<=KINDS-{'no-diagnostics'},'invalid diagnostic exemption')
    active={e['id'] for e in expected if e['prefix'] in inv['prefixes']}
    require(set(inv['expected_ids'])==active if inv['verify'] else not inv['expected_ids'],'prefix/expectation association mismatch')
    require(inv['verify'] or not inv['ignore_unexpected'],'diagnostic exemption without verification')
    require(inv['phase'] in {'frontend','preprocess'} or inv['unsupported'],'unimplemented phase must be explicit')


def validate(doc,root: Path|None=None):
    keys(doc,{'schema','source','count','copied','skipped','files'})
    require(doc['schema']==VERSION,'unknown corpus schema: explicitly re-mine/migrate v1, never guess')
    keys(doc['source'],{'repository','commit','subtree','subtree_tree','input_blobs_sha256',
                        'license_sha256','miner_sha256','transform','lineage'})
    require(isinstance(doc['source']['commit'],str) and SHA1.fullmatch(doc['source']['commit']) is not None,'full upstream commit required')
    require(isinstance(doc['source']['subtree_tree'],str) and SHA1.fullmatch(doc['source']['subtree_tree']) is not None,'upstream tree identity required')
    for f in ('input_blobs_sha256','license_sha256','miner_sha256'):
        require(isinstance(doc['source'][f],str) and SHA256.fullmatch(doc['source'][f]) is not None,'invalid source digest: '+f)
    relative(doc['source']['subtree'])
    require(isinstance(doc['source']['repository'],str) and bool(doc['source']['repository']) and isinstance(doc['source']['transform'],str),'source description required')
    require(doc['source']['lineage'] in {'git-tree','content-only'},'unknown lineage proof')
    integer(doc['count']);integer(doc['copied']);integer(doc['skipped'])
    require(isinstance(doc['files'],list) and len(doc['files'])==doc['count'],'file denominator mismatch')
    ids=set();inv_ids=set();paths=set();copied=0
    for number,r in enumerate(doc['files']):
        try:
            keys(r,{'id','origin','category','file','sha256','source_blob','source_sha256','source_bytes',
                    'transform','skip_reason','edg','license','expected','invocations','declared_runs'})
            relative(r['id']);relative(r['origin'])
            require(r['origin']==doc['source']['subtree']+'/'+r['id'],'source ID/origin relationship mismatch')
            require(r['id'] not in ids,'duplicate file ID');ids.add(r['id'])
            require(isinstance(r['category'],str) and r['category'],'category required')
            require(isinstance(r['source_blob'],str) and SHA1.fullmatch(r['source_blob']) is not None,'source blob identity required')
            require(r['source_sha256'] is None or isinstance(r['source_sha256'],str) and SHA256.fullmatch(r['source_sha256']) is not None,'invalid source hash')
            integer(r['source_bytes']);integer(r['declared_runs'],minimum=1)
            require(isinstance(r['edg'],dict),'EDG inventory object required')
            keys(r['transform'],{'prefix_lines','decoding'})
            integer(r['transform']['prefix_lines'])
            require(r['transform']['decoding'] in {'utf8','utf8-replacement'},'unknown source transformation')
            keys(r['license'],{'spdx','status','evidence','indicators'})
            require(r['license']['spdx']=='Apache-2.0 WITH LLVM-exception','unrecognized declared license')
            require(r['license']['status'] in {'upstream-subtree-notice','mixed-origin-review'},'unknown rights status')
            strings(r['license']['evidence']);strings(r['license']['indicators'])
            require(isinstance(r['expected'],list) and isinstance(r['invocations'],list),'expectation/invocation arrays required')
            exp_ids=set()
            for e in r['expected']:
                expectation(e);require(e['id'] not in exp_ids,'duplicate expectation ID');exp_ids.add(e['id'])
            for inv in r['invocations']:
                invocation(inv,r['expected'])
                require(inv['id'] not in inv_ids,'duplicate invocation ID');inv_ids.add(inv['id'])
            if r['file'] is None:
                require(isinstance(r['skip_reason'],str) and r['skip_reason'] and r['sha256'] is None and not r['invocations'],'index-only source must be explicitly unplanned')
            else:
                relative(r['file']);require(r['file'] not in paths,'duplicate output path');paths.add(r['file'])
                require(r['invocations'] and len(r['invocations'])==r['declared_runs'],'lost invocation denominator')
                require(isinstance(r['sha256'],str) and SHA256.fullmatch(r['sha256']) is not None,'output hash required')
                require(r['source_sha256'] is not None,'copied source identity required')
                copied+=1
                if root is not None:
                    p=root/r['file']
                    require(not p.is_symlink() and p.is_file(),'missing/symlink fixture')
                    require(root.resolve() in p.resolve().parents,'escaping fixture')
                    require(file_hash(p)==r['sha256'],'fixture SHA-256 mismatch')
        except (SchemaError,TypeError,AttributeError) as exc:
            raise SchemaError(f'file {number} ({r.get("id", "?") if isinstance(r,dict) else "?"}): {exc}') from exc
    require(copied==doc['copied'] and doc['skipped']==doc['count']-copied,'copied/skipped denominator mismatch')
    return doc


def verdict(outcome,expect,diagnostics_ok,xfail=False):
    if outcome in TERMINAL|NONEXECUTED:return outcome
    require(outcome in OUTCOMES and expect in {'accept','reject'},'invalid verdict input')
    success=outcome==expect.upper() and diagnostics_ok
    return ('XPASS' if success else 'XFAIL') if xfail else ('PASS' if success else 'FAIL')


def validate_results(rows):
    """Complete per-invocation evidence, not just a few verdict-shaped fields."""
    require(isinstance(rows, list), 'observation array required')
    required = {'id', 'source_id', 'origin', 'file', 'category', 'planned', 'phase',
                'language', 'expect', 'xfail', 'prefixes', 'source_command', 'flags',
                'target', 'unsupported', 'outcome', 'status', 'detail', 'attempted',
                'spawned', 'executed', 'diagnostics_verified', 'elapsed', 'returncode',
                'cmd', 'translation_key', 'stdout_sha256', 'stderr_sha256',
                'stdout_excerpt', 'stderr_excerpt', 'diagnostic_count', 'verification'}
    ids = set()
    for row in rows:
        keys(row, required)
        require(isinstance(row['id'], str) and row['id'], 'observation ID required')
        require(row['id'] not in ids, 'duplicate observation ID'); ids.add(row['id'])
        relative(row['source_id']); relative(row['origin'])
        require(row['file'] is None or isinstance(row['file'], str), 'invalid source file')
        if row['file'] is not None:
            relative(row['file'])
        for field in ('category', 'source_command', 'detail', 'stdout_excerpt', 'stderr_excerpt'):
            require(isinstance(row[field], str), 'invalid observation text: ' + field)
        for field in ('planned', 'xfail', 'attempted', 'spawned', 'executed', 'diagnostics_verified'):
            require(type(row[field]) is bool, 'observation boolean required: ' + field)
        for field in ('prefixes', 'flags', 'unsupported', 'cmd'):
            strings(row[field])
        require(isinstance(row['outcome'], str) and row['outcome'] in OUTCOMES,
                'invalid outcome')
        require(isinstance(row['status'], str) and row['status'] in VERDICTS,
                'invalid verdict')
        require(row['expect'] in ('accept', 'reject'), 'invalid expected outcome')
        require(row['phase'] in ('frontend', 'preprocess', 'codegen', 'filecheck',
                                'execute', 'link', 'unsupported'), 'invalid observed phase')
        require(row['language'] in ('c', 'c++', 'unsupported'), 'invalid observed language')
        require(row['target'] is None or isinstance(row['target'], str) and row['target'],
                'invalid observed target')
        require(row['returncode'] is None or type(row['returncode']) is int,
                'raw return code must be integer or null')
        for field in ('translation_key', 'stdout_sha256', 'stderr_sha256'):
            value = row[field]
            require(value is None or isinstance(value, str) and SHA256.fullmatch(value),
                    'invalid observation digest: ' + field)
        require(type(row['elapsed']) in (int, float) and math.isfinite(row['elapsed'])
                and row['elapsed'] >= 0, 'invalid elapsed time')
        integer(row['diagnostic_count'])
        require(not row['spawned'] or row['attempted'] and bool(row['cmd']),
                'spawn without command/attempt evidence')
        require(row['executed'] == (row['spawned'] and row['outcome'] not in NONEXECUTED),
                'executed-coverage invariant failed')
        require(row['outcome'] not in {'ACCEPT', 'REJECT'} or row['executed'],
                'clean outcome without execution')
        require(row['planned'] or row['file'] is None and row['outcome'] == 'SKIP',
                'unplanned source cannot execute or certify a verdict')
        if row['executed']:
            require(row['stdout_sha256'] is not None and row['stderr_sha256'] is not None,
                    'executed process must retain output identities')
        verification = row['verification']
        if verification is not None:
            keys(verification, {'ok', 'detail', 'match_count', 'missing_count',
                                'unexpected_count', 'matches', 'missing', 'unexpected'})
            require(type(verification['ok']) is bool and isinstance(verification['detail'], str),
                    'invalid diagnostic verification policy')
            for field in ('match_count', 'missing_count', 'unexpected_count'):
                integer(verification[field])
            for field in ('matches', 'missing', 'unexpected'):
                require(isinstance(verification[field], list), 'invalid verification evidence array')
            require(row['executed'], 'diagnostic policy without execution')
        require(not row['diagnostics_verified'] or row['executed']
                and row['outcome'] in {'ACCEPT', 'REJECT'} and verification is not None
                and verification['ok'], 'diagnostic certificate without clean verified evidence')
        if row['outcome'] in TERMINAL | NONEXECUTED:
            require(row['status'] == row['outcome'], 'terminal/nonexecuted result laundered')
        else:
            expected = verdict(row['outcome'], row['expect'],
                               verification is None or verification['ok'], row['xfail'])
            require(row['status'] == expected, 'verdict does not follow its recorded policy')
