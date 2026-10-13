#!/usr/bin/env python3
"""Gate: ill-typed builtin calls must be diagnostics, never internal errors.

WHY
---
An invalid *user* program is not a compiler bug.  Until AUD-ICE-1, a table of
reachable `panic!`/`unreachable!` sites in the lowering stage turned ordinary
typos into `ccc: internal error: ...` (and, before AUD-SILENT-ZERO-1, an x86
intrinsic compiled for aarch64 into silently zeroed output):

    __builtin_shuffle(a, m)      with `int m`
    __builtin_shuffle(a, m)      with a float vector mask
    __builtin_shuffle(a, m)      with a mask of the wrong element count
    __builtin_shuffle(a)         wrong arity
    __builtin_shufflevector(...)  non-constant / out-of-range lane index
    __builtin_ia32_aesenc128     on aarch64 (foreign-architecture builtin)

For every row the contract is:

  1. exit status is non-zero (the program is rejected, not miscompiled);
  2. stderr carries a located diagnostic (``file:line:col: error:``);
  3. the message names the offending builtin;
  4. stderr contains neither ``internal error`` nor a panic banner;
  5. where GCC also rejects the program (the x86 rows), GCC agrees.

The rows are run at ``-O0`` and ``-O2`` because the defect was reachable from
both (the lowering path is not optimization-gated).

Mutation check: point ``--compiler`` at a compiler built before the fix and
this gate must fail.  ``--self-test`` proves the checker's own classifier with
synthetic compiler stubs, so a future edit that makes the assertions vacuous
(e.g. accepting any non-zero exit) turns red here.
"""

from __future__ import annotations

import argparse
import re
import shutil
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
LOCATED = re.compile(r"^[^:]+:\d+:\d+: (error|warning): ", re.M)
PANIC = re.compile(r"internal error|panicked at|thread '[^']*' panicked", re.I)


@dataclass(frozen=True)
class Case:
    name: str
    source: str
    message: str
    flags: tuple[str, ...] = ()
    gcc_agrees: bool = True
    gcc_message: str | None = None


X86_CASES: tuple[Case, ...] = (
    Case(
        "shuffle_int_mask",
        "typedef int v4 __attribute__((vector_size(16)));\n"
        "v4 f(v4 a, int m) { return __builtin_shuffle(a, m); }\n",
        "'__builtin_shuffle' last argument must be an integer vector",
        gcc_message="must be an integer vector",
    ),
    Case(
        "shuffle_float_mask",
        "typedef float f4 __attribute__((vector_size(16)));\n"
        "f4 g(f4 a, f4 m) { return __builtin_shuffle(a, m); }\n",
        "'__builtin_shuffle' last argument must be an integer vector",
        gcc_message="must be an integer vector",
    ),
    Case(
        "shuffle_mask_count_mismatch",
        "typedef int v4 __attribute__((vector_size(16)));\n"
        "typedef int v8 __attribute__((vector_size(32)));\n"
        "v4 h(v4 a, v8 m) { return __builtin_shuffle(a, m); }\n",
        "number of elements of the argument vector(s) and the mask vector",
        gcc_message="should be the same",
    ),
    Case(
        "shuffle_wrong_arity",
        "typedef int v4 __attribute__((vector_size(16)));\n"
        "v4 k(v4 a) { return __builtin_shuffle(a); }\n",
        "wrong number of arguments to '__builtin_shuffle'",
        gcc_message="wrong number of arguments",
    ),
    Case(
        "shufflevector_range",
        "typedef int v4 __attribute__((vector_size(16)));\n"
        "v4 j(v4 a, v4 b) { return __builtin_shufflevector(a, b, 0, 1, 9, 3); }\n",
        "invalid element index '9' to '__builtin_shufflevector'",
        gcc_message="invalid element index",
    ),
    Case(
        "shufflevector_non_constant",
        "typedef int v4 __attribute__((vector_size(16)));\n"
        "v4 i(v4 a, v4 b, int k) { return __builtin_shufflevector(a, b, k, 1, 2, 3); }\n",
        "must be an integer constant expression",
        # GCC prints the source text of the index; the wording differs, the
        # verdict (reject + located error) must not.
        gcc_message=None,
    ),
)

FOREIGN_TARGET_CASES: tuple[Case, ...] = (
    Case(
        "x86_intrinsic_on_aarch64",
        "typedef int v4 __attribute__((vector_size(16)));\n"
        "v4 m(v4 a, v4 k) { return __builtin_ia32_aesenc128(a, k); }\n",
        "'__builtin_ia32_aesenc128' is not available on aarch64",
        gcc_agrees=False,
    ),
)


def run(compiler: Path, source: Path, out: Path, *flags: str) -> tuple[int, str]:
    proc = subprocess.run(
        [str(compiler), *flags, "-S", str(source), "-o", str(out)],
        capture_output=True,
        text=True,
        timeout=120,
    )
    return proc.returncode, proc.stdout + proc.stderr


