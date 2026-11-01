#!/usr/bin/env python3
"""Differential joint-SLP adversarial fixtures; no timing-based pass criteria."""
from __future__ import annotations
import argparse
import json
import re
from pathlib import Path
import tempfile
from aos_pair_shapes import ProbeError, compiler, run
from check_lightpixel import body

ROOT = Path(__file__).resolve().parent
JOINT = [f'joint_alpha{i}' for i in range(4)] + [
    'joint_four', 'joint_retired_read', 'joint_intervening_write', 'joint_volatile'] + [
    f'joint_four_alpha{i}' for i in range(3)]
VALUES = ['joint_value_reverse', 'joint_value_four']
CHROMIUM = ['chromium_convolve_tail', 'chromium_convolve_canonical', 'chromium_rgb24_expand']


def harness() -> str:
    decls, checks = [], []
    for name in JOINT:
        for prefix in ('', 'ref_'):
            decls.append(f'void {prefix}{name}(const unsigned char*,unsigned char*,unsigned);')
        checks.append(f'''
        memcpy(x,initial,64); memcpy(y,initial,64);
        {name}(x+16,x+16+delta,scale); ref_{name}(y+16,y+16+delta,scale);
        if (memcmp(x,y,64)) return fail("{name}",i,delta);
''')
    for name in VALUES:
        for prefix in ('', 'ref_'):
            decls.append(f'unsigned {prefix}{name}(unsigned,unsigned);')
        checks.append(f'if ({name}(packed,scale)!=ref_{name}(packed,scale)) return fail("{name}",i,delta);')
    for name in CHROMIUM[:2]:
        for prefix in ('', 'ref_'):
            decls.append(f'void {prefix}{name}(const int*,unsigned char*);')
        checks.append(f'''
        memcpy(x,initial,64); memcpy(y,initial,64);
        {name}(acc,x+16+delta); ref_{name}(acc,y+16+delta);
        if (memcmp(x,y,64)) return fail("{name}",i,delta);
''')
    guards = []
    for name in JOINT[:5] + JOINT[8:]:
        count = 4 if 'four' in name else 3
        guards.append(f'''
        for(int k=0;k<{count};++k) pin[-{count}+k]=(unsigned char)random32();
        memset(pout-4,0,4);
        {name}(pin-{count},pout-4,scale);
        ref_{name}(pin-{count},expected,scale);
        if(memcmp(pout-4,expected,4)) return fail("guard {name}",i,0);
''')
    decls.extend(['void chromium_rgb24_expand(const unsigned char*,unsigned char*,int);',
                  'void ref_chromium_rgb24_expand(const unsigned char*,unsigned char*,int);'])
    return '''#include <stdio.h>
#include <string.h>
#include <stdint.h>
#include <sys/mman.h>
#include <unistd.h>
''' + '\n'.join(decls) + '''
static uint32_t state=0x83f14a27u;
static uint32_t random32(void) { state^=state<<13;state^=state>>17;state^=state<<5;return state; }
static int fail(const char*n,int i,int d) { fprintf(stderr,"FAIL %s case %d offset %d\\n",n,i,d);return 1; }
int main(void) {
    unsigned char initial[64], x[64],y[64];
    static const uint32_t edges[]={0,1,255,256,32767,32768,0x7fffffff,0x80000000,0xffffffff};
    for (int i=0;i<65536;++i) {
        for(int j=0;j<64;++j) initial[j]=(unsigned char)random32();
        unsigned scale=i<9 ? edges[i] : random32();
        unsigned packed=random32();
        int acc[4];
        for(int j=0;j<4;++j) acc[j]=(int)(i<9 ? edges[(i+j)%9] : random32());
        for(int delta=-3;delta<=4;++delta) {
''' + '\n'.join(checks) + '''
        }
    }
    long page=sysconf(_SC_PAGESIZE);
    if(page<=0) return 1;
    unsigned char *a=mmap(0,2*page,PROT_READ|PROT_WRITE,MAP_PRIVATE|MAP_ANONYMOUS,-1,0);
    unsigned char *b=mmap(0,2*page,PROT_READ|PROT_WRITE,MAP_PRIVATE|MAP_ANONYMOUS,-1,0);
    if(a==MAP_FAILED || b==MAP_FAILED) return 1;
    if(mprotect(a+page,page,PROT_NONE) || mprotect(b+page,page,PROT_NONE)) return 1;
    unsigned char *pin=a+page, *pout=b+page, expected[72];
    for(int i=0;i<4096;++i) {
        unsigned scale=i<9 ? edges[i] : random32();
''' + '\n'.join(guards) + '''
        int n=i%18;
        for(int k=0;k<3*n;++k) pin[-3*n+k]=(unsigned char)random32();
        chromium_rgb24_expand(pin-3*n,pout-4*n,n);
        ref_chromium_rgb24_expand(pin-3*n,expected,n);
        if(memcmp(pout-4*n,expected,4*n)) return fail("guard rgb24",i,n);
    }
    if(munmap(a,2*page) || munmap(b,2*page)) return 1;
    puts("PASS joint pixels 65536 cases 15 kernels overlap -3..4 signed/full-u32 guarded3/4/rgb24");
    return 0;
}
'''


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--lccc', default='target/fastbuild/lccc')
    p.add_argument('--reference', default='gcc-16')
    p.add_argument('--json', type=Path)
    args = p.parse_args()
    cc, ref = compiler(args.lccc), compiler(args.reference)
    rows = []
    with tempfile.TemporaryDirectory(prefix='joint-pixels-') as temp:
        work = Path(temp)
        host = work / 'harness.c'
        host.write_text(harness())
        refs = []
        for source, names in [('joint_pixel_shapes.c', JOINT + VALUES), ('chromium_pixel_shapes.c', CHROMIUM)]:
            obj = work / ('ref_' + source + '.o')
            run([*ref, '-O0', '-Wall', '-Wextra', '-Werror',
                 *[f'-D{n}=ref_{n}' for n in names], '-c', str(ROOT / source), '-o', str(obj)], work, 60)
            refs.append(str(obj))
        for arch, flags in [('sse2', ['-msse2', '-mno-sse4.1', '-mno-avx']),
                            ('sse41', ['-msse4.1', '-mno-avx']), ('avx2', ['-mavx2'])]:
            for opt in ['-O2', '-O3']:
                row = dict(arch=arch, opt=opt, ok=False)
                try:
                    objects, assembly = [], {}
                    for source, names in [('joint_pixel_shapes.c', JOINT + VALUES), ('chromium_pixel_shapes.c', CHROMIUM)]:
                        obj, asm = work / (source + '.o'), work / (source + '.s')
                        base = [*cc, opt, *flags, '-Wall', '-Wextra', '-Werror']
                        run([*base, '-c', str(ROOT / source), '-o', str(obj)], work, 60)
                        run([*base, '-S', str(ROOT / source), '-o', str(asm)], work, 60)
                        objects.append(str(obj))
                        assembly.update({n: body(asm.read_text(), n) for n in names})
                    if arch != 'sse2':
                        for name in JOINT[:5] + JOINT[8:] + VALUES + ['chromium_convolve_canonical']:
                            if not re.search(r'\bv?pmaxub\b', assembly[name]):
                                raise ProbeError(f'{name}: missing joint byte-domain reduction')
                    # Volatile inputs remain three distinct, ordered byte reads;
                    # vectorizing only their pure consumers is legal.
                    reads = re.findall(r'\bmovzbl\s+[^,\n]*\([^)\n]+\),', assembly['joint_volatile'])
                    if len(reads) != 3:
                        raise ProbeError('volatile: expected three distinct byte reads')
                    exe = work / 'probe'
                    run([*ref, '-O0', '-Wall', '-Wextra', '-Werror', str(host), *refs,
                         *objects, '-o', str(exe)], work, 60)
                    receipt = run([str(exe)], work, 60)
                    expected = 'PASS joint pixels 65536 cases 15 kernels overlap -3..4 signed/full-u32 guarded3/4/rgb24\n'
                    if receipt != expected:
                        raise ProbeError(f'incomplete receipt {receipt!r}')
                    row.update(ok=True, receipt=receipt.strip(), assembly=assembly)
                except ProbeError as exc:
                    row['error'] = str(exc)
                rows.append(row)
                print('PASS' if row['ok'] else 'FAIL', arch, opt, row.get('error', ''))
    if args.json:
        args.json.write_text(json.dumps(dict(ok=all(r['ok'] for r in rows), cases=rows), indent=2)+'\n')
    return 0 if all(r['ok'] for r in rows) else 1


if __name__ == '__main__':
    raise SystemExit(main())
