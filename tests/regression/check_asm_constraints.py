#!/usr/bin/env python3
"""GP-17: inline-asm constraint-direction validation against GCC 16.2.

Rejected shapes must fail in both compilers with GCC's exact message;
accepted shapes must compile cleanly in both. Every row is a measured
gcc-16.2 behaviour (2026-10-28).
"""
from __future__ import annotations

import argparse
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

# label, asm statement, expected: None (both accept) or gcc's message
ROWS = (
    ("input-equals", '__asm__("addl $1, %0" : "=r"(o) : "=r"(x));',
     "input operand constraint contains '='"),
    ("input-plus", '__asm__("addl %1, %0" : "=r"(o) : "+r"(x));',
     "input operand constraint contains '+'"),
    ("input-earlyclobber", '__asm__("addl $1, %0" : "=r"(o) : "=&r"(x));',
     "input operand constraint contains '='"),
    ("input-alt-equals", '__asm__("addl %1, %0" : "=r"(o) : "r,=r"(x));',
     "input operand constraint contains '='"),
    ("output-no-modifier", '__asm__("nop" : "r"(o) : "r"(x));',
     "output operand constraint lacks '='"),
    ("output-misplaced-equals", '__asm__("addl %1, %0" : "=r,=r"(o) : "r"(x));',
     "operand constraint contains incorrectly positioned '+' or '='"),
    ("valid-input-register", '__asm__("movl %1, %0" : "=r"(o) : "r"(x));', None),
    ("valid-input-memory", '__asm__("movl %1, %0" : "=r"(o) : "m"(x));', None),
    ("valid-input-immediate", '__asm__("movl %1, %0" : "=r"(o) : "i"(2));', None),
    # NOTE: GCC's `'%' constraint used with last operand` position rule and
    # the alternative-count rule (`differ in number of alternatives`) are
    # measured but deliberately not yet implemented; they stay GP-17's
    # documented residual instead of a half-built check.
    ("valid-output-alternatives", '__asm__("movl %1, %0" : "=rm"(o) : "r"(x));', None),
    ("valid-output-plus", '__asm__("addl %1, %0" : "+r"(o) : "r"(x));', None),
    ("valid-output-earlyclobber", '__asm__("movl %1, %0" : "=&r"(o) : "r"(x));', None),
)


def compile_one(compiler: list[str], source: Path) -> tuple[int, str]:
    proc = subprocess.run(
        [*compiler, "-c", str(source), "-o", str(source.with_suffix(".o"))],
        cwd=ROOT,
        text=True,
        capture_output=True,
        timeout=30,
    )
    return proc.returncode, proc.stderr


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--lccc", default="target/fastbuild/lccc")
    parser.add_argument("--gcc", default="gcc-16")
    args = parser.parse_args()
    lccc = [args.lccc]
    gcc = [args.gcc]

    failures: list[str] = []
    with tempfile.TemporaryDirectory(prefix="lccc-asm-constraints-") as tmp:
        for label, statement, expected in ROWS:
            source = Path(tmp) / f"{label}.c"
            source.write_text(
                f"int probe(int x) {{ int o; {statement} return o; }}\n",
                encoding="utf-8",
            )
            g_rc, g_err = compile_one(gcc, source)
            l_rc, l_err = compile_one(lccc, source)
            if expected is None:
                if g_rc or l_rc:
                    failures.append(
                        f"{label}: both must accept (gcc rc={g_rc} lccc rc={l_rc}) "
                        f"gcc={g_err.splitlines()[:1]} lccc={l_err.splitlines()[:1]}"
                    )
                    continue
                print(f"asm constraints {label}: PASS (accepted)")
                continue
            g_hit = expected in g_err and g_rc != 0
            l_hit = expected in l_err and l_rc != 0
            if not g_hit:
                failures.append(f"{label}: GCC no longer errors `{expected}`: {g_err[:200]!r}")
            elif not l_hit:
                failures.append(
                    f"{label}: LCCC missing `{expected}` (rc={l_rc}): {l_err[:200]!r}"
                )
            else:
                print(f"asm constraints {label}: PASS (rejected, exact GCC message)")

    if failures:
        for line in failures:
            print(f"ERROR: {line}", file=sys.stderr)
        return 1
    print(f"asm constraints: PASS ({len(ROWS)} rows, GCC 16.2.0)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