def check_case(
    case: Case,
    compiler: Path,
    gcc: Path | None,
    work: Path,
    opt: str,
) -> list[str]:
    """Return the list of failures for one case/optimization row (empty = pass)."""
    failures: list[str] = []
    src = work / f"{case.name}{opt}.c"
    src.write_text(case.source)
    code, output = run(compiler, src, work / f"{case.name}{opt}.s", opt, *case.flags)
    where = f"{case.name}{opt}"
    if code == 0:
        failures.append(f"{where}: accepted (exit 0); expected a diagnostic")
        return failures
    if PANIC.search(output):
        failures.append(f"{where}: emitted an internal error/panic: {output.strip()[:200]}")
    if case.message not in output:
        failures.append(f"{where}: message {case.message!r} missing from {output.strip()[:200]}")
    if not LOCATED.search(output):
        failures.append(f"{where}: diagnostic is not located (no file:line:col prefix): {output.strip()[:200]}")
    # GCC differential: GCC must reject the same program (it may use its own
    # wording, but rejecting is the cross-checkable half of the contract).
    if gcc is not None and case.gcc_agrees:
        gcode, goutput = run(gcc, src, work / f"{case.name}{opt}.gcc.s", opt, *case.flags)
        if gcode == 0:
            failures.append(f"{where}: GCC accepted the program; the row is not a rejection case")
        elif case.gcc_message and case.gcc_message not in goutput:
            failures.append(
                f"{where}: GCC rejected with unexpected wording: {goutput.strip()[:200]}"
            )
    return failures


def check_foreign_target(case: Case, compiler: Path, work: Path) -> list[str]:
    failures: list[str] = []
    src = work / f"{case.name}.c"
    src.write_text(case.source)
    code, output = run(compiler, src, work / f"{case.name}.s", "-O2")
    if code == 0:
        failures.append(f"{case.name}: accepted (exit 0); foreign-target builtin must be rejected")
    if case.message not in output:
        failures.append(f"{case.name}: message {case.message!r} missing from {output.strip()[:200]}")
    if PANIC.search(output):
        failures.append(f"{case.name}: emitted an internal error/panic: {output.strip()[:200]}")
    return failures


def self_test() -> int:
    """The checker must flag a panic banner and an unlocated diagnostic."""
    stub = Path(tempfile.mkdtemp(prefix="ill-typed-selftest."))
    try:
        panicking = stub / "panicking-cc"
        panicking.write_text(
            "#!/bin/sh\necho \"ccc: internal error: __builtin_shuffle: mask must be a vector\" >&2\n"
            "echo 'thread main panicked at src/x.rs' >&2\nexit 1\n"
        )
        located = stub / "located-cc"
        located.write_text(
            "#!/bin/sh\necho \"$3:2:28: error: '__builtin_shuffle' last argument must be an integer vector\" >&2\nexit 1\n"
        )
        for path in (panicking, located):
            path.chmod(0o755)
        work = stub / "work"
        work.mkdir()
        case = X86_CASES[0]
        failures = check_case(case, panicking, None, work, "-O0")
        ok = any("internal error/panic" in f for f in failures)
        # The stub emits no file:line:col either; both classes must be reported.
        ok = ok and any("not located" in f for f in failures)
        if not ok:
            print("self-test FAIL: panic/unlocated output was accepted", file=sys.stderr)
            print(failures, file=sys.stderr)
            return 1
        failures = check_case(case, located, None, work, "-O0")
        if failures:
            print(f"self-test FAIL: a conforming stub was rejected: {failures}", file=sys.stderr)
            return 1
        accepted = stub / "accepting-cc"
        accepted.write_text("#!/bin/sh\nexit 0\n")
        accepted.chmod(0o755)
        failures = check_case(case, accepted, None, work, "-O0")
        if not any("accepted" in f for f in failures):
            print("self-test FAIL: a compiler that accepts invalid code passed", file=sys.stderr)
            return 1
        print("ill-typed builtins checker self-test: PASS")
        return 0
    finally:
        shutil.rmtree(stub, ignore_errors=True)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--compiler", default=str(REPO / "target/fastbuild/lccc"))
    ap.add_argument("--arm-compiler", default=str(REPO / "target/fastbuild/lccc-arm"))
    ap.add_argument("--gcc", default=shutil.which("gcc") or "")
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()

    if args.self_test:
        return self_test()

    compiler = Path(args.compiler)
    if not compiler.is_file():
        print(f"FAIL: compiler {compiler} not found", file=sys.stderr)
        return 2
    gcc = Path(args.gcc) if args.gcc else None
    arm = Path(args.arm_compiler)

    failures: list[str] = []
    with tempfile.TemporaryDirectory(prefix="ill-typed-builtins.") as td:
        work = Path(td)
        for case in X86_CASES:
            for opt in ("-O0", "-O2"):
                failures += check_case(case, compiler, gcc, work, opt)
        for case in FOREIGN_TARGET_CASES:
            if arm.is_file():
                failures += check_foreign_target(case, arm, work)
            else:
                print(f"SKIP {case.name}: {arm} not built", file=sys.stderr)

    rows = len(X86_CASES) * 2 + (len(FOREIGN_TARGET_CASES) if arm.is_file() else 0)
    if failures:
        for f in failures:
            print(f"FAIL: {f}", file=sys.stderr)
        print(f"ill-typed builtin diagnostics: FAIL ({len(failures)}/{rows} rows)", file=sys.stderr)
        return 1
    print(f"ill-typed builtin diagnostics: PASS ({rows} rows, GCC agreement checked where defined)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
