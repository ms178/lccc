#!/usr/bin/env python3
"""Execute the native AVX-512 masked scalar-forwarding witness when possible.

`scalar_masked_loads_native_avx512` assembles three stages of a masked
`vmovs{s,d}` witness (original, store-forwarded, full peephole pipeline) and
runs 1536 exact checks per stage on the host CPU. It is `#[ignore]`d in the
ordinary `cargo test` run because a host without AVX512F cannot execute it;
an early `return` inside the test would report a pass that never executed.

This gate runs exactly that test with `--ignored --exact` and requires
`1 passed; 0 failed`. A host whose kernel-reported flags lack avx512f gets an
explicit `SKIP` line (the witness is not executable there; no emulator is
assumed), never a silent pass. `--require` turns that SKIP into a failure for
a capable host whose capability must not be lost.
"""
import argparse
import os
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
TEST = 'backend::x86::codegen::peephole::passes::scalar_transfer_tests::scalar_masked_loads_native_avx512'


def host_has_avx512f(cpuinfo=Path('/proc/cpuinfo')):
    try:
        text = cpuinfo.read_text()
    except OSError:
        return False
    flags = [line.split(':', 1)[1].split() for line in text.splitlines()
             if line.startswith('flags') and ':' in line]
    return bool(flags) and all('avx512f' in row for row in flags)


def verdict(returncode, output):
    """Exactly one executed, passing test; anything else is a failure."""
    summary = re.findall(r'test result: (\w+)\. (\d+) passed; (\d+) failed; (\d+) ignored', output)
    executed = [row for row in summary if int(row[1]) or int(row[2])]
    return (returncode == 0 and len(executed) == 1
            and executed[0][:3] == ('ok', '1', '0'))


def self_test():
    good = 'test result: ok. 0 passed; 0 failed; 0 ignored\ntest result: ok. 1 passed; 0 failed; 0 ignored\n'
    assert verdict(0, good)
    assert not verdict(0, 'test result: ok. 0 passed; 0 failed; 1 ignored\n'), 'ignored is not executed'
    assert not verdict(101, 'test result: FAILED. 0 passed; 1 failed; 0 ignored\n')
    assert not verdict(0, good + good), 'filter matched more than one test'
    assert not verdict(1, good), 'nonzero exit'
    assert not verdict(0, ''), 'no summary'
    print('check_avx512_scalar_witness: self-test PASS')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--require', action='store_true',
                        help='fail instead of SKIP when the host lacks AVX512F')
    parser.add_argument('--self-test', action='store_true')
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return 0
    if not host_has_avx512f():
        if args.require:
            print('FAIL: --require given but the host does not report avx512f', file=sys.stderr)
            return 1
        print('SKIP: host lacks AVX512F; native masked scalar-forwarding witness not executed')
        return 0
    flags_file = ROOT / 'target/lccc-rustflags'
    env = dict(os.environ)
    if flags_file.is_file():
        env['RUSTFLAGS'] = flags_file.read_text().strip()
    command = ['cargo', 'test', '--profile', 'fastbuild', '--lib', '--locked',
               '-j', os.environ.get('CARGO_BUILD_JOBS', '2'),
               '--', '--ignored', '--exact', TEST]
    result = subprocess.run(command, cwd=ROOT, env=env, capture_output=True, text=True,
                            timeout=3600)
    output = result.stdout + result.stderr
    if not verdict(result.returncode, output):
        print(output[-8000:], file=sys.stderr)
        print(f'FAIL: {TEST} did not execute and pass exactly once', file=sys.stderr)
        return 1
    print('check_avx512_scalar_witness: PASS (native AVX-512 masked witness executed: 1 passed)')
    return 0


if __name__ == '__main__':
    sys.exit(main())
