#!/usr/bin/env python3
"""Generated helper arities and canonical-alias permissions, without prototypes.

Deliberately omit headers: their C declarations must not mask the intrinsic's
own operand contract. No unsupported instruction is executed by this test.
"""
import argparse
import os
from pathlib import Path
import re
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--lccc', default=os.environ.get('CCC', str(ROOT / 'target/fastbuild/lccc')))
    args = parser.parse_args()
    prototypes = re.findall(r'^[^#\n;{}]*?\b(__lccc_simd\w+)\(([^;\n{}]*)\);$',
                            (ROOT / 'include/lcccsimd.h').read_text(), re.M)
    assert len(prototypes) > 250, 'vacuous helper contract inventory'
    legs = 0
    with tempfile.TemporaryDirectory(prefix='simd-helper-contract-') as directory:
        source = Path(directory) / 'probe.c'

        def compile(text, mode, opt, flags):
            source.write_text(text)
            return subprocess.run([args.lccc, mode, opt, *flags, '-S', source,
                                   '-o', os.devnull], capture_output=True, text=True, timeout=30)

        for mode in ('-m32', '-m64'):
            for opt in ('-O0', '-O2'):
                for name, params in prototypes:
                    count = len(params.split(','))
                    for supplied in (0, count + 1):
                        result = compile(f'void f(void) {{ {name}({",".join(["0"] * supplied)}); }}',
                                         mode, opt, ['-march=x86-64-v4', '-mfma'])
                        diagnostic = result.stdout + result.stderr
                        if (result.returncode == 0 or name not in diagnostic
                                or 'argument' not in diagnostic or 'internal error' in diagnostic
                                or 'panicked' in diagnostic):
                            raise RuntimeError(f'{mode}/{opt}/{name}/{supplied}: expected arity diagnostic\n{diagnostic}')
                        legs += 1
                # These aliases resolve by appending 128. Permission must be
                # checked on that resolved key rather than defaulting to SSE2.
                for mnemonic, operands in [('divps', 2), ('minps', 2), ('maxps', 2),
                                            ('sqrtps', 1), ('rcpps', 1), ('rsqrtps', 1)]:
                    name = f'__lccc_simd128_ps_{mnemonic}'
                    text = ('typedef float V __attribute__((vector_size(16)));\n'
                            f'void f(V a, V b) {{ {name}(0,{",".join(["a"] * operands)}); }}')
                    for flags, accepted in [(['-mno-sse2', '-msse'], True),
                                            (['-mno-sse'], False)]:
                        result = compile(text, mode, opt, flags)
                        if (result.returncode == 0) != accepted:
                            raise RuntimeError(f'{mode}/{opt}/{name}/{flags}: {result.stderr}')
                        if not accepted and '-msse' not in result.stderr:
                            raise RuntimeError(f'missing SSE diagnostic: {result.stderr}')
                        legs += 1
                    # Unsuffixed spellings are not in the derived header table:
                    # this independently exercises the lowering arity guard.
                    for supplied in (0, operands + 2):
                        result = compile(f'void f(void) {{ {name}({",".join(["0"] * supplied)}); }}',
                                         mode, opt, ['-msse2'])
                        if result.returncode == 0 or 'arguments' not in result.stderr:
                            raise RuntimeError(f'unchecked alias arity: {name}/{supplied}: {result.stderr}')
                        legs += 1
                for name in ('__lccc_simd512_i_vpcmpeqb512',
                             '__lccc_simd512_i_vpcmpeqw512',
                             '__lccc_simd128_ps_not_an_instruction'):
                    result = compile(f'void f(void) {{ {name}(0,0,0); }}', mode, opt,
                                     ['-march=x86-64-v4'])
                    if result.returncode == 0 or 'unknown SIMD helper instruction' not in result.stderr:
                        raise RuntimeError(f'unknown helper fell through: {name}: {result.stderr}')
                    legs += 1
                print(f'PASS {mode} {opt}: {len(prototypes)} helper contracts and SSE aliases', flush=True)
    print(f'PASS: {legs} helper contract compile legs; no runtime/manual completeness claim')


if __name__ == '__main__':
    main()
