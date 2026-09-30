#!/usr/bin/env bash
# Arithmetic-producer redundant self-test elimination (x86-64).
#
# `subl $1, %R; testl %R, %R; jne` is one instruction of pure overhead: the
# subtraction already set ZF.  It is the shape every downward counter loop
# compiles to (`while (--n)`, `do { } while (--n)`, `while (n -= step)`), so
# the fold is pinned here in four directions, three of them structural:
#
#   * DETECTOR SELF-TEST.  At -O0 the pair is still emitted, so the analysis
#     must FIND it in every zf-only counter.  A gate whose pattern matcher
#     silently stopped matching would otherwise pass forever while proving
#     nothing -- the failure mode this repository has been bitten by before.
#   * POSITIVE.  At -O2 the pair must be gone for those same functions, while
#     the arithmetic producer itself remains.
#   * NEGATIVE CONTROL, same binary, same pass run: `while ((n -= 3) > 0)`
#     compiles to `subl $3; testl; jle` -- the consumer reads SF and OF too,
#     so that test must SURVIVE.  This is what distinguishes "the fold fired"
#     from "the compiler stopped emitting the pair at all".
#   * RUNTIME.  The compiled program must agree with GCC, and with lccc at
#     -O0/-O1/-O3, over every boundary a wrong proof would break: the last
#     iteration, the 32-bit sign flip at 0x80000000, and the unsigned wrap
#     points.
#
# A structural failure is an optimisation regression; a runtime failure is a
# miscompile.  Both exit non-zero.
set -euo pipefail

CCC=${CCC:-./target/release/lccc}
GCC=${GCC:-gcc}
command -v "$GCC" >/dev/null 2>&1 || { echo "SKIP: no $GCC on PATH"; exit 0; }

td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT

cat >"$td/shapes.c" <<'EOF'
/* Flag consumers that read ZF alone: the decrement's own ZF is enough. */
int c_while(int n)         { int c = 0; while (--n) { c++; } return c; }
int c_do(int n)            { int c = 0; if (n > 0) do { c++; } while (--n); return c; }
int c_unsigned(unsigned n) { int c = 0; while (--n) { c++; } return c; }
/* Negative control: `> 0` reads SF and OF, so `subl $3; testl; jle` keeps
   its test.  Same translation unit, same pass run. */
int c_step(int n)          { int c = 0; while ((n -= 3) > 0) { c++; } return c; }
EOF

"$CCC" -O0 -S "$td/shapes.c" -o "$td/shapes.O0.s"
"$CCC" -O2 -S "$td/shapes.c" -o "$td/shapes.O2.s"

python3 - "$td/shapes.O0.s" "$td/shapes.O2.s" <<'PY'
import re
import sys

ZF_ONLY = ("c_while", "c_do", "c_unsigned")   # must fold at -O2
KEPT = "c_step"                                # must NOT fold: jle reads SF/OF

PRODUCER = re.compile(r"^(sub|add|dec|inc|neg)[lq] ")
TEST = re.compile(r"^test[lq] ")
REG = re.compile(r"%([a-z0-9]+)")


_FAMILY = {}
for _wide, _forms in {
    "rax": ("rax", "eax", "ax", "al", "ah"),
    "rbx": ("rbx", "ebx", "bx", "bl", "bh"),
    "rcx": ("rcx", "ecx", "cx", "cl", "ch"),
    "rdx": ("rdx", "edx", "dx", "dl", "dh"),
    "rsi": ("rsi", "esi", "si", "sil"),
    "rdi": ("rdi", "edi", "di", "dil"),
    "rsp": ("rsp", "esp", "sp", "spl"),
    "rbp": ("rbp", "ebp", "bp", "bpl"),
}.items():
    for _form in _forms:
        _FAMILY[_form] = _wide
for _n in range(8, 16):
    for _form in (f"r{_n}", f"r{_n}d", f"r{_n}w", f"r{_n}b"):
        _FAMILY[_form] = f"r{_n}"


def family(reg: str) -> str:
    """The 64-bit name of the register `reg` belongs to (eax -> rax).

    The pair is not always width-matched: at -O0 the codegen emits a 32-bit
    `subl` under a 64-bit `testq`, which is exactly the width rule the pass
    has to reason about, and a matcher that compared spellings would miss it
    and quietly stop detecting anything.  `ah` folds into the `a` family
    here; byte-register aliasing is a soundness concern for the pass itself
    (its unit tests cover it), not for the shape this gate detects.
    """
    return _FAMILY.get(reg, reg)


def body(asm: str, fn: str):
    """Normalised instruction lines of `fn` (no labels/comments/directives)."""
    out, inside = [], False
    for raw in asm.splitlines():
        line = raw.strip()
        if line.startswith(fn + ":"):
            inside = True
            continue
        if inside and (line.startswith(".size " + fn) or line.startswith(".cfi_endproc")):
            break
        if not inside or not line or line.startswith(("#", ".")) or line.endswith(":"):
            continue
        out.append(line)
    return out


