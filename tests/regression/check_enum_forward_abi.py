#!/usr/bin/env python3
"""Non-kernel incomplete-enum callback ABI checks, with runtime oracles.

A semantic identity fix must not silently change enum widths or callback ABI.
Both callback fields are declared before the enum is completed. Volatile loads
and noinline implementations prevent a constant/inlined call from hiding a split.
"""
import argparse
import os
from pathlib import Path
import shutil
import shlex
import re
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
SOURCE = '''enum E;
struct Ops { long long (*get)(enum E); enum E (*put)(long long); };
enum E { LOW=LOW_VALUE, HIGH=HIGH_VALUE } PACKING;
__attribute__((noinline)) static long long get(enum E x) {return (long long)x;}
__attribute__((noinline)) static enum E put(long long x) {return (enum E)x;}
static volatile struct Ops ops={get,put};
int main(void) {
    long long values[4]={LOW_VALUE, 0, 1, HIGH_VALUE};
    if (sizeof(enum E)!=EXPECTED_SIZE) return 1;
    for (int i=0;i<4;i++) {
        enum E x=ops.put(values[i]);
        if (ops.get(x)!=values[i]) return 2+i;
    }
    return 0;
}
'''
SHAPES = {
    'wide': ('-1099511627776LL', '1099511627776LL', '', '8'),
    'packed': ('0', '255', '__attribute__((packed))', '1'),
    'ordinary': ('-1', '2147483647', '', '4'),
}
START = {
    'x86': '.text\n.globl _start\n_start:\n call main\n mov %eax,%edi\n mov $60,%eax\n syscall\n.section .note.GNU-stack,"",@progbits\n',
    'i686': '.text\n.globl _start\n_start:\n call main\n mov %eax,%ebx\n mov $1,%eax\n int $0x80\n.section .note.GNU-stack,"",@progbits\n',
    'arm': '.text\n.globl _start\n_start:\n bl main\n mov x8,#93\n svc #0\n',
    'riscv': '.text\n.globl _start\n_start:\n call main\n li a7,93\n ecall\n',
}
QEMU = {'i686': 'qemu-i386', 'arm': 'qemu-aarch64', 'riscv': 'qemu-riscv64'}


def execute(command):
    p = subprocess.run(list(map(str, command)), capture_output=True, text=True,
        timeout=60, env=dict(os.environ, LC_ALL='C', LCCC_STRICT_OPTIONS='1'))
    if p.returncode or p.stderr:
        raise AssertionError(f'{command}: rc={p.returncode}\n{p.stdout}\n{p.stderr}')


def run(lccc, *, gcc=None, clang=None, require_cross=False, require_icx=False):
    lccc = lccc if isinstance(lccc, list) else [str(lccc)]
    oracles = [gcc or ['gcc-16'], clang or shlex.split(os.environ.get('CLANG', 'clang-23'))]
    oracles = [c if isinstance(c, list) else shlex.split(c) for c in oracles]
    gcc_version = subprocess.check_output([*oracles[0], '-dumpfullversion'], text=True, timeout=30).strip()
    clang_version = subprocess.check_output([*oracles[1], '--version'], text=True, timeout=30)
    if not (gcc_version == '16.2' or gcc_version.startswith('16.2.')):
        raise AssertionError(f'expected GCC 16.2, got {gcc_version}')
    if not re.search(r'\bclang version 23\.1\.4(?:\s|$)', clang_version):
        raise AssertionError(f'expected Clang 23.1.4, got {clang_version}')
    icx = os.environ.get('LCCC_ICX_BIN') or os.environ.get('ICX')
    if icx:
        icx = shlex.split(icx)
        macros = subprocess.check_output([*icx, '-dM', '-E', '-x', 'c', '/dev/null'], text=True, timeout=30)
        version = re.search(r'^#define __INTEL_LLVM_COMPILER ([0-9]+)$', macros, re.M)
        if not version or int(version[1]) // 100 != 202602:
            raise AssertionError('expected ICX 2026.2.x version macro')
        oracles.append(icx)
    elif require_icx:
        raise AssertionError('ICX 2026.2.x is required for this run')
    else:
        print('UNMEASURED ICX 2026.2: compiler unavailable; GCC/Clang checks remain mandatory')
    measured = 0
    unmeasured = 0
    with tempfile.TemporaryDirectory(prefix='lccc-enum-forward-abi-') as tmp:
        root = Path(tmp)
        aliases = {}
        for target in START:
            alias = root/f'lccc-{target}'
            alias.symlink_to(Path(lccc[0]).resolve())
            aliases[target] = [str(alias), *lccc[1:]]
        for shape, (low, high, packing, size) in SHAPES.items():
            source = root/'fixture.c'
            source.write_text(SOURCE.replace('LOW_VALUE', low).replace('HIGH_VALUE', high)
                .replace('PACKING', packing).replace('EXPECTED_SIZE', size))
            for opt in ['-O0', '-O2', '-Os']:
                for oracle in oracles:
                    image = root/'oracle'
                    execute([*oracle, '-std=gnu17', opt, '-Wall', '-Wextra', '-Werror', source, '-o', image])
                    execute([image])
                    measured += 1
                for target, command in aliases.items():
                    assembly = root/'start.S'
                    assembly.write_text(START[target])
                    image = root/'fixture'
                    execute([*command, '-std=gnu17', opt, '-Wall', '-Wextra', '-Werror',
                        '-ffreestanding', '-nostdlib', '-static', '-Wl,--fatal-warnings',
                        assembly, source, '-o', image])
                    runner = [] if target == 'x86' else [shutil.which(QEMU[target])]
                    if runner and not runner[0]:
                        if require_cross:
                            raise AssertionError(f'missing runtime: {QEMU[target]}')
                        print(f'UNMEASURED {target} {shape} {opt}: runtime unavailable; build passed')
                        unmeasured += 1
                    else:
                        execute([*runner, image])
                        measured += 1
    print(f'enum forward ABI: PASS ({measured} measured executions, {unmeasured} unmeasured cross executions)')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--lccc', default=str(ROOT/'target/fastbuild/lccc'))
    parser.add_argument('--require-cross', action='store_true')
    parser.add_argument('--require-icx', action='store_true')
    args = parser.parse_args()
    run(args.lccc, require_cross=args.require_cross, require_icx=args.require_icx)
