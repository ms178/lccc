#!/usr/bin/env bash
# Loop-carried accumulators must outlive the loop-local constants they share
# the callee-saved pool with.
#
# THE DEFECT THIS PINS
# --------------------
# Phase 1 of the register allocator gives callee-saved homes to values that
# are live across a call.  Two kinds of value compete for that pool: the ones
# that MUST have such a home (a call point is inside their live range, so a
# caller-saved register would be destroyed by the call — there is no
# alternative) and the hot-loop promotion, which is an optimisation.  They
# used to be admitted in one scan, ordered only by weighted use count, so an
# optional candidate could outrank a mandatory one and the mandatory one paid
# a stack slot that is reloaded on every use.
#
# The second defect in the same area: `mark_loop_spanning` recognised a
# recurrence only when the backedge source's own definition consumed the phi
# directly.  If-conversion puts a `Select` between them, which is exactly what
# an `if` inside a loop lowers to, so `phi -> Select -> BinOp -> phi` — the
# canonical loop-carried accumulation — was reported `recur=false` and lost
# every recurrence protection.
#
# Together, on a loop whose body calls a function and takes a rare branch that
# reads six loop-invariant constants (a 6-register callee-saved pool on
# x86-64), the four accumulators were spilled while the four most expensive
# constants kept registers: 5 stores + 6 reloads per iteration through
# `%rsp`, 124 ms against GCC's 79 ms on the 40M-trip reference kernel.
#
# WHAT IS CHECKED
# ---------------
#   * `CCC_TRACE_ALLOC=1` never reports demoting a value that SPANS a loop and
#     is READ inside it.  Those are the values written and read on every
#     iteration, so their slot round-trip sits on the loop's dependence chain;
#     the loop-invariant constants the same function reads on its rare path
#     are the ones that may go to the stack, because their reload happens only
#     where their block runs.  This is the mechanism, not an instruction count,
#     so the check does not depend on how the backend schedules the body.
#   * the answer still matches GCC: a register-allocation change may move a
#     value, never change what the program computes.
set -euo pipefail

CCC=$(realpath "${CCC:-./target/fastbuild/lccc}")
GCC=${GCC_BIN:-gcc}
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT
cd "$td"

cat > web.c <<'EOF'
#include <stdint.h>
#include <stdio.h>

/* Six callee-saved registers on x86-64; the six constants below are exactly
 * what the allocator used to hoard them with. */
static volatile uint64_t salt = UINT64_C(0x9e3779b97f4a7c15);
static volatile uint64_t cold[6] = { 11, 23, 47, 97, 193, 389 };

__attribute__((noinline)) static uint64_t helper(uint64_t x) {
    return (x ^ salt) * UINT64_C(0xd6e8feb86659fd93);
}

__attribute__((noinline)) static uint64_t exercise(unsigned n, uint64_t seed) {
    uint64_t p = cold[0], q = cold[1], r = cold[2];
    uint64_t s = cold[3], t = cold[4], u = cold[5];
    uint64_t a = seed, b = seed + 1, c = seed + 2, d = seed + 3;
    for (unsigned i = 0; i < n; ++i) {
        uint64_t h = helper(i);
        a += h;
        b ^= a + (b << 3);
        c += b ^ (c >> 5);
        d = (d << 7) | (d >> 57);
        d ^= c;
        if ((h & 255) == 0) {
            a ^= p; b += q; c ^= r; d += s;
            a += t; d ^= u;
        }
    }
    return a ^ b ^ c ^ d ^ p ^ q ^ r ^ s ^ t ^ u;
}

int main(void) {
    printf("%016llx\n", (unsigned long long)exercise(200000u, UINT64_C(0xffff000000000001)));
    return 0;
}
EOF

fail=0
note() { printf 'FAIL: %s\n' "$1" >&2; fail=1; }

CCC_TRACE_ALLOC=1 "$CCC" -O2 -std=gnu17 -S web.c -o web.s 2> trace.txt
"$CCC" -O2 -std=gnu17 -o web-lccc web.c
"$GCC" -O2 -std=gnu17 -o web-gcc web.c

# The function body only: `exercise` is static and `noinline`, so the
# assembler labels it plainly.  Block labels are `.LBB*` (leading dot); the
# next FUNCTION label ends the body.
awk '/^exercise:/{f=1; next} f && /^[a-zA-Z_][a-zA-Z_0-9]*:/ { exit } f' web.s > exercise.s
if [ ! -s exercise.s ]; then
    note "the compiler emitted no body for 'exercise'"
fi

# 1. No loop-spanning value that is read inside its loop may be demoted.
# One grep over one file: `-c` prints 0 and exits 1 when nothing matches, so
# the `|| true` here cannot hide a producer failure the way a pipeline could.
hot_demoted=$(grep -c 'spans_loop=true in_loop_use=true' trace.txt || true)
if [ "$hot_demoted" -ne 0 ]; then
    note "demoted $hot_demoted value(s) that are read inside the loop they span:"
    grep 'spans_loop=true in_loop_use=true' trace.txt | sed -n '1,4p' >&2
fi

# 2. The accumulators must hold callee-saved homes (their homes have to
#    survive the in-loop call).  x86-64 offers six.
saved=$(grep -oE '%(rbx|rbp|r12|r13|r14|r15)' exercise.s | sort -u | wc -l)
if [ "$saved" -lt 4 ]; then
    note "only $saved callee-saved register(s) are live (expected >= 4)"
fi

# 3. Same answer as GCC.
want=$("./web-gcc")
got=$("./web-lccc")
[ "$got" = "$want" ] || note "result differs from GCC ($got vs $want)"

if [ "$fail" -ne 0 ]; then
    exit 1
fi
printf 'hot-loop register homes: ok (%s callee-saved registers, no recurrence demoted)\n' "$saved"
