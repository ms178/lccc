#!/usr/bin/env python3
"""ISA-contract gate for AVX1 translation units.

A translation unit built with `-mavx` must not contain any AVX2 instruction:
the CPU that runs it may have AVX without AVX2 (Sandy/Ivy Bridge, Bulldozer
class), and an AVX2 opcode there is an illegal-instruction fault. This gate
compiles the AVX broadcast and zero-extension fixtures with `-mavx`, strict
diagnostics on, disassembles the object, and fails on any AVX2-only mnemonic:
integer 256-bit operations (`vp*` with a `%ymm` operand), the 128-bit lane
moves (`vinserti128`, `vextracti128`), the AVX2 broadcasts
(`vbroadcasti128`, `vpbroadcast*`, register-source `vbroadcastss/sd`),
permutes (`vperm2i128`, `vpermd`, `vpermq`), blends and masks (`vpblendd`,
`vpmaskmov*`), variable shifts (`vpsllv*`, `vpsrlv*`), and gathers.

Each fixture's `-mavx` and `-mavx2` outputs must match the GCC 16.2 oracle.
The memory-form fixture must select the single-instruction `vbroadcastss` and
`vbroadcastsd` from memory under `-mavx2`, so a regression to a stack store
and reload fails this gate.
"""
from __future__ import annotations

import argparse
import os
from pathlib import Path
import re
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
FIXTURES = ('avx_broadcast_intrinsics', 'avx_zext_upper_zero')
# Fixtures whose -mavx2 object must contain a memory-source broadcast.
MEMORY_FORM_FIXTURE = 'avx2_broadcast_memory_form'
MEMORY_FORM = re.compile(r'\s(vbroadcastss|vbroadcastsd)\s+[^,]*\(%')

# AVX2-only mnemonics that carry no %ymm operand in the disassembly.
AVX2_NAMED = re.compile(
    r'^(vinserti128|vextracti128|vbroadcasti128|vperm2i128|vpermd|vpermq|'
    r'vpblendd|vgatherd[pd]|vgatherq[pd]|vpgatherd[dq]|vpgatherq[dq]|'
    r'vpsllv[dq]|vpsrlv[dq]|vpsravd|vpmaskmov[dq])$')
# Integer 256-bit operations are AVX2 whenever a %ymm register is involved.
INT_VEC = re.compile(r'^vp[a-z0-9]+$')
# Register-source broadcasts are AVX2; the memory-source forms are AVX1.
BCAST_REG = re.compile(r'^vbroadcast(ss|sd)$')
DISASM = re.compile(r'^\s*[0-9a-f]+:\t(?:[0-9a-f]{2} )+\s*\t(\S+)\s*(.*)$')


def run(command: list[str], *, check: bool = True) -> subprocess.CompletedProcess:
    result = subprocess.run(command, cwd=ROOT, capture_output=True, text=True, timeout=120)
    if check and result.returncode:
        raise AssertionError(f'{command}: exit {result.returncode}\n{result.stdout}\n{result.stderr}')
    return result


def avx2_hits(objdump: str, obj: Path) -> list[str]:
    hits = []
    for line in run([objdump, '-d', str(obj)]).stdout.splitlines():
        m = DISASM.match(line)
        if not m:
            continue
        mnem, operands = m.group(1), m.group(2)
        if AVX2_NAMED.match(mnem):
            hits.append(line.strip())
        elif INT_VEC.match(mnem) and '%ymm' in operands:
            hits.append(line.strip())
        elif BCAST_REG.match(mnem) and '%xmm' in operands and '%ymm' in operands:
            hits.append(line.strip())
    return hits


