#!/usr/bin/env python3
"""Rebuild every original native request; distinguish unchanged ELFs from new epochs."""
import argparse
import json,hashlib,subprocess
from pathlib import Path
ap=argparse.ArgumentParser(description=__doc__)
ap.add_argument('--research',type=Path,required=True)
ap.add_argument('--compiler',type=Path,required=True)
a=ap.parse_args()
if not __debug__: ap.error('assertions are required')
root=a.research;data=json.loads((root/'native.json').read_text());out=root/'replay';out.mkdir(exist_ok=True);rows=[]
for b in data['benchmarks']:
 assert hashlib.sha256(Path(b['source']).read_bytes()).hexdigest()==b['source_sha256']
 c=b['compile']['lccc'];cmd=c['command'][:];cmd[0]=str(a.compiler.resolve());cmd[cmd.index('-o')+1]=str(out/b['name'])
 p=subprocess.run(cmd,capture_output=True,text=True,timeout=180);assert p.returncode==0,(b['name'],p.stderr)
 h=hashlib.sha256((out/b['name']).read_bytes()).hexdigest();rows.append(dict(name=b['name'],command=cmd,old_sha256=c['binary_sha256'],new_sha256=h,identical=h==c['binary_sha256']))
r=dict(complete=True,compiler_sha256=hashlib.sha256(a.compiler.read_bytes()).hexdigest(),checks=rows,changed=[x['name'] for x in rows if not x['identical']]);(root/'native-replay.json').write_text(json.dumps(r,indent=2)+'\n');print('changed',r['changed'])
