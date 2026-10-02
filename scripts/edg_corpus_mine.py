#!/usr/bin/env python3
"""Source-pinned EDG C inventory and conservative Clang RUN declarations.

Default execution model for imported Clang sources is each actual Clang RUN,
not EDG option-set positions. Fixtures are never reformatted during migration.
The v2 index is strict compact JSON with finite/null count bounds, exhaustive
schema validation, explicit transformations and support reasons. GNU output is
an inventory of names/directive facts, never GNU test bodies or execution claims.
"""
from __future__ import annotations

import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import re
import shutil
import shlex
import subprocess
import sys
import tempfile
import urllib.request

REPO=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(REPO))
from tools.corpus import diagnostics,directives,publication,schema

SPDX=('// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception\n'
      '// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH\n'
      '// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.\n')
CLANG_C_REL=Path('tests/tests/imported/clang/c')
GNU_C_REL=Path('tests/tests/imported/gnu/c')
SIZE_CAP_BYTES=256*1024
GIANT_RE=re.compile(r'^(arm_sme|arm_sve|arm_cde|arm_mve|arm_neon)_')
PIN='0ac366374c06e54612ddbac397c74ffa40e5182c'
PIN_TREE='6626681cf444dc0bd258aef2bb3aafbfd0f162ae'
CLANG_NOTICE_SHA='8d85c1057d742e597985c7d4e6320b015a9139385cff4cbae06ffc0ebe89afee'
LOSSY_SOURCE_SHA='bec9ccd04a910598f582b0ee6274ed63c29c20259800a0bba2118a89d281a34c'
LOSSY_BODY_SHA='8868bab9eb11840c2bb9e0029723f8d8e7a8fbf14942fb5d7156dffe5a8ff8d2'
STANDARDS={'--c89':'c90','--c90':'c90','--c99':'c99','--c11':'c11',
           '--c17':'c17','--c18':'c17','--c2x':'c23','--c23':'c23'}
EDG_TO_GCC={k:['-std='+v] for k,v in STANDARDS.items()}
EDG_TO_GCC.update({'--strict':['-pedantic-errors'],'--strict_ansi':['-pedantic-errors'],
                  '--no_warnings':['-w'],'--warnings_are_errors':['-Werror'],'--c':['-x','c']})
EDG_NOOP_FLAGS={}  # Version/compatibility controls are semantic, not no-ops.
parse_count=schema.count


def template_to_regex(msg):
    """Compatibility API; -re subset implementation lives in one module."""
    return diagnostics.template_regex(msg).pattern


def translate_options(option_set):
    flags=[];arch=[];unknown=[];standard=None
    try:tokens=shlex.split(option_set)
    except ValueError as exc:return [],[],['tokenization: '+str(exc)]
    i=0
    while i<len(tokens):
        token=tokens[i];i+=1
        if token.startswith('%-'):token=token[1:]
        if token in ('--target','-target','--march','-march'):
            if i<len(tokens):arch.append(tokens[i]);i+=1
            else:unknown.append(token+' <missing operand>')
        elif re.match(r'^(?:--target|-target|--march|-march)=',token):arch.append(token.split('=',1)[1])
        elif token in STANDARDS:standard=STANDARDS[token]
        elif token in EDG_TO_GCC:flags.extend(EDG_TO_GCC[token])
        elif token in ('-D','-U','-I','-include','--include'):
            if i<len(tokens):flags.extend(['-include' if token=='--include' else token,tokens[i]]);i+=1
            else:unknown.append(token+' <missing operand>')
        elif token.startswith(('-D','-U','-I')):flags.append(token)
        elif token in ('--gnu_version','--param','--sysroot','-isystem'):
            if i<len(tokens):unknown.append(token+' '+tokens[i]);i+=1
            else:unknown.append(token+' <missing operand>')
        else:unknown.append(token)
    if standard:flags=[f for f in flags if not f.startswith('-std=')]+['-std='+standard]
    return flags,arch,unknown


