#!/usr/bin/env python3
"""Run the production implicit-effects classifiers without a full crate rebuild.

Expected minimum reads and exact write/full-write masks are independently
reviewed in tests/fixtures/implicit_operands.tsv. Existing conservative extra
reads are allowed; a partial write must never become a full-register kill.
Rust library tests consume the same fixture. No privileged opcodes execute.
"""
import json
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent.parent
text = (ROOT / 'src/backend/x86/codegen/peephole/types.rs').read_text()
# `implicit_full_write_refs` delegates to `_from_write_mask`; extracting one
# without the other made the probe fail to compile, which is how this gate sat
# broken and unwired.  Every body a checked function calls must be listed.
functions = []
for name in [
    'classify_implicit_operands',
    'implicit_full_write_refs_from_write_mask',
    'implicit_full_write_refs',
]:
    start = text.index('fn ' + name + '(')
    end = text.index('\n}\n', start) + 3
    functions.append(text[start:end])
cases = [(fields[0], *map(int, fields[1:])) for fields in (line.split('\t') for line in (ROOT / 'tests/fixtures/implicit_operands.tsv').read_text().splitlines())]
body = ['fn main() { let mut failures = 0;']
for asm, reads, writes, full in cases:
    body.append(f'let t = {json.dumps(asm)}; let (r,w) = classify_implicit_operands(t.as_bytes()); let f = implicit_full_write_refs(t.as_bytes()); if r & {reads} != {reads} || w != {writes} || f != {full} {{ eprintln!("FAIL {{t}}: got reads={{r}} writes={{w}} full={{f}}; expected minimum reads={reads} writes={writes} full={full}"); failures += 1; }}')
body.append(f'println!("{{}} failures / {len(cases)} cases", failures); if failures != 0 {{ std::process::exit(1); }} }}')
with tempfile.TemporaryDirectory(prefix='lccc-implicit-effects-') as tmp:
    source = Path(tmp) / 'probe.rs'
    binary = Path(tmp) / 'probe'
    source.write_text('\n'.join(functions + body))
    subprocess.run(
        ['rustc', '--edition=2024', '-Copt-level=1', '-Dwarnings', str(source), '-o', str(binary)],
        check=True,
    )
    raise SystemExit(subprocess.run([str(binary)]).returncode)
