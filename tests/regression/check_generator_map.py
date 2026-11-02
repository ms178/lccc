#!/usr/bin/env python3
"""Runtime parity plus per-kernel, fail-closed generator-map trace contracts."""
from __future__ import annotations

from collections import Counter
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]
POSITIVES = {f'gen_{letter}' for letter in 'abcdef'}
REFUSALS = {'gen_g', 'gen_volatile', 'gen_constant'}
TRACE = re.compile(r'^\[VEC-GEN\] fn=(\S+) ', re.MULTILINE)


def checked(command: list[str], env: dict[str, str] | None = None) -> str:
    result = subprocess.run(command, env=env, cwd=ROOT, capture_output=True,
                            text=True, timeout=180)
    if result.returncode:
        raise RuntimeError(f'{command!r} exited {result.returncode}\n{result.stdout}\n{result.stderr}')
    return result.stdout + result.stderr


def capture_trace(command: list[str], env: dict[str, str]) -> Counter:
    # A failing producer is never interpreted as an empty or partially complete
    # successful trace. Count only after checked() has validated the exit code.
    return Counter(TRACE.findall(checked(command, env)))


def validate_trace(counts: Counter, enabled: bool) -> None:
    # Each positive fixture has exactly one candidate loop and must produce
    # exactly one trace event. Comparing only the key set lets a duplicate
    # transformation event pass whenever all expected functions are present.
    expected = Counter({name: 1 for name in POSITIVES}) if enabled else Counter()
    unexpected = set(counts) & REFUSALS
    if unexpected:
        raise RuntimeError(f'forbidden shapes were vectorized: {sorted(unexpected)}')
    if counts != expected:
        raise RuntimeError(f'generator trace mismatch: expected {dict(expected)}, got {dict(counts)}')


def main() -> int:
    compiler = os.environ.get('CCC', 'target/fastbuild/lccc')
    reference = shutil.which('gcc-16') or shutil.which('gcc')
    if reference is None:
        raise RuntimeError('GCC reference compiler is required')
    source = 'tests/regression/generator_map_vec.c'
    flags = ['-std=gnu17', '-O2', '-march=x86-64-v3', '-fwrapv']
    # Deterministic A/B policy: the caller cannot silently disable the positive
    # arm through the kill switch, force-SSE2, or a preexisting trace setting.
    env = {k: v for k, v in os.environ.items()
           if k not in ('CCC_NO_GENERATOR_MAP', 'LCCC_FORCE_SSE2', 'LCCC_DEBUG_VECTORIZE')}
    with tempfile.TemporaryDirectory(prefix='lccc-generator-gate-') as td:
        reference_binary = str(Path(td) / 'reference')
        checked([reference, '-O2', '-fwrapv', source, '-o', reference_binary], env)
        expected = checked([reference_binary], env).strip()
        if expected != 'generator_map_vec: ok':
            raise RuntimeError(f'GCC reference self-check failed: {expected}')
        for label, enabled in [('default', True), ('kill-switch', False)]:
            arm_env = dict(env)
            if not enabled:
                arm_env['CCC_NO_GENERATOR_MAP'] = '1'
            binary = str(Path(td) / label)
            checked([compiler, *flags, source, '-o', binary], arm_env)
            actual = checked([binary], arm_env).strip()
            if actual != expected:
                raise RuntimeError(f'{label}: output {actual!r}, expected {expected!r}')
            arm_env['LCCC_DEBUG_VECTORIZE'] = '1'
            assembly = str(Path(td) / f'{label}.s')
            counts = capture_trace([compiler, *flags, '-S', '-o', assembly, source], arm_env)
            validate_trace(counts, enabled)
            if enabled:
                assembly_text = Path(assembly).read_text()
                for kernel in sorted(REFUSALS):
                    match = re.search(rf'^{re.escape(kernel)}:.*?^\.size\s+{re.escape(kernel)},',
                                      assembly_text, re.MULTILINE | re.DOTALL)
                    if match is None or 'vpackssdw' in match.group(0):
                        raise RuntimeError(f'{kernel}: refusal arm missing or packed code emitted')
            print(f'generator_map {label}: ok ({dict(counts)})')
    return 0


if __name__ == '__main__':
    try:
        raise SystemExit(main())
    except (RuntimeError, OSError, subprocess.TimeoutExpired) as error:
        print(f'FAIL: {error}', file=sys.stderr)
        raise SystemExit(1)
