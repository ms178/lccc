#!/usr/bin/env python3
"""`__builtin_types_compatible_p` — type identity, not type spelling.

WHY THIS GATE EXISTS
====================
Two types are compatible when C says they are the same type after removing
top-level qualification (C11 6.2.7).  The predicate therefore has to answer
about the *type*, and a compiler whose type builder keeps a spelling detail
(parameter names, a typedef's name, a `typeof`'s origin) in the type answers
"incompatible" for identical types.  That is not a cosmetic bug: Linux 6.18
asserts its own invariants with it —

    arch/x86/kernel/irq.c:  static_assert(__builtin_types_compatible_p(
                                __typeof__(&func), void *(void *)));

— so a wrong answer is a kernel build failure, and a *right* answer for the
wrong reason hides a type-model defect.  This gate pins the answers against
GCC 16.2 for the shapes the kernel and the C standard actually use: function
type identity (parameter NAMES are not part of the type), bare function types,
function-pointer identity through typedef/member/`typeof`/cast, address spaces
per pointer level, arrays vs pointers, and record tags.

HOW A ROW IS CHECKED
====================
Each row is `<prefix declarations> + int main` printing
`__builtin_types_compatible_p(<left>, <right>)`, compiled and RUN by both
compilers; the printed answers must be equal to each other and to the row's
`expect`.  Compiling and running (rather than reading a diagnostic) is what
makes an answer that is right-by-accident visible: the answer has to survive
lowering, not just sema.

`known_divergences.tsv` documents every place where the answer is knowingly
NOT the oracle's.  It is checked in BOTH directions: the row must still
diverge exactly as recorded (lccc's answer and the oracle's), because a
divergence that quietly starts passing means the row is stale and the
documentation lies — that fails the gate too.

USAGE
    GCC=gcc-16 CCC=target/fastbuild/lccc \
        tests/regression/check_types_compatible_p.py [--verbose]

EXIT STATUS
    0  every case row matches the oracle, every divergence still diverges
    1  a mismatch, or a divergence went stale
    2  the oracle or the compiler is missing or is not the pinned version
"""

import argparse
import os
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).with_name("types_compatible_p")

#: Spellings a row may reference instead of repeating a long declarator.
#: `CC_*` names are substituted into the row's text before it is compiled.
MACROS = {
    # `void (*)(int)` reached through a struct member, a typeof and a cast.
    "CC_TYPEOF_MEMBER": "typeof(((struct s *)0)->f)",
    # A typedef'd struct type reached through a member access.
    "CC_TYPEOF_F": "typeof(f)",
    "CC_TYPEOF_P": "typeof(p)",
}

PRELUDE = """#include <stdio.h>
"""


def run(cmd):
    return subprocess.run(cmd, capture_output=True, text=True)


def require_version(cc, pinned):
    out = run([cc, "--version"])
    if out.returncode != 0:
        raise SystemExit(f"gate: {cc}: not usable ({out.stderr.strip()[:80]})")
    line = out.stdout.splitlines()[0]
    if pinned not in line:
        raise SystemExit(f"gate: {cc}: expected {pinned}, got {line}")


def parse_rows(path, width):
    rows = []
    for lineno, raw in enumerate(path.read_text().splitlines(), 1):
        if not raw.strip() or raw.lstrip().startswith("#"):
            continue
        fields = raw.split("\t")
        if lineno == 1 and fields[0] == "id":
            continue
        if len(fields) != width:
            raise SystemExit(f"{path}:{lineno}: expected {width} tab fields, got {len(fields)}")
        rows.append(fields)
    return rows


def expand(text):
    for name, spelling in MACROS.items():
        text = text.replace(name, spelling)
    return text


