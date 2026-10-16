#!/usr/bin/env python3
"""Fail-closed wrapping-domain differential + Memcheck gate.

Default: 24 wrapping-domain cases. --fixture signed-scale: 30 ABI/optimization
arms, each executing 16 real in-object signed-address wrap witnesses.
--fixture canonical: 60 arms, 82 nonoverflowing raw-wide/explicit-cast cases, both policies.

--fixture pointer: 60 ABI/optimization/policy arms for width-changing
integer/pointer conversions, plus 30 LP64 __int128 arms that explicitly test
implementation-defined high-bit pointer conversions (GCC sign-extends;
LCCC/Clang zero-extend on the pinned x86-64 targets).
--fixture pointer-i128: only the 30 focused LP64 implementation-defined arms.

Pass exact oracle executables explicitly; identities and commands are recorded.
Needs native and 32-bit libc/debug info. A missing Valgrind loader redirect is
an infrastructure failure, never a compiler pass or a silently skipped case.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess


OPTS = ('-O0', '-O1', '-O2', '-O3', '-Os')
POINTER_I128_PASS = 'pointer-to-int128 implementation-defined extension: PASS'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--lccc', required=True)
    parser.add_argument('--gcc', required=True)
    parser.add_argument('--clang', required=True)
    parser.add_argument('--work-dir', type=Path, required=True)
    parser.add_argument('--json', type=Path, required=True)
    parser.add_argument('--fixture', choices=('domains', 'signed-scale', 'canonical',
                                              'pointer', 'pointer-i128'), default='domains')
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    args.work_dir.mkdir(parents=True, exist_ok=True)
    args.json.parent.mkdir(parents=True, exist_ok=True)
    compilers = dict(lccc=args.lccc, gcc=args.gcc, clang=args.clang)
    # A caller's optimization kill switches must not silently change an oracle
    # arm. Match compare_optimizer_witnesses: compile with a clean CCC policy.
    environment = {k: v for k, v in os.environ.items() if not k.startswith('CCC_')}
    scale = args.fixture == 'signed-scale'
    canonical = args.fixture == 'canonical'
    pointer = args.fixture == 'pointer'
    pointer_i128_only = args.fixture == 'pointer-i128'
    pointer_i128 = pointer or pointer_i128_only
    pointer_i128_source = root / 'tests/regression/fixtures/pointer_to_i128.c'
    if pointer_i128_only:
        source = pointer_i128_source
    else:
        source = root / ('tests/regression/integer_pointer_width.c' if pointer else
                         'tests/regression/i32_address_canonical.c' if canonical else
                         'tests/regression/signed_scale_ring_wrap.c' if scale else
                         'tests/regression/wrapv_domains.c')
    if pointer:
        expected_outputs = ['integer pointer widths: PASS']
    elif canonical:
        expected_outputs = ['i32 address canonical: PASS 82']
    elif scale:
        expected_outputs = ['signed scale ring: PASS 16']
    elif pointer_i128_only:
        expected_outputs = []
    else:
        expected_outputs = ['28', '-1073665362', '1', '10']
    opts = OPTS if scale or canonical or pointer_i128 else ('-O2',)
    policies = ('-fwrapv', '-fno-wrapv') if canonical or pointer_i128 else ('-fwrapv',)
    configurations = [(opt, policy) for opt in opts for policy in policies]
    report = dict(complete=False, fixture=args.fixture,
                  compiler_environment_policy='remove inherited CCC_*',
                  compilers={}, compiler_identities={}, cases=[],
                  source_sha256=hashlib.sha256(source.read_bytes()).hexdigest(),
                  additional_sources={},
                  valgrind=subprocess.check_output(['valgrind', '--version'], text=True))
    if pointer:
        report['additional_sources'][str(pointer_i128_source.relative_to(root))] = hashlib.sha256(
            pointer_i128_source.read_bytes()).hexdigest()

    def save():
        temp = args.json.with_suffix('.tmp')
        temp.write_text(json.dumps(report, indent=2) + '\n')
        os.replace(temp, args.json)

    try:
        for name, cc in compilers.items():
            report['compilers'][name] = subprocess.check_output(
                [cc, '--version'], text=True, env=environment)
            resolved = shutil.which(cc)
            report['compiler_identities'][name] = dict(
                path=resolved, sha256=hashlib.sha256(Path(resolved).read_bytes()).hexdigest())

            if not pointer_i128_only:
                for abi, flags in [('x64', []), ('x86', ['-m32', '-msse2'])]:
                    for opt, policy in configurations:
                        binary = (args.work_dir / (name + '-' + abi + '-' + opt[1:] + '-' +
                                                   policy[1:])).resolve()
                        cmd = [cc, opt, policy, *flags, str(source), '-o', str(binary)]
                        build = subprocess.run(cmd, capture_output=True, text=True,
                                               timeout=60, env=environment)
                        if build.returncode:
                            raise RuntimeError(f'build failure: {cmd}\n{build.stderr}')
                        for case, expected in enumerate(expected_outputs):
                            log = args.work_dir / f'{name}-{abi}-{opt[1:]}-{policy[1:]}-{case}.log'
                            parameters = [] if scale or canonical or pointer else [str(case)]
                            command = ['valgrind', '--error-exitcode=97', '--leak-check=full',
                                       '--log-file=' + str(log), str(binary), *parameters]
                            run = subprocess.run(command, capture_output=True, text=True,
                                                 timeout=60, env=environment)
                            row = dict(compiler=name, abi=abi, opt=opt, policy=policy,
                                       case=case, build=cmd, command=command,
                                       status=run.returncode,
                                       binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
                                       stdout=run.stdout.strip(), stderr=run.stderr,
                                       memcheck=log.read_text())
                            row['pass'] = run.returncode == 0 and row['stdout'] == expected
                            report['cases'].append(row)
                            save()
                            if not row['pass']:
                                raise RuntimeError(
                                    f'FAIL: {name}/{abi}/{opt}/{case}; see {args.json}')

            if pointer_i128:
                # C leaves pointer-to-integer results implementation-defined.
                # Test the observed x86-64 compiler policies rather than
                # treating GCC's sign-extension choice as a universal rule.
                # The expected high-bit extension is selected per compiler:
                # GCC16 sign-extends here; LCCC and Clang23 zero-extend.
                sign_extend = '1' if name == 'gcc' else '0'
                for opt, policy in configurations:
                    binary = (args.work_dir / (name + '-pointer-i128-' + opt[1:] + '-' +
                                               policy[1:])).resolve()
                    cmd = [cc, opt, policy, '-m64',
                           '-DPOINTER_I128_SIGN_EXTENDS_HIGH_BIT=' + sign_extend,
                           str(pointer_i128_source), '-o', str(binary)]
                    build = subprocess.run(cmd, capture_output=True, text=True,
                                           timeout=60, env=environment)
                    if build.returncode:
                        raise RuntimeError(f'build failure: {cmd}\n{build.stderr}')
                    log = args.work_dir / f'{name}-pointer-i128-{opt[1:]}-{policy[1:]}.log'
                    command = ['valgrind', '--error-exitcode=97', '--leak-check=full',
                               '--log-file=' + str(log), str(binary)]
                    run = subprocess.run(command, capture_output=True, text=True,
                                         timeout=60, env=environment)
                    row = dict(compiler=name, abi='x64', opt=opt, policy=policy,
                               fixture='pointer_to_i128.c',
                               implementation_defined_policy=(
                                   'sign-extend-high-bit' if sign_extend == '1' else
                                   'zero-extend'),
                               build=cmd, build_stderr=build.stderr, command=command,
                               status=run.returncode,
                               binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
                               stdout=run.stdout.strip(), stderr=run.stderr,
                               memcheck=log.read_text())
                    row['pass'] = run.returncode == 0 and row['stdout'] == POINTER_I128_PASS
                    report['cases'].append(row)
                    save()
                    if not row['pass']:
                        raise RuntimeError(
                            f'FAIL: {name}/pointer-i128/{opt}/{policy}; see {args.json}')

        report['complete'] = True
        print(f"PASS: {len(report['cases'])} exact-oracle/ABI/domain Memcheck cases")
    finally:
        save()


if __name__ == '__main__':
    main()
