#!/usr/bin/env python3
"""Verify retained five-pair program evidence, not authenticity or fresh execution.

The runtime archive is self-contained: binaries, Callgrind counters, process
streams and sidecars. Large compiler-parent profiles are a separate attachment.
"""
import argparse
import gzip
import hashlib
import io
import json
import math
import statistics
import re
import lzma
from pathlib import Path, PurePosixPath
import sys
import tarfile

sys.path.insert(0,str(Path(__file__).resolve().parent))
import verify_evidence_hashes as integrity
import callgrind_ab as cg

ROOT=Path(__file__).resolve().parents[1]
DEFAULT='engineering/evidence/pr834-corpus.json'
META_LIMIT=16*1024*1024


def require(condition,message):
    if not condition:raise ValueError(message)


def sha(path):
    with path.open('rb') as stream:return hashlib.file_digest(stream,'sha256').hexdigest()


def load(stream,limit=META_LIMIT):
    raw=stream.read(limit+1)
    require(len(raw)<=limit,'metadata limit exceeded')
    return json.loads(raw,object_pairs_hook=integrity._pairs)


def verify(root,configuration,check_inputs=False):
    root=Path(root).resolve()
    with Path(configuration).open('rb') as stream:config=load(stream)
    integrity._keys(config,{'schema_version','archive','archive_sha256','inventory','inventory_sha256','forms','controller','controller_sha256','compiler_inputs','geometry'})
    integrity._version(config['schema_version'])
    for key in ('archive','inventory','controller'):
        integrity._hex(config[key+'_sha256'])
        require(sha(integrity.contained(root,config[key]))==config[key+'_sha256'],f'{key} digest mismatch')
    require(isinstance(config['compiler_inputs'],dict) and bool(config['compiler_inputs']),'missing compiler input fingerprint')
    for name,digest in config['compiler_inputs'].items():
        integrity.safe_name(name);integrity._hex(digest)
        if check_inputs:require(sha(integrity.contained(root,name))==digest,f'current compiler input differs: {name}')
    integrity._keys(config['geometry'],{'I1','D1','LL'})
    require(all(isinstance(v,str) and re.fullmatch(r'[0-9]+,[0-9]+,[0-9]+',v) for v in config['geometry'].values()),'invalid geometry')
    forms=config['forms'];require(isinstance(forms,dict) and len(forms)==45,'expected exactly 45 captured forms')
    for name,form in forms.items():
        integrity.safe_name(name,leaf=True)
        integrity._keys(form,{'stem','source_sha256','flags'})
        integrity.safe_name(form['stem'],leaf=True);integrity._hex(form['source_sha256']);integrity._text(form['flags'])
    with gzip.open(integrity.contained(root,config['inventory']),'rb') as stream:inventory=load(stream)
    integrity._keys(inventory,{'schema_version','archive','archive_sha256','mode','files','original_root','verification'})
    integrity._version(inventory['schema_version'])
    require(PurePosixPath(inventory['original_root']).is_absolute(),'original root must be absolute')
    require(inventory['archive']==Path(config['archive']).name,'archive filename mismatch')
    require(inventory['mode']=='runtime','not a runtime evidence archive')
    require(inventory['archive_sha256']==config['archive_sha256'],'inventory/archive mismatch')
    require(isinstance(inventory['files'],list) and bool(inventory['files']),'empty inventory')
    expected={}
    for row in inventory['files']:
        integrity._keys(row,{'path','bytes','sha256'});integrity.safe_name(row['path']);integrity._hex(row['sha256'])
        require(type(row['bytes']) is int and 0<=row['bytes']<=256*1024*1024,'invalid file size')
        require(row['path'] not in expected,'duplicate inventory path');expected[row['path']]=row
    require(sum(r['bytes'] for r in expected.values())<=1024**3,'archive decoded scope exceeds 1 GiB')
    require(sum(r['bytes'] for n,r in expected.items() if n.endswith('.json'))<=64*1024*1024,'metadata scope exceeds 64 MiB')
    verified={};documents={};counters={};geometries={}
    with tarfile.open(integrity.contained(root,config['archive']),'r|xz') as archive:
        for member in archive:
            name=integrity.safe_name(member.name)
            require(name in expected and name not in verified,'extra/duplicate archive member')
            row=expected[name]
            if member.islnk():
                require(member.linkname in verified,'invalid/forward hardlink')
                digest,size=verified[member.linkname]
                if member.linkname in documents:documents[name]=documents[member.linkname]
                if member.linkname in counters:counters[name]=counters[member.linkname];geometries[name]=geometries[member.linkname]
            else:
                require(member.isfile() and member.size==row['bytes'],'invalid member type/size')
                keep=name.endswith(('.json','.cg'))
                require(not keep or member.size<=META_LIMIT,'counter/metadata member too large')
                digest=hashlib.sha256();payload=bytearray();size=0
                with archive.extractfile(member) as stream:
                    while block:=stream.read(integrity.CHUNK):
                        digest.update(block);size+=len(block)
                        if keep:payload.extend(block)
                digest=digest.hexdigest()
                if name.endswith('.json'):documents[name]=load(io.BytesIO(payload))
                if name.endswith('.cg'):
                    events=[];summary=[];geometries[name]={}
                    for line in payload.decode().splitlines():
                        match=re.fullmatch(r'desc: (I1|D1|LL) cache: ([0-9]+) B, ([0-9]+) B, ([0-9]+)-way associative',line)
                        if match:geometries[name][match[1]]=f'{match[2]},{match[4]},{match[3]}'
                        if line.startswith('events:'):events.append(line.split()[1:])
                        elif line.startswith('summary:'):summary.append(line.split()[1:])
                    require(len(events)==len(summary)==1,'missing/duplicate Callgrind events/summary')
                    require(len(events[0])==len(summary[0]) and len(set(events[0]))==len(events[0]),'invalid counter cardinality')
                    counters[name]=cg.validate_events(dict(zip(events[0],map(int,summary[0]))))
            require((digest,size)==(row['sha256'],row['bytes']),f'member digest/size mismatch: {name}')
            verified[name]=(digest,size)
    require(set(verified)==set(expected),'missing archive member')
    return _verify_dataset(documents,counters,geometries,verified,config,inventory,check_inputs)


