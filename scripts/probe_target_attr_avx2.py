#!/usr/bin/env python3
"""Target-attribute probe: AVX2 names called from a __attribute__((target("avx2")))
function in a TU compiled with -mavx.

GCC 16.2 enables the ISA per function through the target attribute, so every AVX2
name is accepted there under plain -mavx.  lccc hides some AVX2 names from the
header under -mavx (`#ifdef __AVX2__`), so they are undeclared inside such a
function even though GCC accepts them.  Verdicts are per compiler, from rc of
`-c` (codegen diagnostics such as "target specific option mismatch" only appear
at codegen, not -fsyntax-only).

Usage: probe_target_attr_avx2.py --gate gate_vNN.json --out result.json
"""
import argparse
import concurrent.futures
import json
import os
import subprocess
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import probe_sse_avx_gating as P  # noqa: E402

FLAGS = ['-O2', '-mavx']

WRAP_VALUE = '''#include <immintrin.h>
extern void probe_use(const void *);
__attribute__((target("avx2"))) void probe_fn(void)
{{
    __typeof__({call}) probe_r = {call};
    probe_use(&probe_r);
}}
'''

WRAP_VOID = '''#include <immintrin.h>
extern void probe_use(const void *);
__attribute__((target("avx2"))) void probe_fn(void)
{{
    {call};
}}
'''


def call_text(name, args):
    return f'{name}({", ".join(args)})'


def compile_once(compiler, name, args, void_form, workdir):
    call = call_text(name, args)
    src = os.path.join(workdir, f'{name}_{int(void_form)}.c')
    with open(src, 'w') as f:
        f.write((WRAP_VOID if void_form else WRAP_VALUE).format(call=call))
    obj = src + '.o'
    proc = subprocess.run([compiler, *FLAGS, '-c', src, '-o', obj],
                          capture_output=True, text=True, timeout=300)
    return proc.returncode, (proc.stderr or '')[:400]


def probe(task):
    name, args, void_form, gcc, lccc, workdir = task
    result = {'args': args, 'void_form': void_form}
    for label, compiler in (('gcc', gcc), ('lccc', lccc)):
        # The form comes from the gating probe, which only uses the void form for
        # names whose GCC declaration returns void.  No fallback: a void form for a
        # non-void call is dead-code-eliminated and accepted by GCC wrongly.
        rc, msg = compile_once(compiler, name, args, void_form, workdir)
        result[label] = rc == 0
        if rc != 0:
            result[label + '_msg'] = msg
    return name, result


def main():
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[1])
    ap.add_argument('--gate', required=True, help='gate_vNN.json from the gating probe')
    ap.add_argument('--gcc', default='gcc-16')
    ap.add_argument('--lccc', default='/home/user/lccc/target/fastbuild/lccc')
    ap.add_argument('--jobs', type=int, default=2)
    ap.add_argument('--out', required=True)
    opts = ap.parse_args()

    gate = json.load(open(opts.gate))
    rows = gate['results']
    # Names in scope: the gating probe's union-flag set, minus names it could not
    # resolve (those need a human argument decision, not a guess).
    names = sorted(n for n, r in rows.items() if r.get('gcc_union') and not r.get('unresolved'))

    with tempfile.TemporaryDirectory(prefix='target_probe_') as workdir:
        tasks = []
        for n in names:
            r = rows[n]
            args = r.get('args') or []
            tasks.append((n, args, bool(r.get('void_form')), opts.gcc, opts.lccc, workdir))
        with concurrent.futures.ThreadPoolExecutor(max_workers=opts.jobs) as pool:
            out = dict(pool.map(probe, tasks))

    gap = sorted(n for n, r in out.items() if r['gcc'] and not r['lccc'])
    failopen = sorted(n for n, r in out.items() if r['lccc'] and not r['gcc'])
    both = sorted(n for n, r in out.items() if r['gcc'] and r['lccc'])
    summary = {'probed': len(out), 'gcc_accepts': sum(r['gcc'] for r in out.values()),
               'lccc_accepts': sum(r['lccc'] for r in out.values()),
               'gap': len(gap), 'failopen': len(failopen), 'both_accept': len(both)}
    doc = {'gcc': opts.gcc, 'lccc': opts.lccc, 'flags': FLAGS, 'summary': summary,
           'lists': {'gap': gap, 'failopen': failopen}, 'results': out}
    with open(opts.out, 'w') as f:
        json.dump(doc, f, indent=1, sort_keys=True)
    print(json.dumps(summary, indent=1))


if __name__ == '__main__':
    main()
