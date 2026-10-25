#!/usr/bin/env python3
"""Debug knobs are neutral: they may explain, never change code.

WHY THIS GATE EXISTS
====================
`docs/optimization-passes.md` lists the diagnostic knobs added with the S07-S08
work (`CCC_DEBUG_VOLRED`, `CCC_DEBUG_STATIC_INIT`, `CCC_DEBUG_TYPES_COMPATIBLE`)
and asserts two things about each of them:

  1. setting it cannot change emitted code -- a benchmark or kernel build run
     with a trace knob exported by accident must produce the same object; and
  2. it is still compiled in -- a knob whose `std::env::var_os` call was
     optimised away, renamed, or deleted is documentation of nothing.

Assertion (1) is a codegen differential on a fixture that reaches all three
decision sites (a `volatile_reduce` candidate loop, a static initializer whose
member holds an address constant, and a `__builtin_types_compatible_p` query),
at every optimization level the compiler exposes.  Assertion (2) is a `strings`
probe of the compiler binary, which fails loudly if a knob stops being read.

A knob that trips (1) is a miscompile-class hazard, not a logging nit: the
compiler would then have two behaviours for one input that differ only in an
environment variable nothing in the build records.
"""

import os
import subprocess
import sys
import tempfile

KNOBS = [
    "CCC_DEBUG_VOLRED",
    "CCC_DEBUG_STATIC_INIT",
    "CCC_DEBUG_TYPES_COMPATIBLE",
]

LEVELS = ["-O0", "-O1", "-O2", "-O3"]

FIXTURE = r"""
/* --- static initializer: a pointer-sized member holding an address constant
   (C11 6.6p9; Linux 6.18 keeps `(unsigned long)&model` in `driver_data`). */
extern int model;
struct cpu_id { unsigned short vendor; unsigned short family; unsigned long data; };
struct cpu_id ids[] = {
    { 0x0000, 0x0000, (unsigned long)&model },
    { 0x0001, 0x0002, (unsigned long)&model },
};

/* --- types_compatible_p: the predicate has to answer about the type, and the
   answer changes the layout of this object, so a knob that perturbed it would
   be visible in the emitted section sizes. */
struct probe {
    char a[__builtin_types_compatible_p(int, const int) ? 8 : 3];
    char b[__builtin_types_compatible_p(int *, long *) ? 8 : 5];
};
struct probe probe_obj;

/* --- volatile_reduce: a volatile read feeding several independent stores is
   the interleave candidate the pass budgets lanes for. */
void lanes(int *restrict out, volatile int *src, int n)
{
    int i;
    for (i = 0; i + 3 < n; i += 4) {
        int v0 = src[0], v1 = src[1], v2 = src[2], v3 = src[3];
        out[i + 0] = v0 + 1;
        out[i + 1] = v1 + 2;
        out[i + 2] = v2 + 3;
        out[i + 3] = v3 + 4;
    }
}

/* --- a compound literal as a designated element (the third evaluator path). */
struct node { int *p; long n; };
extern int target;
struct node nodes[2] = { [1] = (struct node){ &target, 3 } };
"""


def run(cmd, env=None, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, env=env, **kw)


def main():
    ccc = os.environ.get("CCC", "target/fastbuild/lccc")
    if not os.path.exists(ccc):
        print(f"SKIP: {ccc} not built", file=sys.stderr)
        return 0

    failures = []

    # (2) every knob is still read by the binary.
    blob = subprocess.run(["strings", "-a", ccc], capture_output=True).stdout
    for knob in KNOBS:
        if knob.encode() not in blob:
            failures.append(f"{knob}: not present in {ccc} (knob is dead)")

    tmp = tempfile.mkdtemp(prefix="debugknobs.")
    src = os.path.join(tmp, "fixture.c")
    with open(src, "w") as f:
        f.write(FIXTURE)

    base_env = dict(os.environ)
    for knob in KNOBS:
        base_env.pop(knob, None)

    for level in LEVELS:
        default = None
        for knob in [None] + KNOBS:
            env = dict(base_env)
            out = os.path.join(tmp, f"{level.strip('-')}.{knob or 'default'}.s")
            if knob:
                env[knob] = "1"
            proc = run([ccc, "-S", level, "-fno-pic", "-o", out, src], env=env)
            if proc.returncode != 0:
                failures.append(
                    f"{level} {knob or 'default'}: compile failed\n{proc.stderr.strip()}"
                )
                break
            text = open(out, "rb").read()
            if knob is None:
                default = text
            elif default is not None and text != default:
                failures.append(
                    f"{level} {knob}=1 changed emitted code (debug knobs must be neutral)"
                )
        # the fixture must not be a no-op: the default build has to define the
        # three decision sites, otherwise the differential above proves nothing.
        if default is not None:
            for sym in (b"lanes:", b"ids:", b"nodes:", b"probe_obj:"):
                if sym not in default:
                    failures.append(f"{level}: fixture lost {sym.decode()} (differential is vacuous)")

    if failures:
        print("debug-knob neutrality: FAILED")
        for f in failures:
            print("  " + f)
        return 1
    print(
        f"debug-knob neutrality: PASSED ({len(KNOBS)} knobs x {len(LEVELS)} levels, "
        "identical objects, fixture reaches every decision site)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
