#!/usr/bin/env bash
# A call-free leaf must not pay for callee-saved registers.
#
# The allocator's callee-saved bank exists so a value can survive a call. A
# function that makes no call never needs that guarantee: a caller-saved home
# is exactly as safe, and a callee-saved one costs a `push`/`pop` pair plus a
# bigger frame. `RegAllocConfig::leaf_caller_saved_homes` (x86-64, on by
# default when the caller-saved pool is wide enough) therefore lets a leaf's
# hot loop values compete for the volatile pool first, with the Phase-2c
# overflow still taking callee-saved homes — the prologue saves exactly the
# registers the loop actually needs.
#
# That decision is invisible in ordinary testing: a leaf that homes three loop
# values in rbx/r12/re13 still computes the right answer, just slower, so no
# execution test can see the regression. This gate pins the SHAPE:
#
#   1. a call-free leaf keeping four loop values live references no
#      callee-saved register and emits no push/pop;
#   2. `CCC_NO_LEAF_CALLER_HOME=1` restores the callee-saved shape — the kill
#      switch still isolates the policy for A/B runs;
#   3. the same loop wrapped around a call DOES preserve its state in
#      callee-saved registers (the policy must not leak into non-leaves);
#   4. every arm prints gcc's own output, so a policy that changes behaviour
#      is a failure even when its shape is right.
#
# The shapes are read from `-S` output: the linked image's register choice can
# be changed by the assembler's relaxation, while the emitted text is exactly
# what the allocator decided.
#
# Usage: bash tests/regression/check_leaf_caller_homes.sh
#        CCC=/path/to/lccc GCC_ORACLE=/usr/bin/gcc bash tests/regression/check_leaf_caller_homes.sh
set -eu

CCC=${CCC:-target/fastbuild/lccc}
GCC_ORACLE=${GCC_ORACLE:-gcc}
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

if [ ! -x "$CCC" ]; then
    echo "FAIL: lccc not found at $CCC" >&2
    exit 1
fi
if ! command -v "$GCC_ORACLE" >/dev/null 2>&1; then
    echo "SKIP: no reference compiler '$GCC_ORACLE' on PATH" >&2
    exit 0
fi

work=$TMP/work
mkdir -p "$work"

# Four live loop values (a, b, c and the induction variable): more than the
# three scratch registers any single instruction sequence needs, and exactly
# the pressure the policy exists for.
cat >"$work/leaf.c" <<'EOF'
#include <stdio.h>

__attribute__((noinline)) unsigned long kernel(unsigned long n) {
    unsigned long a = 0x243f6a8885a308d3UL, b = 0x13198a2e03707344UL, c = 7;
    for (unsigned long i = 0; i < n; i++) {
        a += i;
        b ^= a;
        c += b >> 3;
    }
    return a + b + c;
}

int main(void) {
    unsigned long s = 0;
    for (unsigned long n = 1; n <= 997; n += 16) s = s * 31 + kernel(n);
    printf("%lu\n", s);
    return 0;
}
EOF

# The same loop around a call the compiler cannot summarize: the values must
# survive it, so at least one callee-saved register has to appear. The call
# goes through a volatile function pointer on purpose — a direct call to a
# local callee is what the interprocedural pass exists for (it proves the
# callee leaves the volatile pool alone, in which case the loop legitimately
# keeps its state there), and this arm is about the conservative case.
cat >"$work/caller.c" <<'EOF'
#include <stdio.h>

__attribute__((noinline)) unsigned long step(unsigned long x) { return x * 3 + 1; }

static unsigned long (*volatile step_ptr)(unsigned long) = step;

__attribute__((noinline)) unsigned long kernel(unsigned long n) {
    unsigned long a = 0x243f6a8885a308d3UL, b = 0x13198a2e03707344UL, c = 7;
    for (unsigned long i = 0; i < n; i++) {
        a += i;
        b ^= a;
        c += step_ptr(a >> 7);
    }
    return a + b + c;
}

