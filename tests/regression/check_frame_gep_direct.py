#!/usr/bin/env python3
"""Constant-offset frame addresses must address memory directly.

`tests/regression/frame_gep_direct.c` keeps eight escaped FSE-like states in
one local struct under high register pressure (zstd's sequence decoder shape).
A compiler that spills `&q.s[k]` and reloads that pointer before every field
access pays an extra dependent load per access. Fail on any reloaded spilled
frame address that is then dereferenced, and require the exact GCC output.
The pre-fix compiler emitted eight such reload/dereference pairs here.
"""
import argparse
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / 'tests/regression/frame_gep_direct.c'


def reloaded_frame_derefs(lines):
    frame_regs, spills, found = {}, {}, []
    for index, line in enumerate(lines):
        text = line.strip()
        lea = re.fullmatch(r'leaq (-?\d+)\(%r[sb]p\), %(r\w+)', text)
        if lea:
            frame_regs[lea.group(2)] = lea.group(1)
            continue
        store = re.fullmatch(r'movq %(r\w+), (-?\d+\(%r[sb]p\))', text)
        if store and store.group(1) in frame_regs:
            spills[store.group(2)] = frame_regs[store.group(1)]
            continue
        reload = re.fullmatch(r'movq (-?\d+\(%r[sb]p\)), %(r\w+)', text)
        if reload and reload.group(1) in spills:
            reg = reload.group(2)
            for later in lines[index + 1:index + 4]:
                if re.search(r'\(%' + reg + r'\)', later):
                    found.append((index + 1, text, later.strip()))
        written = re.fullmatch(r'\S+ .*, %(r\w+)', text)
        if written and written.group(1) in frame_regs:
            frame_regs.pop(written.group(1))
    return found


def selftest():
    bad = ['leaq 64(%rsp), %rax', 'movq %rax, 144(%rsp)', 'movq 144(%rsp), %rcx', 'movq (%rcx), %rdi']
    good = ['leaq 64(%rsp), %rax', 'movq %rax, 144(%rsp)', 'movq 64(%rsp), %rdi']
    clobbered = ['leaq 64(%rsp), %rax', 'addq $8, %rax', 'movq %rax, 144(%rsp)',
                 'movq 144(%rsp), %rcx', 'movq (%rcx), %rdi']
    assert len(reloaded_frame_derefs(bad)) == 1
    assert not reloaded_frame_derefs(good)
    assert not reloaded_frame_derefs(clobbered)


def switched(disabled):
    env = {k: v for k, v in os.environ.items() if k != 'CCC_NO_FRAME_GEP_DIRECT'}
    if disabled:
        env['CCC_NO_FRAME_GEP_DIRECT'] = '1'
    return env


def run(cmd, env=None):
    result = subprocess.run(cmd, capture_output=True, text=True, env=env)
    if result.returncode:
        sys.exit(f'command failed: {" ".join(map(str, cmd))}\n{result.stderr}')
    return result.stdout


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--lccc', default=os.environ.get('CCC', str(ROOT / 'target/fastbuild/lccc')))
    parser.add_argument('--gcc', default=os.environ.get('GCC', 'gcc'))
    args = parser.parse_args()
    selftest()
    with tempfile.TemporaryDirectory() as work:
        work = Path(work)
        asm = work / 'frame.s'
        run([args.lccc, '-O2', '-march=x86-64-v3', '-S', SOURCE, '-o', asm], env=switched(False))
        found = reloaded_frame_derefs(asm.read_text().splitlines())
        if found:
            sys.exit('FAIL: reloaded frame-address dereferences:\n' + '\n'.join(map(str, found)))
        # Negative control: the kill switch restores the pre-fix code, so the
        # detector demonstrably sees the defect and the switch reaches it.
        control = work / 'frame-off.s'
        run([args.lccc, '-O2', '-march=x86-64-v3', '-S', SOURCE, '-o', control], env=switched(True))
        if not reloaded_frame_derefs(control.read_text().splitlines()):
            sys.exit('FAIL: negative control: CCC_NO_FRAME_GEP_DIRECT=1 shows no reloaded '
                     'frame-address dereference, so this fixture no longer exercises the fix')
        run([args.gcc, '-O2', SOURCE, '-o', work / 'gcc'])
        reference = run([work / 'gcc'])
        for disabled in (False, True):
            exe = work / f'lccc-{int(disabled)}'
            run([args.lccc, '-O2', '-march=x86-64-v3', SOURCE, '-o', exe], env=switched(disabled))
            mine = run([exe])
            if mine != reference:
                sys.exit(f'FAIL: output (CCC_NO_FRAME_GEP_DIRECT={int(disabled)}) {mine!r} != GCC {reference!r}')
    print('check_frame_gep_direct: PASS (no reloaded frame addresses; negative control fires; '
          'GCC output matches with and without the fix)')


if __name__ == '__main__':
    main()