def parse_sft(path):
    parsed=directives.parse_text(path.read_text(encoding='utf-8',errors='replace'))
    edg=parsed['edg'];sets=[s.strip() for s in edg.get('options','').split(edg.get('options_sep',':') or ':')] if 'options' in edg else []
    translated=[translate_options(edg.get('options_all','')+' '+s) for s in sets]
    info=dict(parsed,invocations=directives.build_invocations(path.name,parsed,translate_options),type=edg.get('type','cp'),option_sets=sets,options_all=shlex.split(edg.get('options_all','')),
              languages_per_set=['c++' if re.search(r'--c\+\+|--cpp|--cp\b',s) else 'c' for s in sets],
              gcc_flags_per_set=[t[0] for t in translated],arch_tags=sorted({a for t in translated for a in t[1]}),
              untranslated_edg_flags=sorted({u for t in translated for u in t[2]}))
    text=path.read_text(encoding='utf-8',errors='replace')
    info.update(cases=edg.get('cases'),has_main=bool(re.search(r'\bmain\s*\(',text)),
                test_number_uses=text.count('TEST_NUMBER'),contains_expanded_system_headers='/usr/include' in text)
    return info


def miner_identity():
    h=hashlib.sha256()
    for name in ['scripts/edg_corpus_mine.py','tools/corpus/directives.py','tools/corpus/schema.py','tools/corpus/diagnostics.py']:
        h.update(name.encode()+b'\0'+(REPO/name).read_bytes()+b'\0')
    return h.hexdigest()


def blob(data):return hashlib.sha1(b'blob '+str(len(data)).encode()+b'\0'+data).hexdigest()


def _record(origin,category,data,output,file_path,*,commit,source_blob,prefix_lines,edg_facts=None,skip_reason=None):
    original=data.decode('utf-8',errors='replace') if data is not None else ''
    parsed=directives.parse_text(original,prefix_lines=prefix_lines) if data is not None else None
    if parsed is None:parsed=dict(edg=edg_facts or {},runs=[],controls=[],expected=[],conditional_unterminated=False)
    file_id=origin.removeprefix(str(CLANG_C_REL)+'/')
    inv=directives.build_invocations(file_id,parsed,translate_options) if file_path else []
    indicators=['usr-include-text'] if '/usr/include' in original else []
    decoding='utf8'
    if data is not None:
        try:data.decode('utf-8')
        except UnicodeDecodeError:decoding='utf8-replacement'
    for i in inv:
        if decoding!='utf8':i['unsupported'].append('lossy upstream source decoding')
        if indicators:i['unsupported'].append('mixed-origin indicators require rights review')
    return dict(id=file_id,origin=origin,category=category,file=file_path,
                sha256=hashlib.sha256(output).hexdigest() if output is not None else None,
                source_blob=source_blob,source_sha256=hashlib.sha256(data).hexdigest() if data is not None else None,
                source_bytes=len(data) if data is not None else 0,transform=dict(prefix_lines=prefix_lines,decoding=decoding),
                skip_reason=skip_reason,edg=parsed['edg'],
                license=dict(spdx='Apache-2.0 WITH LLVM-exception',
                             status='mixed-origin-review' if indicators else 'upstream-subtree-notice',
                             evidence=[f'edgcpp/compiler@{commit}:tests/tests/imported/clang/LICENSE.txt','upstream-notice-sha256:'+CLANG_NOTICE_SHA],indicators=indicators),
                expected=parsed['expected'] if file_path else [],invocations=inv,
                declared_runs=len(inv) if file_path else max(1,len(parsed['runs'])))


def document(records,source):
    return dict(schema=schema.VERSION,source=source,count=len(records),
                copied=sum(r['file'] is not None for r in records),skipped=sum(r['file'] is None for r in records),files=records)


def validate_tree_manifest(tree):
    """Recompute Git tree objects; don't trust a SHA label on arbitrary JSON."""
    schema.require(tree.get('commit')==PIN and tree.get('tree_id',tree.get('sha'))==PIN_TREE and tree.get('truncated') is False,'unrecognized/truncated source tree')
    entries={};children={}
    for entry in tree['tree']:
        path=entry['path'];schema.relative(path);schema.require(path not in entries,'duplicate source tree path')
        kind=entry['type'];mode=entry['mode'];oid=entry['sha']
        schema.require(kind in {'tree','blob'} and schema.SHA1.fullmatch(oid) is not None,'invalid source object')
        schema.require(mode in ({'040000'} if kind=='tree' else {'100644','100755','120000'}),'invalid Git tree mode')
        entries[path]=entry;parent,name=path.rsplit('/',1) if '/' in path else ('',path)
        children.setdefault(parent,[]).append((name,entry))
    for path,e in entries.items():
        if e['type']=='tree' and path not in children:
            schema.require(e['sha']==hashlib.sha1(b'tree 0\0').hexdigest(),'missing source directory children: '+path)
    schema.require(all(not d or d in entries and entries[d]['type']=='tree' for d in children),'orphan source tree path')
    for directory in sorted(children,key=lambda p:p.count('/'),reverse=True):
        body=b''
        for name,e in sorted(children[directory],key=lambda pair:(pair[0]+('/' if pair[1]['type']=='tree' else '')).encode()):
            body+=e['mode'].lstrip('0').encode()+b' '+name.encode()+b'\0'+bytes.fromhex(e['sha'])
        value=hashlib.sha1(b'tree '+str(len(body)).encode()+b'\0'+body).hexdigest()
        schema.require(value==(entries[directory]['sha'] if directory else PIN_TREE),'Git tree object mismatch: '+directory)
    schema.require('' in children and sum(e['type']=='blob' for e in entries.values())==4635,'lost pinned source-tree denominator')
    return entries


