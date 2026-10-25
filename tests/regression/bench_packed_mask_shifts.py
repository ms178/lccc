#!/usr/bin/env python3
"""Canonical paired runner, with separate GCC driver and packed-shift kernel TUs.

Use the runner's normal options, e.g. --candidate before=/path/to/frozen/lccc
--compilers lccc,gcc,clang --gcc gcc-16 --clang clang-23 --cflag=-msse2
--reps 15 --json /path/results.json --artifact-dir /path/artifacts --strict.
The driver is identical across compiler arms; no LTO or kernel inlining.
"""
import hashlib
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'tests/benchmark'))
import run_benchmarks as lab

DRIVER = ROOT / 'tests/benchmark/programs/packed_mask_shifts_driver.c'
KERNELS = ROOT / 'tests/regression/packed_mask_shifts.c'
NAMES = ('packed_shl16', 'packed_ashr32', 'packed_mask16', 'packed_tree32', 'packed_shared32_control')
lab.BENCHMARKS = tuple(lab.Benchmark(name, 'Dependent SSE2 map including call/memory cost',
                       (str(DRIVER),), ('simd', 'sse2', 'micro'), ci_sharded=False) for name in NAMES)
original_compile = lab.compile_one
# Do not change discovery or any measurement/statistics/environment code.
gcc = lab.build_arg_parser().parse_args(lab.normalize_flag_valued_options(sys.argv[1:])).gcc


def compile_separate(compiler, benchmark, source, output, artifacts):
    index = NAMES.index(benchmark.name)
    iterations = 33554432 if index == 3 else 268435456
    output.parent.mkdir(parents=True, exist_ok=True)
    output.unlink(missing_ok=True)
    kernel = output.with_suffix('.kernel.o')
    driver = output.with_suffix('.driver.o')
    commands = [
        [compiler.executable, *compiler.flags, '-msse2', '-DONLY128', '-DMASK_KERNELS_ONLY', '-c', str(KERNELS), '-o', str(kernel)],
        [gcc, '-O2', '-msse2', '-fno-lto', f'-DCASE={index}', f'-DITERATIONS={iterations}', '-c', str(DRIVER), '-o', str(driver)],
        [gcc, '-fno-lto', str(driver), str(kernel), '-o', str(output)],
    ]
    for command in commands:
        result = lab.invoke(command, timeout_s=180, cwd=ROOT)
        if not result['ok']:
            break
    result.update({'commands': commands, 'iterations': iterations,
                   'driver_compiler': gcc, 'source_sha256': lab.sha256_file(KERNELS),
                   'driver_sha256': lab.sha256_file(DRIVER),
                   'binary_sha256': lab.sha256_file(output) if result['ok'] else None,
                   'binary_bytes': output.stat().st_size if result['ok'] else 0,
                   'text_bytes': lab.text_size(output) if result['ok'] else 0})
    if artifacts:
        directory = artifacts / benchmark.name / compiler.key
        directory.mkdir(parents=True, exist_ok=True)
        lab.write_text(directory / 'compile.json', __import__('json').dumps(result, indent=2))
        asm = lab.invoke([compiler.executable, *compiler.flags, '-msse2', '-DONLY128', '-DMASK_KERNELS_ONLY', '-S', str(KERNELS), '-o', str(directory / 'kernels.s')], timeout_s=180, cwd=ROOT)
        if not asm['ok']:
            raise RuntimeError(asm)
    return result


lab.compile_one = compile_separate
if __name__ == '__main__':
    raise SystemExit(lab.main())
