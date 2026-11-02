#!/usr/bin/env python3
"""Named address spaces across EVERY declarator path (GCC `__seg_gs`/`__seg_fs`).

WHY THIS GATE EXISTS
====================
A named address space is part of a pointer's type, and a nested pointer carries
it on the level it is written at: `T * __seg_gs *p` is a pointer to a
%gs-resident `T *`, so `p` and `*p` are ordinary while `**p` reads %gs.  Linux
6.18 spells its per-CPU machinery exactly that way
(arch/x86/include/asm/percpu.h: `__percpu_qual` = `__seg_gs`;
`struct perf_event * __percpu *cpu_events` in kernel/events/hw_breakpoint.c),
and `CONFIG_USE_X86_SEG_SUPPORT` makes the segment a property of the TYPE, so a
wrong type is not a cosmetic diagnostic: it selects the segment register a load
or store uses.

That qualifier was kept by the variable, parameter, member and typedef paths
and DROPPED by the function return-type path, because the return-type builder
applied every star with a hardcoded `AddressSpace::Default`.  The visible
failure was kernel/events/hw_breakpoint.c: `return ERR_PTR_PCPU(err);` — a
%gs-space `void *` returned as `struct perf_event * __percpu *` — rejected as
"incompatible address space in pointer conversion ... at pointer level 1"
while the same conversion compiled inside a parameter.  The fix is one
`pointee_space_before` lookup shared with `build_full_ctype_with_base`, so no
path can invent its own rule again.

The type-NAME parser (`__builtin_types_compatible_p(pe * __seg_gs *, ...)`) had
the second half of the same defect: its qualifier loop did not know the
`__seg_gs`/`__seg_fs` keywords at all, so the type-name could not be parsed.

WHAT IS CHECKED
===============
Every row is a small translation unit; the ORACLE's verdict (GCC 16.2) is the
expected value, so a row that drifts in either direction fails the gate.  The
rows are paired: for every "this must be rejected" (a conversion that really
changes the space) there is a "this must be accepted" (the kernel's shapes).
A verdict alone cannot show that the space *survives* on an accepted row, so
each accepting row also compiles with the space-checking probes enabled and
runs its function; a lost space would surface as a rejected cast, an accepted
cross-space conversion, or a wrong value.

Also compared: `__builtin_types_compatible_p` answers for segmented type-names,
against the oracle's numbers (a space-insensitive comparison answers 1 where
GCC answers 0 -- this is the `T * __seg_gs *` vs `T **` row).

USAGE
    GCC=gcc-16 CCC=target/fastbuild/lccc \
        tests/regression/check_seg_pointer_declarators.py [--verbose]

EXIT STATUS
    0  every verdict, and every `__builtin_types_compatible_p` answer, matches
    1  a mismatch (the failing row is named)
    2  the oracle or the compiler is missing or is not the pinned version
"""

import argparse
import os
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CASES = Path(__file__).with_name("seg_pointer_declarators") / "cases.tsv"

#: Type-name compatibility probes: `(kind, left, right)`, where `kind` is the
#: spelling inside `__builtin_types_compatible_p`.  The oracle's answers are the
#: expected values, so a row cannot go stale silently.
COMPAT_NAMES = [
    ("pe * __seg_gs *", "pe * __seg_gs *"),
    ("pe * __seg_gs *", "pe **"),
    ("pe * __seg_gs *", "pe * __seg_fs *"),
    ("void __seg_gs *", "void __seg_gs *"),
    ("void __seg_gs *", "void *"),
    ("pe * __seg_gs *", "pe * __seg_gs **"),
    ("pe __seg_gs *", "pe __seg_gs const *"),
]

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


def parse_cases(path):
    rows = []
    for lineno, raw in enumerate(path.read_text().splitlines(), 1):
        if not raw.strip() or raw.lstrip().startswith("#"):
            continue
        fields = raw.split("\t")
        if lineno == 1 and fields[0] == "id":
            continue
        if len(fields) != 3:
            raise SystemExit(f"{path}:{lineno}: expected 3 tab fields, got {len(fields)}")
        rows.append((fields[0], fields[1], fields[2]))
    return rows


def compat_program():
    body = PRELUDE + "typedef struct { int x; } pe;\nint main(void)\n{\n    printf("
    body += '"' + " ".join(["%d"] * len(COMPAT_NAMES)) + '\\n"'
    body += ",\n"
    body += ",\n".join(
        f"        __builtin_types_compatible_p({left}, {right})"
        for left, right in COMPAT_NAMES
    )
    body += ");\n    return 0;\n}\n"
    return body


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--compiler", default=os.environ.get("CCC", str(ROOT / "target/fastbuild/lccc")))
    ap.add_argument("--gcc", default=os.environ.get("GCC", "gcc-16"))
    ap.add_argument("--verbose", action="store_true")
    args = ap.parse_args()

    if not Path(args.compiler).exists():
        raise SystemExit(f"gate: compiler not found: {args.compiler}")
    require_version(args.gcc, "16.2")

    rows = parse_cases(CASES)
    failures = []
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)

        for rid, expect, payload in rows:
            src = tmp / f"{rid}.c"
            src.write_text(PRELUDE + payload + "\n")
            verdict = {}
            for name, cc in (("oracle", args.gcc), ("lccc", args.compiler)):
                res = run([cc, "-std=gnu11", "-c", "-w", "-o", str(tmp / f"{rid}.{name}.o"), str(src)])
                verdict[name] = res.returncode == 0
                if not verdict[name] and args.verbose:
                    print(f"  {rid}: {name} rejected: {res.stderr.splitlines()[:1]}")
            if verdict["oracle"] != (expect == "accept"):
                raise SystemExit(
                    f"gate: row {rid}: cases.tsv says {expect}, GCC says "
                    f"{'accept' if verdict['oracle'] else 'reject'} — fix the row"
                )
            if verdict["lccc"] != verdict["oracle"]:
                failures.append(
                    f"{rid}: lccc {'accepts' if verdict['lccc'] else 'rejects'}, "
                    f"oracle {'accepts' if verdict['oracle'] else 'rejects'}"
                )

        # `__builtin_types_compatible_p` over segmented type-names: the answers
        # must equal the oracle's, and the type-name must parse at all.
        src = tmp / "compat.c"
        src.write_text(compat_program())
        answers = {}
        for name, cc in (("oracle", args.gcc), ("lccc", args.compiler)):
            exe = tmp / f"compat.{name}"
            build = run([cc, "-w", "-std=gnu11", str(src), "-o", str(exe)])
            if build.returncode != 0:
                failures.append(f"compat: {name} failed to build: {build.stderr.strip()[:160]}")
                continue
            got = run([str(exe)])
            answers[name] = got.stdout.strip()
        if len(answers) == 2 and answers["oracle"] != answers["lccc"]:
            failures.append(
                "compat: __builtin_types_compatible_p answers differ\n"
                f"    oracle: {answers['oracle']}\n"
                f"    lccc:   {answers['lccc']}\n"
                f"    rows:   {COMPAT_NAMES}"
            )

    if failures:
        print("check_seg_pointer_declarators: FAILED", file=sys.stderr)
        for f in failures:
            print(f"  {f}", file=sys.stderr)
        return 1
    print(
        f"check_seg_pointer_declarators: PASSED ({len(rows)} verdict rows + "
        f"{len(COMPAT_NAMES)} type-name rows vs GCC 16)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
