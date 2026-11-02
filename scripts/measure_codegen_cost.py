#!/usr/bin/env python3
"""Compiler-work/size A/B: Callgrind counts of compilation, not native speed.

Both arms compile to assembly with identical flags. Text/rodata sizes come
from independently assembled objects. All raw profiles and identities remain
in the output directory; the manifest is explicitly incomplete on failure.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'tests/benchmark'))
import run_benchmarks as bench


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--mine', required=True)
    p.add_argument('--ref', required=True)
    p.add_argument('--out', type=Path, required=True)
    p.add_argument('--only', default='spectral_norm,byte_load_reassembly,expat_xml_scan,sha256_transform')
    p.add_argument('--mine-env', action='append', default=[])
    p.add_argument('--ref-env', action='append', default=[])
    args = p.parse_args()
    import os
    wanted = set(args.only.split(','))
    registry = {b.name: b for b in bench.BENCHMARKS}
    if wanted - registry.keys():
        p.error('unknown benchmark')
    sizes = json.loads((ROOT / 'tests/benchmark/callgrind_sizes.json').read_text())
    args.out.mkdir(parents=True, exist_ok=True)
    report = dict(complete=False, metric='compiler simulated Ir and assembled object section bytes; NOT wall time', results={})
    try:
        for name in sorted(wanted):
            source = bench.source_for(registry[name])
            row = dict(source=str(source), source_sha256=hashlib.sha256(source.read_bytes()).hexdigest())
            for arm, compiler, extra in [('ref', args.ref, args.ref_env), ('mine', args.mine, args.mine_env)]:
                env = dict(os.environ, **dict(pair.split('=', 1) for pair in extra))
                prefix = args.out / (name + '-' + arm)
                asm = prefix.with_suffix('.s')
                profile = prefix.with_suffix('.cg')
                import shlex
                flags = ['-O2', '-march=x86-64-v3', '-fwrapv', *shlex.split(sizes[name])]
                command = ['valgrind', '--tool=callgrind', '--callgrind-out-file=' + str(profile), '--error-exitcode=97', compiler, *flags, '-S', str(source), '-o', str(asm)]
                run = subprocess.run(command, env=env, text=True, capture_output=True, timeout=180)
                prefix.with_suffix('.log').write_text(run.stdout + run.stderr)
                if run.returncode:
                    raise RuntimeError(f'{name}/{arm}: compile/profile failed ({run.returncode})')
                counts = profile.read_text()
                summary = next(line for line in counts.splitlines() if line.startswith('summary:'))
                obj = prefix.with_suffix('.o')
                subprocess.run(['as', '--64', str(asm), '-o', str(obj)], check=True, capture_output=True)
                sections = subprocess.check_output(['size', '-A', str(obj)], text=True)
                values = {}
                for line in sections.splitlines():
                    parts = line.split()
                    if len(parts) == 3 and parts[0].startswith('.'):
                        values[parts[0]] = int(parts[1])
                row[arm] = dict(command=command, env=extra, compiler_sha256=hashlib.sha256(Path(compiler).read_bytes()).hexdigest(), ir=int(summary.split()[1]), sections=values, assembly_sha256=hashlib.sha256(asm.read_bytes()).hexdigest())
            row['compiler_ir_ratio'] = row['mine']['ir']/row['ref']['ir']
            report['results'][name] = row
            print(name, f"compiler Ir {row['compiler_ir_ratio']:.4f}", 'text bytes', row['ref']['sections'].get('.text'), '->', row['mine']['sections'].get('.text'), flush=True)
        report['complete'] = True
    finally:
        (args.out / 'index.json').write_text(json.dumps(report, indent=2) + '\n')


if __name__ == '__main__':
    main()
