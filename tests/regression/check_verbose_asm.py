#!/usr/bin/env python3
"""Presentation-only -fverbose-asm must preserve native ELF bytes on all targets."""
import argparse
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
SOURCE = '''volatile int sink;
int visible(int x) { if (x > 0) {sink=x; return x+7;} return -x; }
int other(int x) {return visible(x)+visible(x+1);}
'''


def execute(command):
    env = dict(os.environ, LCCC_STRICT_OPTIONS='1', LC_ALL='C')
    proc = subprocess.run(list(map(str, command)), text=True, capture_output=True, env=env, timeout=30)
    if proc.returncode or proc.stderr:
        raise AssertionError(f'{command}: {proc.returncode}\n{proc.stdout}\n{proc.stderr}')


def run(compiler):
    compiler = compiler if isinstance(compiler, list) else [str(compiler)]
    with tempfile.TemporaryDirectory(prefix='lccc-verbose-asm-') as tmp:
        root = Path(tmp)
        source = root/'fixture.c'
        source.write_text(SOURCE)
        count = 0
        for target in ['x86', 'i686', 'arm', 'riscv']:
            alias = root/f'lccc-{target}'
            alias.symlink_to(Path(compiler[0]).resolve())
            command = [str(alias), *compiler[1:]]
            for optimization in ['-O0', '-O2', '-Os']:
                texts = []
                objects = []
                for label, options in [('plain', []), ('verbose', ['-fverbose-asm']),
                        ('disabled', ['-fverbose-asm', '-fno-verbose-asm'])]:
                    assembly = root/f'{label}.s'
                    obj = root/f'{label}.o'
                    execute([*command, optimization, *options, '-S', source, '-o', assembly])
                    texts.append(assembly.read_text())
                    execute([*command, optimization, *options, '-c', source, '-o', obj])
                    objects.append(obj.read_bytes())
                if 'LCCC -fverbose-asm' not in texts[1] or 'IR instructions' not in texts[1]:
                    raise AssertionError(f'{target} {optimization}: no function commentary')
                if texts[0] != texts[2]:
                    raise AssertionError(f'{target} {optimization}: disabling commentary changed assembly')
                if not objects[0] == objects[1] == objects[2]:
                    raise AssertionError(f'{target} {optimization}: commentary changed ELF bytes')
                count += 1
    print(f'verbose assembly: PASS ({count} cross-target modes, native ELF byte equality)')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--lccc', default=str(ROOT/'target/fastbuild/lccc'))
    run(parser.parse_args().lccc)