def upgrade_index(index: Path,tree: dict,*,fetch_source=None):
    """Explicit v1 migration. This is the ONLY reader that admits legacy Infinity.

    Original fixtures are checked before and after publication. Source lineage
    is verified against pinned upstream Git blob identities, not output hashes.
    A lossy UTF-8 import is recorded/unsupported, never disguised by rewriting it.
    """
    validate_tree_manifest(tree)
    commit=tree['commit'];schema.require(schema.SHA1.fullmatch(commit) is not None,'invalid source pin')
    text=index.read_text(encoding='utf-8')
    old=json.loads(text,object_pairs_hook=schema._pairs,parse_constant=lambda v: None if v=='Infinity' else (_ for _ in ()).throw(ValueError(v)))
    if old.get('schema')==schema.VERSION:schema.validate(schema.loads(text),index.parent.parent)
    lookup={r['path']:r for r in tree['tree'] if r['type']=='blob'}
    records=[];before={}
    for r in old['files']:
        relative=r['origin'].removeprefix(str(CLANG_C_REL)+'/');entry=lookup[relative]
        if r.get('file'):
            path=index.parent.parent/r['file'];raw=path.read_bytes()
            schema.require(hashlib.sha256(raw).hexdigest()==r['sha256'],'legacy fixture hash mismatch: '+r['file'])
            schema.require(raw.startswith(SPDX.encode()),'unknown legacy attribution prefix: '+r['file'])
            body=raw[len(SPDX.encode()):];data=body
            if blob(body)!=entry['sha']:
                if fetch_source is None:raise schema.SchemaError('source transformation requires pinned original bytes: '+relative)
                data=fetch_source(commit,r['origin'])
                schema.require(blob(data)==entry['sha'],'upstream source blob mismatch')
                schema.require(data.decode('utf-8',errors='replace').encode()==body,'unexplained source transformation')
            before[r['file']]=r['sha256']
            rec=_record(r['origin'],r['category'],data,raw,r['file'],commit=commit,
                        source_blob=entry['sha'],prefix_lines=3,skip_reason=r.get('skip_reason'))
        else:
            rec=_record(r['origin'],r['category'],None,None,None,commit=commit,source_blob=entry['sha'],
                        prefix_lines=3,edg_facts=r.get('edg') or dict(type=r.get('type','cp'),option_sets=r.get('option_sets',[])),
                        skip_reason=r.get('skip_reason','index-only'))
            rec['source_bytes']=entry['size'];rec['declared_runs']=r.get('declared_runs',max(1,len(r.get('run_commands',[]))))
            if r.get('contains_expanded_system_headers') or r.get('license',{}).get('indicators'):
                rec['license']['indicators']=['usr-include-text'];rec['license']['status']='mixed-origin-review'
        records.append(rec)
    inputs='\n'.join(f"{r['id']}\0{r['source_blob']}\0{r['source_bytes']}" for r in records).encode()
    source=dict(repository='https://github.com/edgcpp/compiler',commit=commit,subtree=str(CLANG_C_REL),
                subtree_tree=tree['tree_id'],input_blobs_sha256=hashlib.sha256(inputs).hexdigest(),
                license_sha256=schema.file_hash(REPO/'third_party_licenses/EDG-Clang-Apache-2.0-LLVM.txt'),
                miner_sha256=miner_identity(),transform='declared attribution-prefix plus per-file UTF-8 decoding',lineage='git-tree')
    doc=document(records,source);schema.validate(doc,index.parent.parent)
    with publication.lock(index.parent/'.index.lock'):
        for name,value in before.items():schema.require(schema.file_hash(index.parent.parent/name)==value,'fixture changed during migration')
        publication.atomic_bytes(index,schema.dumps(doc).encode())
    return doc


