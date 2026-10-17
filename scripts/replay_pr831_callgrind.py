#!/usr/bin/env python3
"""Prove bounded measured ELFs still match after a linker-only compiler repair."""
import argparse
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import callgrind_ab as cg
ROOT=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT/'tests/benchmark'))
import run_benchmarks as bench
ap=argparse.ArgumentParser(description=__doc__)
ap.add_argument('--research',type=Path,required=True)
ap.add_argument('--compiler',type=Path,required=True)
a=ap.parse_args()
if not __debug__:ap.error('assertions are required')
rows=[];items={b.name:b for b in bench.BENCHMARKS}
with tempfile.TemporaryDirectory() as td:
 for tag in ['main','gcc-16','clang-23']:
  root=a.research/('callgrind-'+tag)
  index=json.loads((root/'index.json').read_text())
  assert index['complete'] and not index['failures']
  for name in index['selected']:
   manifest=json.loads((root/name/'manifest.json').read_text())
   source=bench.source_for(items[name]);record=manifest['results'][source.stem]
   assert hashlib.sha256(source.read_bytes()).hexdigest()==record['source_sha256']
   exe=Path(td)/name
   error=cg.compile_program(str(a.compiler.resolve()),manifest['opt'],source,exe,include=manifest['include'])
   assert error is None,(name,error)
   actual=hashlib.sha256(exe.read_bytes()).hexdigest()
   assert actual==record['binary_sha256']['mine'],(tag,name)
   rows.append(dict(arm=tag,name=name,sha256=actual))
report=dict(complete=True,compiler_sha256=hashlib.sha256(a.compiler.read_bytes()).hexdigest(),checks=rows)
(a.research/'callgrind-replay.json').write_text(json.dumps(report,indent=2)+'\n')
print('PASS: bounded candidate ELFs unchanged:',len(rows))