int main(void) {
    unsigned long s = 0;
    for (unsigned long n = 1; n <= 97; n += 4) s = s * 31 + kernel(n);
    printf("%lu\n", s);
    return 0;
}
EOF

note() { printf '  %-8s %s\n' "$1" "$2"; }
fail=0
ok() { note ok "$1"; }
bad() {
    note FAIL "$1"
    fail=1
}

body_of() { awk -v fn="$2" '$0 ~ "^" fn ":" { inb = 1; next } inb && /^[[:space:]]*\.size[[:space:]]/ { exit } inb' "$1"; }
# Callee-saved GPRs a leaf must not touch when its pressure fits the volatile
# pool: rbx, r12-r15 (rbp is the frame pointer and excluded by -O2).
callee_refs() { body_of "$1" kernel | grep -Ec '%(rbx|r1[2-5])\b' || true; }
pushes() { body_of "$1" kernel | grep -Ec '^[[:space:]]+push|^[[:space:]]+pop' || true; }

run_both() {
    src=$1
    tag=$2
    "$GCC_ORACLE" -O2 -o "$work/$tag.ref" "$src" 2>/dev/null || bad "$tag: gcc oracle failed"
    "$GCC_ORACLE" -O2 "$src" -o "$work/$tag.ref" 2>/dev/null || true
    want=$("$work/$tag.ref")
    "$CCC" -O2 -S -o "$work/$tag.s" "$src" || bad "$tag: lccc -S failed"
    "$CCC" -O2 -o "$work/$tag.bin" "$src" || bad "$tag: lccc link failed"
    got=$("$work/$tag.bin") || bad "$tag: lccc binary did not run"
    [ "${got:-}" = "$want" ] && ok "$tag: prints gcc's output ($want)" \
        || bad "$tag: printed '${got:-<crash>}', gcc printed '$want'"
}

# ---- 1: a call-free leaf uses the volatile pool -------------------------
run_both "$work/leaf.c" leaf
callee=$(callee_refs "$work/leaf.s")
pushcount=$(pushes "$work/leaf.s")
[ "$callee" -eq 0 ] && ok "call-free leaf references no callee-saved register" \
    || bad "call-free leaf references $callee callee-saved register(s) — the leaf policy is off"
[ "$pushcount" -eq 0 ] && ok "call-free leaf emits no push/pop" \
    || bad "call-free leaf emits $pushcount push/pop — a callee-saved home it never needs"

# ---- 2: the kill switch still isolates the policy ----------------------
CCC_NO_LEAF_CALLER_HOME=1 "$CCC" -O2 -S -o "$work/leaf_nop.s" "$work/leaf.c" \
    || bad "kill-switch -S failed"
callee_no=$(callee_refs "$work/leaf_nop.s")
[ "$callee_no" -ge 1 ] && ok "CCC_NO_LEAF_CALLER_HOME restores the callee-saved shape ($callee_no refs)" \
    || bad "CCC_NO_LEAF_CALLER_HOME changed nothing — the kill switch is dead"
CCC_NO_LEAF_CALLER_HOME=1 "$CCC" -O2 -o "$work/leaf_nop.bin" "$work/leaf.c" || bad "kill-switch link failed"
got=$(CCC_NO_LEAF_CALLER_HOME=1 "$work/leaf_nop.bin") || bad "kill-switch binary did not run"
[ "${got:-}" = "$want" ] && ok "kill-switch binary prints gcc's output" \
    || bad "kill-switch binary printed '${got:-<crash>}'"

# ---- 3: a loop around a call still preserves state --------------------
run_both "$work/caller.c" caller
callee_call=$(callee_refs "$work/caller.s")
[ "$callee_call" -ge 1 ] && ok "call in the loop keeps its state in callee-saved registers ($callee_call)" \
    || bad "call in the loop used no callee-saved register — the leaf policy leaked into a non-leaf"

if [ "$fail" -ne 0 ]; then
    echo "FAIL: leaf caller-saved homes" >&2
    exit 1
fi
echo "PASS: leaf caller-saved homes (a call-free leaf pays no callee-saved register; the kill switch and the call-in-loop arm still behave)"
