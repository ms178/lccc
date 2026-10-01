#!/usr/bin/env bash
# Compare/branch fusion across a loop LATCH (x86-64).
#
# `cmpl %esi, %r8d; setl %r10b; movzbl %r10b, %r10d; testb %r10b, %r10b; jne` is
# five instructions where the last two are free: the `cmpl` already set the flag
# the jump wants.  The fusion that collapses this is fail-closed -- it refuses
# unless the carrier register is provably dead at the jump -- and it refused on
# EVERY loop whose test branched backwards, because the gate asked the
# liveness question of the PRE-transform text:
#
#   * in the original text the carrier really is live at the latch, because the
#     zero-extending relay reads the byte again on the next iteration;
#   * the relay is one of the very instructions the fusion deletes, so asking
#     before the deletion is asking the wrong question;
#   * a byte-wide `setl` cannot kill the 64-bit family, so no family-granular
#     fixpoint can ever see that back edge as dead.  The gate was therefore
#     unreachable, not merely unlucky.
#
# So this gate pins the behaviour END TO END.  A unit test of the liveness
# predicate is not sufficient and is not what this file does: the failure mode
# that produced the `incapable` class in this same series was a harness that
# passed while the real pipeline still refused.  Measured on one input, the
# pre-fix compiler emits the five-instruction sequence above and the post-fix
# compiler emits `cmpl` + `jl`; this gate asserts the second and fails on the
# first.
#
#   * DETECTOR SELF-TEST.  The gate's own matcher is run against a checked-in
#     copy of the pre-fix output first.  A regex that silently stopped matching
#     would otherwise make the negative halves of this gate pass forever.
#   * POSITIVE.  With rotation enabled the latch must be fused and the
#     setcc/relay/test triple must be gone.
#   * NEGATIVE CONTROL, same binary, same pass run: `cnt` is the SAME rotated
#     loop as `d1`, but its boolean is consumed by an `add` rather than by a
#     test/jump, so the carrier is genuinely live and the setcc/relay pair MUST
#     survive.  This is what distinguishes "the latch got fused" from "the pass
#     started deleting boolean materialisation".
#   * RUNTIME.  The compiled programs must agree with GCC.
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
/* Latch shapes: the loop's exit test is a backward branch, so the boolean
   materialisation sits on the cycle and its carrier is read around the edge. */
int d1(int n)                 { int s = 0, i = 0; do { s += i; } while (++i < n); return s; }
unsigned long d2(const unsigned char *v, int n)
                              { unsigned long s = 0; int i = 0; do { s += v[0]; } while (++i < n); return s; }
int d3(int n)                 { int s = 1, i = 0; while (i < n) { s *= 2; i++; } return s; }
int d4(int n)                 { int s = 0, i = n; do { s += i; } while (--i > 0); return s; }

/* NEGATIVE CONTROL: the same rotated loop shape as d1, but the boolean feeds
   an `add` instead of a test/jump, so the carrier is observably live. */
int cnt(int n)                { int i = 0, c = 0; do { c += (i < n); } while (++i < n); return c; }
int acc(const int *a, int n)  { int s = 0; for (int i = 0; i < n; i++) s += (a[i] > 0); return s; }
EOF

# ── detector self-test ──────────────────────────────────────────────────────
# Verbatim pre-fix output for `f` on the loop this gate is about.  Without
# this, "no setcc remains" is satisfied just as well by a matcher that never
# matched anything.
cat >"$td/prefix.s" <<'EOF'
f:
    movzbl (%rdi), %eax
    movl %eax, %edi
    xorl %edx, %edx
    xorl %r8d, %r8d
    addq %rdi, %rdx
    addl $1, %r8d
    cmpl %esi, %r8d
    setl %r10b
    movzbl %r10b, %r10d
    testb %r10b, %r10b
jne .LBB1
    movq %rdx, %rax
    ret
EOF

python3 - "$td/prefix.s" <<'PY'
import re, sys
asm = open(sys.argv[1]).read()
if not re.search(r"^\s*set[a-z]+ %r10b$", asm, re.M):
    sys.exit("FAIL detector self-test: the self-test fixture no longer matches "
             "the un-fused shape this gate is about")
if not re.search(r"^\s*test[bwlq] %r10b, %r10b$", asm, re.M):
    sys.exit("FAIL detector self-test: relay/test pair not found in fixture")
print("PASS detector self-test: the un-fused shape is recognisable")
PY

CCC_LOOP_ROTATE=1 "$CCC" -O2 -S "$td/shapes.c" -o "$td/shapes.s"

python3 - "$td/shapes.s" <<'PY'
import re, sys

asm = open(sys.argv[1]).read()

# Every 8-bit and 32-bit register spelling, NOT just the %r8b..%r15b forms: the
# byte names %sil/%dil/%spl/%bpl do not end in `b` and the word names
# %esi/%edi do not end in `d`, so a matcher built on those suffixes calls a
# function "fused" with its carrier sitting right there in %sil.
LOW = (r"%(?:al|cl|dl|bl|spl|bpl|sil|dil"
       r"|r8b|r9b|r10b|r11b|r12b|r13b|r14b|r15b)")