def verify_index(index: Path):
    doc=schema.validate(schema.loads(index.read_text(encoding='utf-8')),index.parent.parent)
    schema.require(doc['source']['commit']==PIN and doc['source']['subtree_tree']==PIN_TREE,'unrecognized upstream pin')
    schema.require(doc['source']['miner_sha256']==miner_identity(),'stale generated declarations: regenerate with the current miner')
    schema.require(doc['source']['license_sha256']==schema.file_hash(REPO/'third_party_licenses/EDG-Clang-Apache-2.0-LLVM.txt'),'license notice identity changed')
    inputs='\n'.join(f"{r['id']}\0{r['source_blob']}\0{r['source_bytes']}" for r in doc['files']).encode()
    schema.require(hashlib.sha256(inputs).hexdigest()==doc['source']['input_blobs_sha256'],'source inventory identity mismatch')
    for r in doc['files']:
        if not r['file']:continue
        raw=(index.parent.parent/r['file']).read_bytes();prefix=r['transform']['prefix_lines']
        schema.require(prefix==3 and raw.startswith(SPDX.encode()),'unknown source prefix transformation')
        body=raw[len(SPDX.encode()):]
        if r['transform']['decoding']=='utf8':
            schema.require(blob(body)==r['source_blob'] and hashlib.sha256(body).hexdigest()==r['source_sha256'],'source/output lineage mismatch')
        else:
            schema.require(r['id']=='C/C23/n2927.sft.c' and r['source_sha256']==LOSSY_SOURCE_SHA and hashlib.sha256(body).hexdigest()==LOSSY_BODY_SHA,'unrecognized lossy source transformation')
            schema.require(all('lossy upstream source decoding' in i['unsupported'] for i in r['invocations']),'lossy fixture was admitted')
        parsed=directives.parse_text(body.decode('utf-8'),prefix_lines=prefix)
        inv=directives.build_invocations(r['id'],parsed,translate_options)
        for i in inv:
            if r['transform']['decoding']!='utf8':i['unsupported'].append('lossy upstream source decoding')
            if r['license']['indicators']:i['unsupported'].append('mixed-origin indicators require rights review')
        schema.require(inv==r['invocations'] and parsed['expected']==r['expected'],'stale/corrupted late declaration: '+r['id'])
    return doc