def output(binary: Path) -> str:
    return run([str(binary)]).stdout


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--lccc', default='target/fastbuild/lccc')
    parser.add_argument('--gcc', default=os.environ.get('GCC', 'gcc-16'))
    parser.add_argument('--objdump', default=os.environ.get('OBJDUMP', 'objdump'))
    parser.add_argument('--lccc-i686', default='target/fastbuild/lccc-i686',
                        help='i686 driver; its -m32 -mavx objects get the same ISA gate')
    args = parser.parse_args()
    checked = 0
    with tempfile.TemporaryDirectory(prefix='lccc-avx1-isa-') as td:
        tmp = Path(td)
        for name in FIXTURES:
            source = ROOT / 'tests/regression' / (name + '.c')
            strict = ['-O2', '-Wall', '-Wextra', '-Werror']

            # 1. AVX1 object: no AVX2 instruction anywhere in the translation unit.
            avx_obj = tmp / (name + '.avx.o')
            run([args.lccc, *strict, '-mavx', '-c', str(source), '-o', str(avx_obj)])
            hits = avx2_hits(args.objdump, avx_obj)
            assert not hits, f'{name}: AVX2 instructions under -mavx:\n' + '\n'.join(hits)
            checked += 1

            # 2. The AVX1 binary computes what the GCC oracle computes.
            avx_bin = tmp / (name + '.avx')
            run([args.lccc, *strict, '-mavx', str(source), '-o', str(avx_bin)])
            gcc_bin = tmp / (name + '.gcc')
            run([args.gcc, '-O2', '-mavx', str(source), '-o', str(gcc_bin)])
            got, want = output(avx_bin), output(gcc_bin)
            assert got == want, f'{name}: -mavx output {got!r} != GCC {want!r}'
            checked += 1

            # 3. AVX2 build: the same fixture computes what the GCC oracle computes.
            avx2_bin = tmp / (name + '.avx2')
            run([args.lccc, *strict, '-mavx2', str(source), '-o', str(avx2_bin)])
            gcc2_bin = tmp / (name + '.gcc2')
            run([args.gcc, '-O2', '-mavx2', str(source), '-o', str(gcc2_bin)])
            got2, want2 = output(avx2_bin), output(gcc2_bin)
            assert got2 == want2, f'{name}: -mavx2 output {got2!r} != GCC {want2!r}'
            checked += 1

        # 4. AVX2 selection: a scalar broadcast from memory is the single
        #    memory-source instruction, with no stack store and reload.
        source = ROOT / 'tests/regression' / (MEMORY_FORM_FIXTURE + '.c')
        mem_obj = tmp / (MEMORY_FORM_FIXTURE + '.o')
        run([args.lccc, '-O2', '-Wall', '-Wextra', '-Werror', '-mavx2', '-c', str(source), '-o', str(mem_obj)])
        disasm = run([args.objdump, '-d', str(mem_obj)]).stdout
        forms = [line.strip() for line in disasm.splitlines() if MEMORY_FORM.search(line)]
        assert len(forms) == 2, f'{MEMORY_FORM_FIXTURE}: expected vbroadcastss and vbroadcastsd from memory, found {forms}'
        checked += 1
        # 5. i686 leg: the same AVX1 contract for 32-bit x86, checked against
        #    GCC -m32 -mavx. Runs only when the i686 driver is built.
        if Path(args.lccc_i686).exists():
            for name in FIXTURES:
                source = ROOT / 'tests/regression' / (name + '.c')
                strict = ['-O2', '-Wall', '-Wextra', '-Werror']
                obj = tmp / (name + '.i686.avx.o')
                run([args.lccc_i686, *strict, '-m32', '-mavx', '-c', str(source), '-o', str(obj)])
                hits = avx2_hits(args.objdump, obj)
                assert not hits, f'{name}: AVX2 instructions under -m32 -mavx:\n' + '\n'.join(hits)
                exe = tmp / (name + '.i686.avx')
                run([args.lccc_i686, *strict, '-m32', '-mavx', str(source), '-o', str(exe)])
                gcc32 = tmp / (name + '.gcc32')
                run([args.gcc, '-O2', '-m32', '-mavx', str(source), '-o', str(gcc32)])
                got, want = output(exe), output(gcc32)
                assert got == want, f'{name}: i686 -m32 -mavx output {got!r} != GCC {want!r}'
                checked += 2
        else:
            raise AssertionError(f'i686 driver not built at {args.lccc_i686}; the i686 leg cannot be skipped')
    print(f'check_avx1_isa_clean: {checked} checks passed over {len(FIXTURES)} fixtures')


if __name__ == '__main__':
    main()
