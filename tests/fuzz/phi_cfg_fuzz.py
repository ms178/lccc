#!/usr/bin/env python3
"""CFG/phi-web differential fuzzer for LCCC.

Generates defined-behavior C programs whose branch diamonds, switch joins,
loop-carried state, continue/break paths, and postfix decrement expressions
exercise phi lowering and copy coalescing.  Compare complete stdout and exit
status against a reference compiler.
"""
from __future__ import annotations
import argparse, concurrent.futures, json, random, subprocess
from pathlib import Path

MASK=(1<<64)-1

def u64(x): return f"UINT64_C(0x{x & MASK:016x})"

def gen(seed: int) -> str:
    r=random.Random(seed)
    n=r.randint(3,24)
    rounds=r.randint(3,10)
    lines=[
        '#include <stdint.h>', '#include <stdio.h>', '#include <inttypes.h>',
        'static volatile uint64_t observe;',
        'static uint64_t rot(uint64_t x, unsigned n) { n &= 63u; return n ? ((x << n) | (x >> ((64u-n)&63u))) : x; }',
        'static uint64_t mix(uint64_t a, uint64_t b, unsigned n) { a ^= rot(b + UINT64_C(0x9e3779b97f4a7c15), n); a *= UINT64_C(0xbf58476d1ce4e5b9); return a ^ (a >> 31); }',
        'static uint64_t kernel(uint64_t seed, unsigned limit) {',
        f'  uint64_t a={u64(r.getrandbits(64))} ^ seed;',
        f'  uint64_t b={u64(r.getrandbits(64))} + seed;',
        f'  uint64_t c={u64(r.getrandbits(64))};',
        '  uint64_t table[8];',
        '  for (unsigned z=0; z<8; ++z) table[z] = mix(a+z,b^z,z);',
        '  for (unsigned i=0; i<limit; ++i) {',
        '    switch ((unsigned)((a ^ b ^ c ^ i) & 7u)) {',
    ]
    for k in range(8):
        x=r.getrandbits(64); y=r.getrandbits(64)
        if k % 4 == 0:
            body=f'a = mix(a ^ {u64(x)}, b + c, i); b ^= rot(c + {u64(y)}, i);'
        elif k % 4 == 1:
            body=f'c = mix(c + {u64(x)}, a ^ b, i+1u); a += rot(b ^ {u64(y)}, i);'
        elif k % 4 == 2:
            body=f'b = mix(b + {u64(x)}, c, i+2u); c ^= rot(a + {u64(y)}, i);'
        else:
            body=f'a ^= mix(b, {u64(x)}, i); c += mix(a, {u64(y)}, i+3u);'
        lines += [f'      case {k}: {{ {body} break; }}']
    lines += [
        '    }',
        '    if ((a + i) & 1u) {',
        '      uint64_t old = b++;',
        '      a ^= mix(old, c, i);',
        '    } else {',
        '      uint64_t old = c--;',
        '      b ^= mix(old, a, i+1u);',
        '    }',
        '    for (unsigned j=0; j<3; ++j) {',
        '      unsigned idx=(unsigned)((a+b+c+j)&7u);',
        '      if ((table[idx] ^ a) & 1u) { a += table[idx]; continue; }',
        '      b ^= table[idx]; c += (a ^ b) + j;',
        '    }',
        '    observe = a ^ b ^ c;',
        '    if ((observe & 15u) == 3u) { c ^= observe; }',
        '  }',
        '  return a ^ rot(b,17) ^ rot(c,39) ^ observe;',
        '}',
        'int main(void) {',
        f'  uint64_t x = kernel({u64(r.getrandbits(64))}, {n}u);',
        f'  uint64_t y = kernel(x ^ {u64(r.getrandbits(64))}, {rounds}u);',
        '  printf("%016" PRIx64 " %016" PRIx64 "\\n", x, y);',
        '  return 0;',
        '}',
    ]
    return '\n'.join(lines)+'\n'

def call(cmd, timeout):
    try:
        p=subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, timeout=timeout)
        return p.returncode,p.stdout,p.stderr
    except subprocess.TimeoutExpired: return 'TIMEOUT','','timeout'

def one(args):
    seed,level,ccc,gcc,out=args
    d=out/f'{seed:05d}-{level}'; d.mkdir(parents=True,exist_ok=True)
    src=d/'x.c'; src.write_text(gen(seed)); cb=d/'c'; gb=d/'g'
    flags=['-std=gnu11','-w','-march=raptorlake','-mtune=raptorlake','-fomit-frame-pointer']
    gl='-Os' if level=='Oz' else '-'+level
    gr,_,ge=call([gcc,gl,*flags,str(src),'-o',str(gb)],45)
    cr,_,ce=call([ccc,'-'+level,*flags,str(src),'-o',str(cb)],60)
    if gr!=0 or cr!=0:
        return {'seed':seed,'level':level,'status':'compile','gcc_rc':gr,'ccc_rc':cr,'detail':(ge+'\n'+ce)[-1000:]}
    gr,go,ge=call([str(gb)],10); cr,co,ce=call([str(cb)],10)
    if gr==cr and go==co:
        for p in (src,cb,gb): p.unlink(missing_ok=True)
        try:d.rmdir()
        except OSError:pass
        return {'seed':seed,'level':level,'status':'pass'}
    return {'seed':seed,'level':level,'status':'mismatch','gcc_rc':gr,'ccc_rc':cr,'gcc_out':go,'ccc_out':co,'detail':(ge+'\n'+ce)[-1000:]}

def main():
    ap=argparse.ArgumentParser(); ap.add_argument('--ccc',required=True); ap.add_argument('--gcc',required=True); ap.add_argument('--seeds',default='0:200'); ap.add_argument('--levels',default='O0,O3,Os'); ap.add_argument('--jobs',type=int,default=2); ap.add_argument('--out',type=Path,required=True); ns=ap.parse_args(); ns.out.mkdir(parents=True,exist_ok=True)
    lo,hi=map(int,ns.seeds.split(':')); levels=ns.levels.split(','); jobs=max(1,ns.jobs)
    cases=[(s,l,ns.ccc,ns.gcc,ns.out) for l in levels for s in range(lo,hi)]
    failed=[]; counts={}
    with concurrent.futures.ThreadPoolExecutor(max_workers=jobs) as ex:
        for row in ex.map(one,cases):
            counts[row['status']]=counts.get(row['status'],0)+1
            if row['status']!='pass':
                failed.append(row); print('FAIL',row['seed'],row['level'],row['status'],flush=True)
    report={'cases':len(cases),'counts':counts,'failures':failed,'levels':levels,'seeds':ns.seeds,'ccc':ns.ccc,'gcc':ns.gcc}
    (ns.out/'summary.json').write_text(json.dumps(report,indent=2)+'\n'); print(json.dumps(report,indent=2)); return 1 if failed else 0
if __name__=='__main__': raise SystemExit(main())