def mine_clang_c(edg_root,out_root,*,force=False,index_only=False,include_giants=False,emit_sidecars=False):
    src=edg_root/CLANG_C_REL;candidates=sorted(src.rglob('*.sft.c')) if src.is_dir() else []
    if not candidates:
        print('error: empty EDG C input; live corpus untouched',file=sys.stderr);return 3
    try:
        commit=subprocess.check_output(['git','-C',str(edg_root),'rev-parse','HEAD'],text=True,timeout=20).strip()
        tree_id=subprocess.check_output(['git','-C',str(edg_root),'rev-parse',f'HEAD:{CLANG_C_REL}'],text=True,timeout=20).strip()
        if subprocess.check_output(['git','-C',str(edg_root),'status','--porcelain','--',str(CLANG_C_REL)],text=True,timeout=20):
            raise schema.SchemaError('source subtree is dirty; refuse false source pin')
        schema.require(commit==PIN and tree_id==PIN_TREE,'fresh mining requires the pinned EDG commit/subtree')
        notice=(edg_root/'tests/tests/imported/clang/LICENSE.txt').read_bytes()
        schema.require(hashlib.sha256(notice).hexdigest()==CLANG_NOTICE_SHA,'upstream subtree notice changed')
        schema.require(notice in (REPO/'third_party_licenses/EDG-Clang-Apache-2.0-LLVM.txt').read_bytes(),'upstream subtree notice is not retained')
        tracked=subprocess.check_output(['git','-C',str(edg_root),'ls-tree','-r','-z','HEAD','--',str(CLANG_C_REL)],timeout=20)
        tracked_blobs={}
        for row in tracked.split(b'\0'):
            if not row:continue
            meta,name=row.split(b'\t',1);mode,kind,oid=meta.split()
            if name.endswith(b'.sft.c'):tracked_blobs[name.decode()]=(mode.decode(),oid.decode())
        schema.require({str(p.relative_to(edg_root)) for p in candidates}==set(tracked_blobs),'source candidates differ from the pinned Git tree')
    except (OSError,subprocess.SubprocessError,schema.SchemaError) as exc:
        print('error: pinned source required: '+str(exc),file=sys.stderr);return 2
    out_root.mkdir(parents=True,exist_ok=True)
    live=out_root/'clang-c'
    with tempfile.TemporaryDirectory(prefix='.clang-c-stage-',dir=out_root) as td:
        stage=Path(td);records=[]
        for path in candidates:
            if path.is_symlink():raise schema.SchemaError('source symlink not admitted')
            data=path.read_bytes();schema.require(tracked_blobs[str(path.relative_to(edg_root))][0]=='100644' and blob(data)==tracked_blobs[str(path.relative_to(edg_root))][1],'source body differs from pinned Git blob');rel=path.relative_to(src);stem=path.name.removesuffix('.sft.c')
            reason='index-only' if index_only else 'giant-generated' if GIANT_RE.match(stem) and not include_giants else 'size-cap' if len(data)>SIZE_CAP_BYTES and not include_giants else None
            output=SPDX.encode()+data.decode('utf-8',errors='replace').encode() if reason is None else None
            file=str(Path('clang-c')/rel.parent/(stem+'.c')) if reason is None else None
            rec=_record(str(CLANG_C_REL/rel),rel.parts[0] if len(rel.parts)>1 else 'top',data,output,file,
                        commit=commit,source_blob=blob(data),prefix_lines=3,skip_reason=reason)
            if file:
                dest=stage/rel.parent/(stem+'.c');dest.parent.mkdir(parents=True,exist_ok=True);dest.write_bytes(output)
                if emit_sidecars:dest.with_suffix('.c.meta.json').write_text(schema.dumps(rec))
            records.append(rec)
        inputs='\n'.join(f"{r['id']}\0{r['source_blob']}\0{r['source_bytes']}" for r in records).encode()
        doc=document(records,dict(repository='https://github.com/edgcpp/compiler',commit=commit,subtree=str(CLANG_C_REL),
                     subtree_tree=tree_id,input_blobs_sha256=hashlib.sha256(inputs).hexdigest(),
                     license_sha256=schema.file_hash(REPO/'third_party_licenses/EDG-Clang-Apache-2.0-LLVM.txt'),miner_sha256=miner_identity(),
                     transform='declared attribution-prefix plus per-file UTF-8 decoding',lineage='git-tree'))
        schema.validate(doc)
        (stage/'corpus-index.json').write_text(schema.dumps(doc),encoding='utf-8')
        (stage/'LICENSE-NOTICE.txt').write_text('Upstream subtree notice: tests/tests/imported/clang/LICENSE.txt\n'
            'Apache-2.0 WITH LLVM-exception and the retained legacy NCSA terms.\n'
            'Per-file source identity, transformations and review indicators are in corpus-index.json.\n'
            'Indicators are not legal clearance; mixed-origin material requires maintainer review.\n')
        # Validate copied identities against the staged generation itself.
        for r in records:
            if r['file']:schema.require(schema.file_hash(stage/Path(r['file']).relative_to('clang-c'))==r['sha256'],'staged hash mismatch')
        with publication.lock(out_root/'.corpus-publish.lock'):
            publication.publish_directory(stage,live,force=force)
        # On exchange, stage holds the old complete tree. Removing it occurs
        # only AFTER the live path points atomically to the validated new tree.
    print(f"clang-c: {doc['count']} indexed, {doc['copied']} copied, {doc['skipped']} unplanned source skips")
    return 0


def mine_gnu_manifest(edg_root,out_root):
    src=edg_root/GNU_C_REL
    if not src.is_dir():return 2
    records=[];types=Counter();categories=Counter()
    for path in sorted(src.rglob('*.sft.c')):
        info=parse_sft(path);category=str(path.relative_to(src).parent)
        records.append(dict(name=path.name.removesuffix('.sft.c'),category=category,type=info['type'],
                            option_sets=info['option_sets'],cases=info['cases'],has_main=info['has_main'],test_number_uses=info['test_number_uses']))
        types[info['type']]+=1;categories[category]+=1
    out_root.mkdir(parents=True,exist_ok=True)
    publication.atomic_bytes(out_root/'gnu-torture-manifest.jsonl',''.join(schema.dumps(r) for r in records).encode())
    publication.atomic_bytes(out_root/'gnu-torture-manifest-summary.json',schema.dumps(dict(count=len(records),by_type=dict(types),by_category=dict(categories),
        scope='names and directive facts only; no source bodies, no GNU execution coverage',source_subtree=str(GNU_C_REL))).encode())
    return 0