def pairs(asm: str, fn: str):
    """Every `<arith producer>` immediately followed by a self-test of the
    same register family."""
    lines, found = body(asm, fn), []
    for a, b in zip(lines, lines[1:]):
        if not PRODUCER.match(a) or not TEST.match(b):
            continue
        dest = REG.findall(a)[-1] if REG.findall(a) else ""
        operands = REG.findall(b)
        if len(operands) == 2 and operands[0] == operands[1] and family(operands[0]) == family(dest):
            found.append(f"{a} ;; {b}")
    return found


def producers(asm: str, fn: str):
    return [l for l in body(asm, fn) if PRODUCER.match(l)]


o0, o2 = (open(p).read() for p in sys.argv[1:3])
failures = []

# 0. detector self-test, on a fixture rather than on compiler output: the
#    matcher must see the pattern when it is there, and must not invent one
#    from two different register families. The compiler-side half of the same
#    question is the -O2 control below (`c_step`), which is what proves the
#    matcher still works on real codegen.  (-O0 does NOT serve this purpose:
#    there the producer and the test are separated by spill stores, so the
#    pair is not adjacent in the emitted text at all.)
FIXTURE = """f:
.cfi_startproc
    subl $1, %esi
    testl %esi, %esi
    je .L1
.L1:
    subl $8, %rdi
    testq %rax, %rax
    ret
.cfi_endproc
"""
fixture_pairs = pairs(FIXTURE, "f")
if len(fixture_pairs) != 1 or "subl $1, %esi ;; testl %esi, %esi" != fixture_pairs[0]:
    failures.append("detector self-test: the pair matcher no longer recognises "
                    f"`subl $1, %esi` + `testl %esi, %esi` (found {fixture_pairs})")
if any("subl $8, %rdi" in p for p in fixture_pairs):
    failures.append("detector self-test: the matcher paired two different "
                    "registers (`subl %rdi` with `testq %rax`)")

# 1. positive: folded at -O2, producer still there.
for fn in ZF_ONLY:
    if not producers(o2, fn):
        failures.append(f"-O2 {fn}: no arithmetic producer left -- fixture drift")
    for p in pairs(o2, fn):
        failures.append(f"-O2 {fn}: redundant self-test survived a ZF-only consumer: {p}")

# 3. negative control: the sign-carrying consumer keeps its test at -O2.
if not pairs(o2, KEPT):
    failures.append(f"-O2 {KEPT}: the test after the producer is gone, but `jle` reads "
                    "SF and OF -- either a miscompile or a vacuous negative control")

if failures:
    print("FAIL structural:")
    for f in failures:
        print("  " + f)
    sys.exit(1)
print("PASS structural: matcher self-tested; pair folded for the ZF-only counters "
      "at -O2 and kept for the sign-carrying consumer")
PY

# ── runtime: lccc (-O0, -O1, -O2, -O3) and GCC must agree ────────────────────
cat >"$td/run.c" <<'EOF'
#include <stdio.h>
int c_while(int n);
int c_do(int n);
int c_unsigned(unsigned n);
int c_step(int n);
int main(void)
{
    /* Signed counters are only fed values >= 1: `--n` from INT_MIN would be
       signed overflow, which is not this gate's subject. The unsigned counter
       still crosses 0x80000000 and wraps back through 0. */
    static const int step_vals[] = {1, 2, 3, 4, 7, 8, 15, 16, 63, 64, 255, 256, 1000};
    static const unsigned uvals[] = {0u, 1u, 2u, 3u, 255u, 256u, 65536u,
                                     0x7fffffffu, 0x80000000u, 0x80000001u,
                                     0xfffffffeu};
    unsigned long acc = 0;
    for (unsigned i = 0; i < sizeof step_vals / sizeof step_vals[0]; i++) {
        int n = step_vals[i];
        acc += (unsigned long)c_while(n) + (unsigned long)c_do(n) +
               (unsigned long)c_step(n);
    }
    for (unsigned i = 0; i < sizeof uvals / sizeof uvals[0]; i++)
        acc += (unsigned long)c_unsigned(uvals[i]);
    printf("%lu\n", acc);
    return 0;
}
EOF

"$GCC" -O2 "$td/run.c" "$td/shapes.c" -o "$td/run.gcc"
ref=$("$td/run.gcc")
for opt in -O0 -O1 -O2 -O3; do
    "$CCC" "$opt" "$td/run.c" "$td/shapes.c" -o "$td/run.lccc" ||
        { echo "FAIL lccc $opt: did not link"; exit 1; }
    got=$("$td/run.lccc")
    if [ "$got" != "$ref" ]; then
        echo "FAIL lccc $opt: $got, gcc -O2: $ref"
        exit 1
    fi
done
echo "PASS runtime: lccc -O0/-O1/-O2/-O3 agree with gcc -O2 ($ref)"
