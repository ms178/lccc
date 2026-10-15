"""Replay frozen marker/UTF-8 red/green tests without modifying evidence."""
import argparse
import gzip
import hashlib
import json
from pathlib import Path
import shutil
import subprocess

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--out', type=Path, required=True)
a = p.parse_args()
frozen = Path(__file__).resolve().parent
out = a.out.resolve()
out.mkdir(parents=True, exist_ok=True)
manifest = json.loads((frozen/'manifest.json').read_text())
for name, expected in manifest.items():
    raw = gzip.decompress((frozen/(name+'.gz')).read_bytes())
    assert hashlib.sha256(raw).hexdigest() == expected['sha256'], name
    (out/name).write_bytes(raw)
rustc = shutil.which('rustc')
if not rustc:
    raise SystemExit('rustc is required')
head = '''#![allow(dead_code)]
mod common {pub mod fx_hash {pub type FxHashMap<K,V> = std::collections::HashMap<K,V>;pub type FxHashSet<K> = std::collections::HashSet<K>;}}
#[path="utils.rs"] mod utils;
mod text_processing {#[derive(Clone,Debug)]pub enum LineResolver {Flat(usize)} impl LineResolver {pub fn line_at(&self,_:usize)->usize {match self {Self::Flat(x)=>*x}}}}
'''
for arm, expected in [('before', 101), ('after', 0)]:
    main = out/(arm+'-main.rs')
    main.write_text(head+f'#[path="{arm}-with-tests.rs"] mod macros;\n')
    exe = out/(arm+'-tests')
    subprocess.run([rustc, '--edition=2024', '-C', 'opt-level=1', '--test', str(main), '-o', str(exe)], check=True)
    result = subprocess.run([str(exe)], capture_output=True, text=True)
    (out/(arm+'-replay.log')).write_text(result.stdout+result.stderr)
    print(arm, result.returncode)
    assert result.returncode == expected, result.stdout+result.stderr
    assert ('2 passed; 2 failed' if arm == 'before' else '4 passed; 0 failed') in result.stdout
