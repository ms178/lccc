#!/usr/bin/env python3
"""Classify gap names by the lowest ISA that GCC accepts them at.

For each name GCC accepts under -mavx (or -mavx2), compile the same call in a plain
function under -msse4.2 (no AVX).  Accepted there => SSE-level (outside AVX scope);
rejected => the name needs AVX/AVX2.  Usage: classify_gap_isa.py gate.json tier out.json
"""
import concurrent.futures, json, os, subprocess, sys, tempfile
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

WRAP = '''#include <immintrin.h>
extern void probe_use(const void *);
void probe_fn(void)
{{
    {body}
}}
'''


def accepts(gcc, flags, name, args, void_form, workdir):
    call = f'{name}({", ".join(args)})'
    body = f'{call};' if void_form else f'__typeof__({call}) probe_r = {call}; probe_use(&probe_r);'
    src = os.path.join(workdir, f'{name}_{abs(hash(tuple(flags)))}.c')
    with open(src, 'w') as f:
        f.write(WRAP.format(body=body))
    p = subprocess.run([gcc, '-O2', *flags, '-c', src, '-o', src + '.o'],
                       capture_output=True, text=True, timeout=300)
    return p.returncode == 0


def main():
    gate, tier, out = sys.argv[1], sys.argv[2], sys.argv[3]
    d = json.load(open(gate))
    gcc = d['gcc']
    flags = {'avx': ['-mavx'], 'avx2': ['-mavx2']}[tier]
    names = d['lists'][tier]['gap']
    sse = ['-msse4.2']
    with tempfile.TemporaryDirectory(prefix='classify_') as workdir:
        def work(n):
            r = d['results'][n]
            return n, accepts(gcc, sse, n, r['args'], bool(r['void_form']), workdir)
        with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
            res = dict(pool.map(work, names))
    sse_level = sorted(n for n, ok in res.items() if ok)
    avx_level = sorted(n for n, ok in res.items() if not ok)
    doc = {'tier': tier, 'gap_total': len(names), 'sse_level': sse_level,
           'avx_level': avx_level}
    json.dump(doc, open(out, 'w'), indent=1)
    print(tier, 'gap', len(names), 'sse-level', len(sse_level), 'avx-level', len(avx_level))


if __name__ == '__main__':
    main()
