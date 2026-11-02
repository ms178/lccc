import sys,json,gzip,subprocess
from pathlib import Path
sys.path.insert(0,'scripts');import godbolt as gb
out=Path('engineering/evidence/2026-10-14-auipc-range/vendor-controls');out.mkdir(parents=True,exist_ok=True)
source=Path('tests/benchmark/programs/gzip_crc32.c').read_text();rows=[]
compilers=gb.list_compilers(refresh=True)
for cid,flags in [('cg162','-O2 -march=x86-64-v3'),('cclang2310','-O2 -march=x86-64-v3'),('cicxlatest','-O2 -march=x86-64-v3'),('cicc2021100','-O2 -march=core-avx2')]:
 flags+=' -DDATA_SIZE=4096 -DPASSES=2'
 result=gb.compile_on_godbolt(cid,source,flags,execute=True)
 if result is None:raise RuntimeError(cid+' failed')
 version=gb._request_json(gb.API+'/compiler/'+cid+'/compile',body={'source':'int main(void){return 0;}','lang':'c','options':{'userArguments':'--version'}},timeout=90)
 (out/(cid+'.json.gz')).write_bytes(gzip.compress(json.dumps({'flags':flags,'result':result,'version_probe':version},indent=2).encode(),mtime=0))
 row={'id':cid,'metadata':next((x for x in compilers if x['id']==cid),None),'flags':flags,'execution':result.get('execResult'),'version_stdout':version.get('stdout')}
 rows.append(row);print(json.dumps(row),flush=True)
 assert row['execution'] and row['execution']['code']==0
(out/'summary.json').write_text(json.dumps(rows,indent=2)+'\n')
