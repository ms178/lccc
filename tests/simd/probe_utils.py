#!/usr/bin/env python3
"""Dependency-free process, host-ISA and diagnostic helpers for SIMD drivers.

Design constraints, all deliberate:

* No import of another *script* from tests/simd.  The earlier driver did
  ``from aos_pair_shapes import ...`` and only worked because Python puts the
  script's directory first on sys.path; that is an invisible coupling to an
  unrelated benchmark script.  This module is a library with no side effects.
* Every subprocess runs in its own process group and is killed as a group on
  timeout.  A compiler may spawn an assembler or linker that inherits the
  pipes; killing only the parent leaves ``communicate()`` blocked forever.
* ``run_rc`` never raises for a nonzero child status: a probe must be able to
  inspect the exit code *and* stderr.  ``run_rc_safe`` additionally folds
  infrastructure exceptions into a reserved rc so a missing toolchain or a
  timeout always produces a hard ERROR row instead of a Python traceback.
* ``is_diagnostic`` classifies a failed compile from stderr alone.  The old
  driver treated any failure as "diagnosed correctly", so an ICE, a missing
  toolchain or a timeout silently became a green row.
* ``host_cpu_flags`` reads /proc/cpuinfo (Linux) or the AArch64 ``Features``
  line; an unreadable file returns the empty set so callers fail closed.
"""
from __future__ import annotations

import os
from pathlib import Path
import shlex
import signal
import subprocess


class ProbeError(RuntimeError):
    """A process failed before it could supply a trustworthy result."""


def compiler(command: str) -> list[str]:
    """Split a compiler command; make relative executable paths absolute.

    Probes run with a temporary cwd, so a relative ``target/fastbuild/lccc``
    would otherwise stop resolving.
    """
    args = shlex.split(command)
    if not args:
        raise ValueError('compiler command must not be empty')
    if '/' in args[0]:
        args[0] = str(Path(args[0]).resolve())
    return args


def run_rc(command: list[str], cwd: Path, timeout: float) -> tuple[int, str, str]:
    """Return (returncode, stdout, stderr); never raise on a child failure."""
    try:
        proc = subprocess.Popen(
            command, cwd=cwd, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            text=True, errors='replace', start_new_session=True,
        )
    except OSError as exc:
        raise ProbeError(f'{shlex.join(command)}: {exc}') from exc
    try:
        out, err = proc.communicate(timeout=timeout)
    except BaseException as exc:
        try:
            os.killpg(proc.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        proc.communicate()
        if isinstance(exc, subprocess.TimeoutExpired):
            raise ProbeError(
                f'{shlex.join(command)}: timed out after {timeout}s'
            ) from exc
        if isinstance(exc, (KeyboardInterrupt, SystemExit)):
            raise
        raise ProbeError(f'{shlex.join(command)}: {exc}') from exc
    return proc.returncode, out, err


def run_rc_safe(command: list[str], cwd: Path,
                timeout: float) -> tuple[int, str, str]:
    """``run_rc`` with infrastructure exceptions folded into a synthetic rc.

    A missing compiler, a timeout or a process-group kill must become a hard
    ERROR row with a receipt, not an uncaught Python traceback that loses the
    whole report.  rc == -1 is reserved for that case; the stderr prefix is in
    ICE_MARKERS so ``is_diagnostic`` can never mistake it for a real
    user-facing diagnostic.
    """
    try:
        return run_rc(command, cwd, timeout)
    except ProbeError as exc:
        return -1, '', f'probe infrastructure failure: {exc}'


def host_cpu_flags() -> frozenset[str]:
    """CPUID feature tokens of the *host*, not of the compilation target."""
    try:
        for line in Path('/proc/cpuinfo').read_text(
            encoding='utf-8', errors='replace'
        ).splitlines():
            key, sep, value = line.partition(':')
            if sep and key.strip() in ('flags', 'Features', 'features'):
                return frozenset(value.split())
    except OSError:
        pass
    return frozenset()


# A compile failure is only a legitimate "diagnosed" result when the compiler
# said something understandable.  An empty message, an ICE, a toolchain error
# or a timeout is infrastructure, never evidence about the finding.
ICE_MARKERS = (
    'internal compiler error', 'ice:', 'panicked', 'panic:', 'traceback',
    'segmentation fault', 'bus error', 'assertion failed', 'timed out',
    'timeout', 'killed', 'no such file', 'not found', 'permission denied',
    'cannot execute', 'out of memory', 'probe infrastructure failure',
)
# A compiler crash is a *narrower* concept than an infrastructure failure.
# SA-07's oracle may only count a compile failure as a confirmed ICE when the
# compiler actually crashed; a feature diagnostic or a bad flag must stay a
# hard ERROR.  Keeping these two sets separate is what stops a broken
# toolchain from masquerading as evidence.
CRASH_MARKERS = (
    'internal compiler error', 'ice:', 'panicked', 'panic:', 'traceback',
    'segmentation fault', 'bus error', 'sigsegv', 'sigabrt',
    'assertion failed', 'unreachable executed', 'stack overflow',
    'core dumped', 'signal 11', 'signal 6',
)


def is_crash(stderr: str) -> str:
    """Return the crash marker found in compiler stderr, or '' if none."""
    low = stderr.lower()
    return next((marker for marker in CRASH_MARKERS if marker in low), '')


def compiler_crash(rc: int, stderr: str) -> str:
    """Identify a compiler crash from (rc, stderr), or return ''.

    A negative subprocess return code is a POSIX signal; that is a crash even
    when the compiler printed nothing at all.
    """
    low = stderr.lower()
    if 'probe infrastructure failure' in low:
        return ''          # rc == -1 is our sentinel, not a POSIX signal
    marker = is_crash(low)
    if marker:
        return marker
    if rc < 0:
        return f'signal {-rc}'
    return ''

DIAGNOSTIC_MARKERS = (
    'error:', 'warning:', 'diagnostic', 'requires', 'required', 'target specific',
    'unsupported', 'unrecognized', 'unknown', 'invalid', 'illegal', 'operand',
    'mismatch', 'inlining failed', 'always_inline', 'avx512', 'avx2', 'vinserti128',
    'immediate', 'feature', 'not enabled', 'bad register', 'undefined',
)


def is_diagnostic(stderr: str, hints: tuple[str, ...] = ()) -> tuple[bool, str]:
    """Classify a failed compile.  Returns (is_real_diagnostic, reason)."""
    low = stderr.lower()
    if not low.strip():
        return False, 'empty compiler stderr'
    for marker in ICE_MARKERS:
        if marker in low:
            return False, f'infrastructure/ICE marker {marker!r}'
    for hint in hints:
        if hint.lower() in low:
            return True, f'expected diagnostic {hint!r}'
    for marker in DIAGNOSTIC_MARKERS:
        if marker in low:
            return True, f'diagnostic-shaped stderr ({marker!r})'
    return False, 'unclassified compile failure (no diagnostic marker)'


def write_json(path: Path, payload: dict) -> None:
    """Write a JSON receipt, creating parent directories.

    The earlier driver called ``path.write_text`` directly, so
    ``--json artifacts/receipts/audit.json`` crashed when the directory did
    not exist.
    """
    import json

    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + '\n',
                    encoding='utf-8')
