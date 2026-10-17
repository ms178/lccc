#!/usr/bin/env python3
"""Inliner contract for devirtualized per-iteration predicates.

See `tests/regression/inline_devirt_hot_leaf.c`. The pre-change compiler left
`call pred_a`/`call pred_b` in count_a/count_b: the template parameter had
been constant-folded, but the IR still held an indirect call the inliner could
not see. sum_a/sum_b carry the same shape in callers already past the
caller-size cap, with scorers above the ordinary static limit; only the
devirtualized hot-leaf budget inlines those. sum_i/sum_s repeat it with
direct calls: the declared-inline scorer is admitted (hot-leaf direct
extension), the plain-static one keeps the historical caps. matches_i/matches_s
call a match-length scan WITH its own loop (zstd's ZSTD_count) on the match
path of a capped caller's loop: the declared-inline scan is admitted (hot loop
helper), the plain-static twin keeps the call.

Gates:
  * default build: no calls survive in count_a, count_b, sum_a or sum_b;
  * default build: no call survives in sum_i or matches_i;
  * default build: sum_s and matches_s keep their calls (direct plain-static
    callees are deliberately not hot-leaf or hot-loop-helper candidates);
  * negative controls: CCC_NO_INLINE_DEVIRT=1 must leave the predicate calls
    in count_a/count_b, CCC_NO_INLINE_HOT_LEAF=1 must leave the scorer
    calls in sum_a/sum_b while count_a/count_b stay inlined,
    and CCC_NO_INLINE_HOT_DIRECT=1 must leave the calls in sum_i and
    matches_i -- so the
    check demonstrably detects each defect and each switch reaches its
    extension;
  * the program output matches GCC in the default build and with every
    inliner extension switch set (a kill switch must stay correct).
"""
import argparse
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / 'tests/regression/inline_devirt_hot_leaf.c'
DEVIRT = ('count_a', 'count_b')
HOT_LEAF = ('sum_a', 'sum_b')
HOT_DIRECT = ('sum_i', 'matches_i')
PLAIN_STATIC = ('sum_s', 'matches_s')
SWITCHES = (
    'CCC_NO_INLINE_DEVIRT',
    'CCC_NO_INLINE_HOT_LEAF',
    'CCC_NO_INLINE_JOINT_FIXPOINT',
    'CCC_NO_INLINE_SNAPSHOT_REFRESH',
    'CCC_NO_INLINE_SNAPSHOT_CLEANUP',
    'CCC_NO_INLINE_CALLER_CLEANUP',
    'CCC_NO_INLINE_HOT_DIRECT',
)


def calls_in(asm, name):
    body = re.search(rf'^{name}:\n(.*?)^\.size {name},', asm, re.S | re.M)
    if not body:
        sys.exit(f'FAIL: function {name} not found')
    return re.findall(r'^\s*call\s+(\S+)', body.group(1), re.M)


def run(cmd, env=None):
    result = subprocess.run(cmd, capture_output=True, text=True, env=env)
    if result.returncode:
        sys.exit(f'command failed: {" ".join(map(str, cmd))}\n{result.stderr}')
    return result.stdout


def switch_env(switch):
    env = {k: v for k, v in os.environ.items() if k not in SWITCHES}
    if switch:
        env[switch] = '1'
    return env


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--lccc', default=os.environ.get('CCC', str(ROOT / 'target/fastbuild/lccc')))
    parser.add_argument('--gcc', default=os.environ.get('GCC', 'gcc'))
    args = parser.parse_args()
    with tempfile.TemporaryDirectory() as work:
        work = Path(work)

        def calls(switch, names):
            asm = work / f'inline-{switch or "default"}.s'
            run([args.lccc, '-O2', '-march=x86-64-v3', '-S', SOURCE, '-o', asm], switch_env(switch))
            text = asm.read_text()
            return {name: calls_in(text, name) for name in names}

        default = calls(None, DEVIRT + HOT_LEAF + HOT_DIRECT + PLAIN_STATIC)
        leftovers = {k: v for k, v in default.items() if v and k not in PLAIN_STATIC}
        if leftovers:
            sys.exit(f'FAIL: calls survived: {leftovers}')
        kept = [name for name in PLAIN_STATIC if not default[name]]
        if kept:
            sys.exit(f'FAIL: the plain-static direct callee was inlined into the capped {kept}; '
                     'direct plain-static calls must keep the historical caps')
        direct = calls('CCC_NO_INLINE_HOT_DIRECT', DEVIRT + HOT_LEAF + HOT_DIRECT)
        if not all(direct[name] for name in HOT_DIRECT) or any(direct[name] for name in DEVIRT + HOT_LEAF):
            sys.exit(f'FAIL: negative control: CCC_NO_INLINE_HOT_DIRECT=1 must leave exactly the '
                     f'sum_i and matches_i calls, got {direct}')
        if not all(calls('CCC_NO_INLINE_DEVIRT', DEVIRT).values()):
            sys.exit('FAIL: negative control: CCC_NO_INLINE_DEVIRT=1 no longer leaves the '
                     'predicate calls, so this fixture no longer exercises devirtualization')
        hot = calls('CCC_NO_INLINE_HOT_LEAF', DEVIRT + HOT_LEAF)
        if not all(hot[name] for name in HOT_LEAF) or any(hot[name] for name in DEVIRT):
            sys.exit(f'FAIL: negative control: CCC_NO_INLINE_HOT_LEAF=1 must leave exactly the '
                     f'sum_a/sum_b scorer calls, got {hot}')

        run([args.gcc, '-O2', SOURCE, '-o', work / 'gcc'])
        reference = run([work / 'gcc'])
        for switch in (None,) + SWITCHES:
            exe = work / f'lccc-{switch or "default"}'
            run([args.lccc, '-O2', '-march=x86-64-v3', SOURCE, '-o', exe], switch_env(switch))
            mine = run([exe])
            if mine != reference:
                sys.exit(f'FAIL: output with {switch or "default build"} {mine!r} != GCC {reference!r}')
    print('check_inline_devirt_hot_leaf: PASS (devirtualized predicates, hot leaves, the '
          'declared-inline direct leaf and loop helper inlined; the plain-static direct twins '
          'kept; all three negative controls fire; GCC output matches under every inliner switch)')


if __name__ == '__main__':
    main()
