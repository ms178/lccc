#!/usr/bin/env python3
"""LICM: speculative hoisting of loads from non-weak global objects.

LICM's must-execute rule keeps a load in its loop unless its block
dominates every loop block.  Before loop rotation even the body block of
`for (i = 0; i < n; i++) c += G;` fails that test, so LCCC re-read `G` on
every iteration, while GCC 16.2, Clang 23.1.4 and ICX 2026.2 load it once.
A load of a non-weak object symbol, entirely inside the object, cannot
fault (`passes::licm::speculation_safe_globals`), so the rule is waived for
it; whether the loop modifies the object is still the memory analysis's
question.

This gate pins both directions:

* positives -- the symbol is referenced only OUTSIDE the loop (before the
  loop's header label) for a plain global, a conditional-block load
  (`if (p[i]) c += G;`), a struct member at a constant offset, an array
  element at a constant index, and an `extern` declaration;
* refusals, asserted AS refusals -- the load stays inside the loop for an
  incomplete array (`extern int U[];`, unknown size), a load wider than its
  object (a 4-byte `may_alias` read of a 2-byte array; the runtime twin uses
  a 4-byte array so every build is well defined), a constant offset past the object, a thread-local, a volatile
  object and a global the loop stores to;
* runtime -- weak symbols left UNDEFINED (address 0) behind `if (&W)` in
  the loop, both `__attribute__((weak))` and `#pragma weak`: a speculated
  load would fault; and every positive/refusal function's value against
  gcc-16 (and with LICM disabled), at -O1 and -O2; the shapes are asserted
  at -O2 and -O3, the tiers that run LICM.

Usage: check_licm_global_speculation.py --lccc target/fastbuild/lccc [--gcc gcc-16]
"""

from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
import tempfile

SHAPES = r"""
int G, H;
struct S { int a, b, c; } S;
int A[8];
extern int E;
int A2[2];
_Alignas(4) char small[2];
typedef int __attribute__((may_alias)) alias_int;
extern int U[];
__thread int T;
volatile int V;

int plain(int n) { int c = 0; for (int i = 0; i < n; i++) c += G; return c; }
int cond(const int *p, int n) { int c = 0; for (int i = 0; i < n; i++) if (p[i]) c += H; return c; }
int member(const int *p, int n) { int c = 0; for (int i = 0; i < n; i++) if (p[i]) c += S.b; return c; }
int element(const int *p, int n) { int c = 0; for (int i = 0; i < n; i++) if (p[i]) c += A[3]; return c; }
int ext(const int *p, int n) { int c = 0; for (int i = 0; i < n; i++) if (p[i]) c += E; return c; }

int r_incomplete(const int *p, int n) { int c = 0; for (int i = 0; i < n; i++) if (p[i]) c += U[0]; return c; }
int r_wide(const int *p, int n) { int c = 0; for (int i = 0; i < n; i++) if (p[i]) c += *(alias_int *)small; return c; }
int r_past(const int *p, int n) { int c = 0; for (int i = 0; i < n; i++) if (p[i]) c += ((int *)A2)[3]; return c; }
int r_tls(const int *p, int n) { int c = 0; for (int i = 0; i < n; i++) if (p[i]) c += T; return c; }
int r_volatile(int n) { int c = 0; for (int i = 0; i < n; i++) c += V; return c; }
int r_stored(const int *p, int n) { int c = 0; for (int i = 0; i < n; i++) { c += G; if (p[i]) G = i; } return c; }
"""

POSITIVE = {
    "plain": "G",
    "cond": "H",
    "member": "S",
    "element": "A",
    "ext": "E",
}
REFUSAL = {
    "r_incomplete": "U",
    "r_wide": "small",
    "r_past": "A2",
    "r_tls": "T",
    "r_volatile": "V",
    "r_stored": "G",
}

RUNTIME = r"""
#include <stdio.h>
extern int W __attribute__((weak));
extern int PW;
#pragma weak PW
int G = 3, H = 5, E = 7, U[4] = {11, 0, 0, 0}, A2[4] = {1, 2, 3, 13};
struct S { int a, b, c; } S = {1, 17, 2};
int A[8] = {0, 0, 0, 19};
_Alignas(4) char small[4] = {1, 0, 0, 0};
typedef int __attribute__((may_alias)) alias_int;
__thread int T = 23;
volatile int V = 29;

__attribute__((noinline)) int weak_attr(int n) {
    int c = 0;
    for (int i = 0; i < n; i++) if (&W) c += W;
    return c;
}
__attribute__((noinline)) int weak_pragma(int n) {
    int c = 0;
    for (int i = 0; i < n; i++) { if (&PW) c += PW; c += i; }
    return c;
}
#define LOOP(name, body) __attribute__((noinline)) int name(const int *p, int n) \
    { int c = 0; for (int i = 0; i < n; i++) body; return c; }
__attribute__((noinline)) int plain(int n) { int c = 0; for (int i = 0; i < n; i++) c += G; return c; }
LOOP(cond, if (p[i]) c += H)
LOOP(member, if (p[i]) c += S.b)
LOOP(element, if (p[i]) c += A[3])
LOOP(ext, if (p[i]) c += E)
LOOP(r_incomplete, if (p[i]) c += U[0])
LOOP(r_wide, if (p[i]) c += *(alias_int *)small)
LOOP(r_past, if (p[i]) c += ((int *)A2)[3])
LOOP(r_tls, if (p[i]) c += T)
__attribute__((noinline)) int r_volatile(int n) { int c = 0; for (int i = 0; i < n; i++) c += V; return c; }
LOOP(r_stored, { c += G; if (p[i]) G = i; })

int main(void) {
    static const int p[9] = {1, 0, 1, 1, 0, 0, 1, 0, 1};
    long acc = weak_attr(5) * 1000003L + weak_pragma(6);
    acc = acc * 31 + plain(9) + cond(p, 9) + member(p, 9) + element(p, 9) + ext(p, 9);
    acc = acc * 31 + r_incomplete(p, 9) + r_wide(p, 9) + r_past(p, 9) + r_tls(p, 9);
    acc = acc * 31 + r_volatile(9) + r_stored(p, 9);
    printf("%ld\n", acc);
    return 0;
}
"""