def selftest():
    import unittest
    class Contracts(unittest.TestCase):
        def test_counts(self):
            for token,want in [(None,(1,1)),('2',(2,2)),('1+',(1,None)),('0-1',(0,1))]:self.assertEqual(schema.count(token),want)
        def test_header_and_literals(self):
            p=directives.parse_text('//type: fp\n/* c */ int x;\n//type: fn\nconst char *s="expected-error {{bad}}";\n')
            self.assertEqual(p['edg']['type'],'fp');self.assertEqual(p['expected'],[])
        def test_invocations(self):
            p=directives.parse_text('//options: --c99:--c11\n// RUN: %clang_cc1 -verify=c99 -std=c99 %s\n// RUN: %clang_cc1 -verify=c11 -std=c11 %s\n// c99-no-diagnostics\n// c11-no-diagnostics\nint x;\n')
            inv=directives.build_invocations('x.sft.c',p,translate_options)
            self.assertEqual([i['prefixes'] for i in inv],[['c99'],['c11']]);self.assertFalse(any(i['unsupported'] for i in inv))
        def test_translation(self):
            flags,arch,unknown=translate_options('--strict %-DNAME="a b" --c99 --c11 --target linux_aarch64')
            self.assertIn('-DNAME=a b',flags);self.assertIn('-std=c11',flags);self.assertIn('-pedantic-errors',flags)
            self.assertEqual(arch,['linux_aarch64']);self.assertFalse(unknown)
        def test_finite_json(self):self.assertEqual(schema.loads(schema.dumps({'max':schema.count('1+')[1]})),{'max':None})
    result=unittest.TextTestRunner().run(unittest.defaultTestLoader.loadTestsFromTestCase(Contracts))
    return 0 if result.wasSuccessful() else 1


def main(argv=None):
    ap=argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument('mode',choices=['clang-c','gnu-manifest','summary','selftest','upgrade-index','verify-index'])
    ap.add_argument('--edg-root',type=Path,default=Path('/opt/lccc-work/edg'))
    ap.add_argument('--out-root',type=Path,default=REPO/'tests/corpus')
    ap.add_argument('--index',type=Path,default=REPO/'tests/corpus/clang-c/corpus-index.json')
    ap.add_argument('--tree-manifest',type=Path)
    ap.add_argument('--force',action='store_true');ap.add_argument('--index-only',action='store_true')
    ap.add_argument('--include-giants',action='store_true');ap.add_argument('--emit-sidecars',action='store_true')
    args=ap.parse_args(argv)
    try:
        if args.mode=='selftest':return selftest()
        if args.mode=='verify-index':
            doc=verify_index(args.index);print(f"strict index: {doc['count']} records / {doc['copied']} hashes / all invocation declarations verified");return 0
        if args.mode=='upgrade-index':
            if not args.tree_manifest:ap.error('--tree-manifest is required for pinned migration')
            tree=schema.loads(args.tree_manifest.read_text());tree['tree_id']=tree.get('tree_id',tree.get('sha'))
            def fetch(commit,path):
                return urllib.request.urlopen(f'https://raw.githubusercontent.com/edgcpp/compiler/{commit}/{path}',timeout=45).read()
            doc=upgrade_index(args.index,tree,fetch_source=fetch);print(f"upgraded strict index: {doc['count']} records / {doc['copied']} unchanged fixtures");return 0
        if args.mode=='clang-c':return mine_clang_c(args.edg_root,args.out_root,force=args.force,index_only=args.index_only,include_giants=args.include_giants,emit_sidecars=args.emit_sidecars)
        if args.mode=='gnu-manifest':return mine_gnu_manifest(args.edg_root,args.out_root)
        for name,subtree in [('clang C',CLANG_C_REL),('GNU C inventory',GNU_C_REL)]:
            print(name,sum(1 for _ in (args.edg_root/subtree).rglob('*.sft.c')))
        return 0
    except (OSError,ValueError,subprocess.SubprocessError) as exc:
        print('corpus miner failed: '+str(exc),file=sys.stderr);return 2


if __name__=='__main__':raise SystemExit(main())