def _verify_dataset(documents,counters,geometries,verified,config,inventory,check_inputs):
    """Cross-check bounded decoded metadata after byte-integrity verification."""
    forms=config['forms']
    before=documents['identities-before.json'];after=documents['identities-after.json']
    require(before==after,'measurement identities changed')
    require(not before['candidate']['status'] and not before['baseline']['status'],'dirty measured tree')
    require(before['controller_sha256']==config['controller_sha256'],'wrong controller')
    def relative(path):
        name=PurePosixPath(path).relative_to(PurePosixPath(inventory['original_root'])).as_posix()
        integrity.safe_name(name);require(name in verified,f'uncovered receipt path: {name}')
        return name
    receipt_count=0
    def receipt(info):
        nonlocal receipt_count
        process=info['process']
        require(type(process['returncode']) is int and process['returncode']==0 and not process['failure'],'unsuccessful process in comparable evidence')
        require(type(process['elapsed']) in (int,float) and math.isfinite(process['elapsed']) and process['elapsed']>=0,'invalid process elapsed time')
        for stream in ('stdout','stderr'):
            name=relative(info['raw_outputs'][stream])
            require(verified[name][0]==process[stream+'_sha256'],'raw stream digest mismatch')
        if 'receipt' in info:
            name=relative(info['receipt']['path'])
            require(verified[name][0]==info['receipt']['sha256'],'sidecar digest mismatch')
            require(documents[name]=={k:v for k,v in info.items() if k!='receipt'},'sidecar/manifest content mismatch')
        receipt_count+=1
    ratios=[];equal={name:True for name in forms}
    for round_no in range(1,6):
        prefix=f'bounded-{round_no:02}'
        index=documents[prefix+'/index.json']
        require(index['complete'] is True and not index['failures'],'incomplete corpus round')
        require(set(index['selected'])==set(index['results'])==set(forms),'incomplete form coverage')
        for name,form in forms.items():
            manifest=documents[f'{prefix}/{name}/manifest.json'];entry=manifest['results'][form['stem']]
            require(manifest['comparable']==manifest['selected']==1 and not manifest['failures'],'invalid comparison count')
            require(entry['status']=='COMPARABLE-SIMULATION','comparison not successful')
            require(manifest['geometry']==config['geometry'],'manifest geometry differs')
            require(manifest['opt']==form['flags'] and entry['source_sha256']==form['source_sha256'],'input/flags mismatch')
            require(len(manifest['paths']['mine'].encode())==len(manifest['paths']['ref'].encode()),'unequal output path lengths')
            values={}
            for arm,identity in [('mine','candidate' if round_no%2 else 'baseline'),('ref','baseline' if round_no%2 else 'candidate')]:
                require(manifest['tools'][arm]['sha256']==before[identity]['binary_sha256'],'compiler arm identity mismatch')
                binary=manifest['paths'][arm]+'/'+form['stem'];binary_name=relative(binary)
                require(verified[binary_name][0]==entry['binary_sha256'][arm],'binary digest mismatch')
                for phase in ('compilation','native','instrumentation'):receipt(entry[phase][arm])
                require(entry['native'][arm]['command']==[binary],'native executable mismatch')
                instrument=entry['instrumentation'][arm]
                require(instrument['command'][-1]==binary,'instrumented executable mismatch')
                counter=relative(binary+'.cg')
                require(verified[counter][0]==instrument['counter_file_sha256'],'counter digest mismatch')
                require(geometries[counter]==config['geometry'],'raw geometry differs')
                for cache,geometry in config['geometry'].items():
                    require(instrument['command'].count(f'--{cache}={geometry}')==1,'geometry argv differs')
                require(cg.validate_events(entry[arm])==counters[counter],'reported/raw counters differ')
                require(entry['native'][arm]['process']['stdout_sha256']==instrument['process']['stdout_sha256'],'instrumented output differs')
                values[identity]=counters[counter]['Ir']
            require(entry['native']['mine']['process']['stdout_sha256']==entry['native']['ref']['process']['stdout_sha256'],'A/B output mismatch')
            require(entry['metrics']==cg.compare_events(entry['ref'],entry['mine']),'derived metrics differ from raw counters')
            equal[name] &= entry['binary_sha256']['mine']==entry['binary_sha256']['ref']
            ratios.append(values['candidate']/values['baseline'])
    costs=documents['costs.json']
    require(costs['complete'] is True and not costs['failures'] and len(costs['rows'])==225,'incomplete compiler timings')
    seen=set()
    for row in costs['rows']:
        key=(row['round'],row['name']);require(key not in seen and key[0] in range(1,6) and key[1] in forms,'duplicate/invalid compiler timing')
        seen.add(key)
        for arm in ('candidate','baseline'):
            records=row['arms'][arm]['records'];require(len(records)==5,'expected five retained invocations per batch')
            require(row['arms'][arm]['batch_mean_seconds']==statistics.mean(r['receipt']['process']['elapsed'] for r in records),'batch mean differs from raw receipts')
            for record in records:
                require(record['error'] is None,'failed compiler timing');receipt(record['receipt'])
                command=record['receipt']['command'];binary=command[command.index('-o')+1]
                require(verified[relative(binary)][0]==record['elf_sha256'],'timed binary digest mismatch')
    return {'files':len(verified),'program_pairs':len(ratios),'linked_process_receipts':receipt_count,'program_Ir_range':[min(ratios),max(ratios)],'ELF_equal_all_rounds':sum(equal.values()),'compiler_inputs_checked':check_inputs,'compiler_profiles_included':False}


def main(argv=None):
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--root',type=Path,default=ROOT)
    p.add_argument('--manifest',default=DEFAULT);p.add_argument('--check-current-inputs',action='store_true');a=p.parse_args(argv)
    try:result=verify(a.root,integrity.contained(a.root,a.manifest),a.check_current_inputs)
    except (OSError,ValueError,KeyError,TypeError,EOFError,tarfile.TarError,lzma.LZMAError) as error:
        print('corpus-receipts: FAIL:',error,file=sys.stderr);return 1
    print('corpus-receipts: PASS',json.dumps(result,sort_keys=True));return 0


if __name__=='__main__':raise SystemExit(main())