def run(cmd: list[str], env: dict[str, str] | None = None) -> str:
    proc = subprocess.run(cmd, capture_output=True, text=True, env=env, timeout=120)
    if proc.returncode != 0:
        raise SystemExit(f"FAIL: {' '.join(cmd)} (rc {proc.returncode})\n{proc.stdout}{proc.stderr}")
    return proc.stdout


def function_lines(asm: str, name: str) -> list[str]:
    lines = asm.splitlines()
    try:
        start = lines.index(f"{name}:")
    except ValueError as exc:
        raise SystemExit(f"FAIL: no function {name} in the assembly") from exc
    body = []
    for line in lines[start + 1:]:
        if line.startswith(".size") or line.strip() == ".cfi_endproc":
            break
        body.append(line)
    return body


LABEL = re.compile(r"^(\.L[\w.]+):")
JUMP = re.compile(r"^\s*j\w+\s+(\.L[\w.]+)\s*$")


def loop_region(body: list[str], name: str) -> tuple[int, int]:
    """The (header, back-edge) line span of the function's loop: the first
    label targeted by a LATER jump, through the last such jump."""
    labels = {}
    for idx, line in enumerate(body):
        m = LABEL.match(line)
        if m:
            labels[m.group(1)] = idx
    best = None
    for idx, line in enumerate(body):
        m = JUMP.match(line)
        if m and m.group(1) in labels and labels[m.group(1)] < idx:
            head = labels[m.group(1)]
            if best is None or head < best[0]:
                best = (head, idx)
            elif head == best[0]:
                best = (head, max(best[1], idx))
    if best is None:
        raise SystemExit(f"FAIL: {name}: no loop (back edge) found:\n" + "\n".join(body))
    return best


def mentions(line: str, sym: str) -> bool:
    return re.search(rf"(?<![\w.]){re.escape(sym)}(?![\w])", line) is not None


def check_shapes(lccc: str, tmp: str, opt: str) -> None:
    src = os.path.join(tmp, "shapes.c")
    with open(src, "w") as f:
        f.write(SHAPES)
    asm = run([lccc, opt, "-S", src, "-o", "-"])
    for name, sym in {**POSITIVE, **REFUSAL}.items():
        body = function_lines(asm, name)
        head, back = loop_region(body, name)
        inside = [l for l in body[head:back + 1] if mentions(l, sym)]
        before = [l for l in body[:head] if mentions(l, sym)]
        if name in POSITIVE:
            if inside or not before:
                raise SystemExit(
                    f"FAIL: {opt} {name}: load of {sym} not hoisted out of the loop:\n"
                    + "\n".join(body))
        elif not inside:
            raise SystemExit(
                f"FAIL: {opt} {name}: load of {sym} left the loop (unsafe speculation):\n"
                + "\n".join(body))
        print(f"PASS: {opt} {name}: {sym} {'hoisted' if name in POSITIVE else 'kept in loop'}")


def check_runtime(lccc: str, gcc: str | None, tmp: str) -> None:
    src = os.path.join(tmp, "rt.c")
    with open(src, "w") as f:
        f.write(RUNTIME)
    results = {}
    for opt in ("-O1", "-O2"):
        for licm in ("on", "off"):
            exe = os.path.join(tmp, f"rt{opt}{licm}")
            env = dict(os.environ)
            if licm == "off":
                env["CCC_DISABLE_PASSES"] = "licm"
            run([lccc, opt, src, "-o", exe], env)
            results[f"lccc {opt} licm={licm}"] = run([exe]).strip()
    if gcc:
        exe = os.path.join(tmp, "rt_gcc")
        run([gcc, "-O2", src, "-o", exe])
        results["gcc-16 -O2"] = run([exe]).strip()
    values = set(results.values())
    if len(values) != 1:
        raise SystemExit("FAIL: runtime values differ:\n" + "\n".join(f"  {k}: {v}" for k, v in results.items()))
    print(f"PASS: runtime ({len(results)} builds agree, weak symbols undefined): {values.pop()}")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--lccc", required=True)
    ap.add_argument("--gcc", default=None)
    args = ap.parse_args()
    gcc = args.gcc
    if gcc:
        probe = subprocess.run([gcc, "--version"], capture_output=True, text=True)
        if probe.returncode != 0 or "16.2" not in probe.stdout:
            raise SystemExit(f"FAIL: --gcc {gcc} is not GCC 16.2")
    with tempfile.TemporaryDirectory(prefix="lccc-licm-spec.") as tmp:
        # LICM runs in the -O2+ pipeline; the -O1 tier is loop-free scalar
        # cleanup by design (src/passes/mod.rs), so shapes are asserted where
        # the pass exists. The runtime check below still covers -O1.
        for opt in ("-O2", "-O3"):
            check_shapes(args.lccc, tmp, opt)
        check_runtime(args.lccc, gcc, tmp)
    print("licm-global-speculation gate OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