def answer(cc, tmp, rid, prefix, left, right, extra_decls="", check_sema_expect=None):
    """Compile and run one row; return the printed answer or raise."""
    src = tmp / f"{rid}.c"
    decls = "" if prefix.strip() == "-" else expand(prefix) + "\n"
    body = PRELUDE + expand(extra_decls) + decls
    if decls:
        body += "\n"
    exp_left = expand(left)
    exp_right = expand(right)
    static_assert_line = (
        f"    _Static_assert(__builtin_types_compatible_p({exp_left}, {exp_right}) "
        f"== {check_sema_expect}, \"sema const_eval mismatch\");\n"
        if check_sema_expect is not None
        else ""
    )
    body += (
        "int main(void)\n{\n"
        + static_assert_line
        + f"    int sema_arr[__builtin_types_compatible_p({exp_left}, {exp_right}) ? 2 : 1];\n"
        "    int sema_ans = (int)(sizeof(sema_arr) / sizeof(int)) - 1;\n"
        f"    int rt_ans = __builtin_types_compatible_p({exp_left}, {exp_right});\n"
        "    if (sema_ans != rt_ans) return 3;\n"
        "    printf(\"%d\\n\", rt_ans);\n"
        "    return 0;\n}\n"
    )
    src.write_text(body)
    exe = tmp / f"{rid}.bin"
    build = run([cc, "-std=gnu11", "-w", str(src), "-o", str(exe)])
    if build.returncode != 0:
        return ("BUILD-FAILED", build.stderr.strip().splitlines()[:2])
    out = run([str(exe)])
    if out.returncode != 0:
        return ("RUN-FAILED", out.stderr.strip().splitlines()[:2])
    return (out.stdout.strip(), None)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--compiler", default=os.environ.get("CCC", str(ROOT / "target/fastbuild/lccc")))
    ap.add_argument("--gcc", default=os.environ.get("GCC", "gcc-16"))
    ap.add_argument("--verbose", action="store_true")
    args = ap.parse_args()

    if not Path(args.compiler).exists():
        raise SystemExit(f"gate: compiler not found: {args.compiler}")
    require_version(args.gcc, "16.2")

    cases = parse_rows(HERE / "cases.tsv", 5)
    divergences = parse_rows(HERE / "known_divergences.tsv", 9)
    failures = []
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)

        for rid, expect, prefix, left, right in cases:
            got = {}
            for name, cc in (("oracle", args.gcc), ("lccc", args.compiler)):
                value, err = answer(
                    cc, tmp, f"{rid}.{name}", prefix, left, right, check_sema_expect=expect
                )
                if err is not None:
                    failures.append(f"{rid}: {name} {value}: {err}")
                    got[name] = None
                else:
                    got[name] = value
            if got.get("oracle") is None or got.get("lccc") is None:
                continue
            if got["oracle"] != expect:
                raise SystemExit(
                    f"gate: row {rid}: cases.tsv says {expect}, GCC 16 says {got['oracle']} — fix the row"
                )
            if got["lccc"] != got["oracle"]:
                failures.append(
                    f"{rid}: lccc says {got['lccc']}, GCC says {got['oracle']} "
                    f"for `{left}` vs `{right}`"
                )
            elif args.verbose:
                print(f"  {rid}: {got['lccc']} (both)")

        for rid, prefix, left, right, want_lccc, want_oracle, _cause, _impact, _fix in divergences:
            for name, cc, want in (("oracle", args.gcc, want_oracle), ("lccc", args.compiler, want_lccc)):
                value, err = answer(cc, tmp, f"{rid}.{name}", prefix, left, right)
                if err is not None:
                    failures.append(f"{rid}: {name} {value}: {err}")
                    continue
                if value != want:
                    failures.append(
                        f"{rid}: {name} now answers {value}, known_divergences.tsv records {want} "
                        "— the row is stale: fix it or delete it"
                    )

    if failures:
        print("check_types_compatible_p: FAILED", file=sys.stderr)
        for f in failures:
            print(f"  {f}", file=sys.stderr)
        return 1
    print(
        f"check_types_compatible_p: PASSED ({len(cases)} rows vs GCC 16; "
        f"{len(divergences)} tracked divergences still open)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
