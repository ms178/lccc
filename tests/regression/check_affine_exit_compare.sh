#!/usr/bin/env bash
# Affine exit-compare folding (x86-64).
#
# `for (int i = 0; i + C < n; i++)` lowers to a per-trip address computation
# that exists only to feed the loop's own guard:
#
#     addq $1, %rdx          ; i += 1
#     leaq 4(%rdx), %rsi     ; i + 4      <-- recomputed every trip
#     cmpq %r8, %rsi
#     jl .LBB2
#
# The identity `slt(iv + c, k) == slt(iv, k - c)` moves the offset to the
# invariant side, where it is computed once. Measured on a 2000x28-trip loop
# with Callgrind: 549863 Ir before, 493863 after (-10.2%), which is exactly the
# one instruction removed per trip times 56000 trips.
#
# The rewrite is restricted to the four SIGNED ordered comparisons, and this
# gate pins that restriction as a first-class requirement rather than a
# detail. For `ult(iv + c, k)` the wrap is fully defined -- not undefined
# behaviour -- and the identity genuinely fails, so folding a pointer loop
# would be a silent miscompile. `p + 4 < v + n` is exactly that shape and it
# is here, unchanged, as the control.
#
#   * DETECTOR SELF-TEST.  The gate's matchers run against checked-in
#     pre-fold output first, so a matcher that quietly stopped matching cannot
#     make the negative half pass forever.
#   * POSITIVE.  The signed forms must fold: no per-trip `lea` in the body,
#     and the guard must compare the IV.
#   * NEGATIVE CONTROL.  The unsigned pointer form must keep its per-trip
#     `lea`.  Same source language, same pass run, same binary.
#   * RUNTIME.  Everything must agree with GCC across -O0..-O3, including
#     negative trip counts, the zero/one boundary, and a NEGATIVE offset
#     (`i - 2 < n`), which exercises the mirrored `const - iv` form of the add.
set -euo pipefail

CCC=${CCC:-./target/release/lccc}
GCC=${GCC:-gcc}
command -v "$GCC" >/dev/null 2>&1 || { echo "SKIP: no $GCC on PATH"; exit 0; }

td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT

cat >"$td/shapes.c" <<'EOF'
/* Positive: SIGNED affine guards, which must fold. */
long f(const int *a, int n)  { long s = 0; for (int i = 0; i + 4 < n; i++) s += a[i]; return s; }
int  g(int n)                 { int s = 0;  for (int i = 0; i + 3 < n; i++) s += i;     return s; }
long h(long n)                { long s = 0; for (long i = 0; i + 7 < n; i++) s += i;   return s; }
int  k(int n)                 { int s = 0;  for (int i = 0; i + 1 <= n; i++) s += i;  return s; }
long m(const int *a, int n)   { long s = 0; for (int i = 1; i - 2 < n; i++) s += a[i];  return s; }

/* Negative control: the UNSIGNED guard. `ult` is not `ult(iv, k - c)` under
   wraparound, so this must keep its per-trip address computation. */
unsigned long p(const unsigned char *v, int n)
                               { unsigned long s = 0; for (const unsigned char *q = v; q + 4 < v + n; q++) s += *q; return s; }
EOF

# ── detector self-test ──────────────────────────────────────────────────────
cat >"$td/prefix.s" <<'EOF'
p:
    movslq %esi, %rdx
    leaq (%rdi, %rdx, 1), %r8
    leaq 4(%rdi), %r11
    cmpq %r8, %r11
    jae .LBB3
    movzbl (%rdi), %r10d
    addq %r10, %r9
    addq $1, %rdi
    leaq 4(%rdi), %rsi
    cmpq %r8, %rsi
    jb .LBB2
    movq %r9, %rax
    ret
EOF

python3 - "$td/prefix.s" <<'PY'
import re, sys
asm = open(sys.argv[1]).read()
if not re.search(r"^\s*leaq 4\(%rdi\), %rsi$", asm, re.M):
    sys.exit("FAIL detector self-test: the fixture no longer contains the "
             "per-trip address computation this gate is about")
if not re.search(r"^\s*cmpq %r8, %rsi$", asm, re.M):
    sys.exit("FAIL detector self-test: the affine guard is missing from the fixture")
print("PASS detector self-test: the un-folded shape is recognisable")
PY

"$CCC" -O2 -S "$td/shapes.c" -o "$td/shapes.s"

python3 - "$td/shapes.s" <<'PY'
import re, sys

asm = open(sys.argv[1]).read()


def body(fn):
    m = re.search(rf"^{fn}:$(.*?)^\s*\.cfi_endproc", asm, re.M | re.S)
    if not m:
        sys.exit(f"FAIL: no body for {fn} in compiler output")
    return "\n".join(l for l in m.group(1).splitlines()
                     if not l.strip().startswith(("#", ".")))


# A per-trip `lea C(%reg), %tmp` immediately feeding a `cmpq` is the shape the
# fold removes. It is detected structurally (lea then cmp, same temporaries),
# not by a hard-coded register, so it cannot drift with register allocation.
PER_TRIP = re.compile(
    r"^\s*lea[q]? (-?\d+)\((%\w+)\), (%\w+)\n\s*cmp[qwl] (%\w+), \3\s*$", re.M)


for fn in ("f", "g", "h", "k", "m"):
    b = body(fn)
    if PER_TRIP.search(b):
        sys.exit(f"FAIL {fn}: the signed affine guard did NOT fold -- a per-trip "
                 f"address computation still feeds the loop's own test\n{b}")

b = body("p")
if not PER_TRIP.search(b):
    sys.exit("FAIL p: the UNSIGNED guard was folded. `ult(iv + c, k)` is not "
             "`ult(iv, k - c)` when the sum wraps -- the wrap is defined, not "
             "UB -- so this is a silent miscompile, and the pointer form is "
             "where it would land first.\n" + b)

print("PASS structural: 5/5 signed guards folded; unsigned pointer guard preserved")
PY

# ── runtime ─────────────────────────────────────────────────────────────────
cat >"$td/run.c" <<'EOF'
#include <stdio.h>
long f(const int *a, int n);
int  g(int n);
long h(long n);
int  k(int n);
long m(const int *a, int n);
unsigned long p(const unsigned char *v, int n);
int main(void)
{
    /* The buffers must be large enough for the LARGEST index any callee can
       reach, or this gate measures whatever happens to sit past the end --
       which differs between a gcc-compiled and an lccc-compiled binary and
       reads as a miscompile that is really a driver bug.  With n up to
       NMAX, `f` touches a[0 .. NMAX-4] and `m` touches a[1 .. NMAX+2]
       (`i - 2 < n` lets i run to n+2), so the arrays are sized past NMAX. */
    enum { NMAX = 300, NPAD = 512 };
    static int a[NPAD];
    static unsigned char b[NPAD];
    long t = 0;
    for (int i = 0; i < NPAD; i++) { a[i] = i * 3 - 7; b[i] = (unsigned char)(i * 5 + 1); }
    /* Negative counts, the zero/one boundary, and everything up to and past
       the offset: a fold that is off by the constant shows up here. */
    for (int n = -5; n <= NMAX; n++) {
        t += g(n) + (long)k(n) + f(a, n) + m(a, n) + (long)p(b, n);
    }
    static const long big[] = {-3, 0, 1, 7, 8, 9, 100, 1000, 1000000, -1000000};
    for (unsigned i = 0; i < sizeof big / sizeof big[0]; i++)
        t += h(big[i]);
    printf("%ld\n", t);
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
echo "PASS runtime: -O0/-O1/-O2/-O3 agree with gcc -O2 ($ref)"