WIDE = (r"%(?:eax|ecx|edx|ebx|esp|ebp|esi|edi"
        r"|r8d|r9d|r10d|r11d|r12d|r13d|r14d|r15d)")


def body(fn):
    m = re.search(rf"^{fn}:$(.*?)^\s*\.cfi_endproc", asm, re.M | re.S)
    if not m:
        sys.exit(f"FAIL: no body for {fn} in compiler output")
    return m.group(1)


def un_fused(text):
    """The setcc/relay/test shape the fusion exists to remove."""
    return (bool(re.search(rf"^\s*set[a-z]+ {LOW}$", text, re.M))
            and bool(re.search(rf"^\s*movzbl {LOW}, {WIDE}$", text, re.M))
            and bool(re.search(rf"^\s*test[bwlq] {LOW}, {LOW}$", text, re.M)))


fused = 0
for fn in ("d1", "d2", "d3", "d4"):
    b = body(fn)
    if un_fused(b):
        sys.exit(f"FAIL {fn}: the latch was NOT fused -- setcc/relay/test survived\n{b}")
    if not re.search(r"^\s+j[a-z]+ \.LBB", b, re.M):
        sys.exit(f"FAIL {fn}: no conditional branch at all\n{b}")
    fused += 1

# The liveness gate must still refuse when the carrier is live.
for fn in ("cnt", "acc"):
    b = body(fn)
    if not re.search(rf"^\s*set[a-z]+ {LOW}$", b, re.M) or \
       not re.search(rf"^\s*movzbl {LOW}, {WIDE}$", b, re.M):
        sys.exit(f"FAIL {fn}: the carrier is live (its boolean is consumed by "
                 f"arithmetic), so the setcc/relay MUST survive; deleting them "
                 f"means the liveness gate no longer gates anything\n{b}")

print(f"PASS structural: {fused}/4 latch shapes fused; live carriers preserved")
PY

# ── runtime: the rotated build must agree with GCC ───────────────────────────
cat >"$td/run.c" <<'EOF'
#include <stdio.h>
int d1(int n);
unsigned long d2(const unsigned char *v, int n);
int d3(int n);
int d4(int n);
int cnt(int n);
int acc(const int *a, int n);
int main(void)
{
    enum { NPAT = 6, NPAD = 64 };
    static const int vals[] = {1, 2, 3, 7, 8, 15, 16, 63, 64, 255, 256, 1000, 4096};
    /* Padded so that EVERY (j, m) pair formed below stays inside the object.
       `acc` reads a[0..m) and is handed &words[j], so a 6-element array with
       j=5, m=5 reads five elements past the end -- undefined behaviour whose
       result legitimately differs between two correct compilers, which would
       make this gate report a miscompile that is really a driver bug.  (Both
       GCC and lccc were observed faulting on exactly that before the padding,
       which is how it was found.) */
    static const unsigned char bytes[NPAD] = {0u, 1u, 2u, 3u, 250u, 255u};
    static const int words[NPAD] = {0, 1, -1, 7, -250, 255};
    unsigned long total = 0;
    for (unsigned i = 0; i < sizeof vals / sizeof vals[0]; i++) {
        int n = vals[i];
        total += (unsigned long)d1(n) + (unsigned long)d3(n)
             + (unsigned long)d4(n) + (unsigned long)cnt(n);
        for (unsigned j = 0; j < NPAT; j++) {
            int m = (int)(n % NPAT);
            if (m == 0) { m = 1; }
            total += d2(&bytes[j], m);
            total += (unsigned long)acc(&words[j], m);
        }
    }
    printf("%lu\n", total);
    return 0;
}
EOF

"$GCC" -O2 "$td/run.c" "$td/shapes.c" -o "$td/run.gcc"
ref=$("$td/run.gcc")

# The rotated build is the one that exercises the fusion; it must be runtime
# correct, which is the whole point of a fail-closed liveness gate.
CCC_LOOP_ROTATE=1 "$CCC" -O2 "$td/run.c" "$td/shapes.c" -o "$td/run.lccc" ||
    { echo "FAIL lccc -O2 CCC_LOOP_ROTATE=1: did not link"; exit 1; }
got=$("$td/run.lccc")
if [ "$got" != "$ref" ]; then
    echo "FAIL runtime (rotated): lccc $got, gcc -O2 $ref"
    exit 1
fi

# Rotation must not have changed behaviour at any other level either.
for opt in -O0 -O1 -O2 -O3; do
    "$CCC" "$opt" "$td/run.c" "$td/shapes.c" -o "$td/run.lccc" ||
        { echo "FAIL lccc $opt: did not link"; exit 1; }
    got=$("$td/run.lccc")
    if [ "$got" != "$ref" ]; then
        echo "FAIL lccc $opt: $got, gcc -O2: $ref"
        exit 1
    fi
done

echo "PASS runtime: rotated build and -O0/-O1/-O2/-O3 agree with gcc -O2 ($ref)"
