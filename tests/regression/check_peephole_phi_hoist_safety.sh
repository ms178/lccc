#!/usr/bin/env bash
# Peephole phi-diamond preinitialisation safety gate.
#
# The one-move phi-diamond pattern hoists the fallthrough arm's MOV above
# the conditional branch (preinitialise the cheap incoming, invert the
# branch, drop the unconditional join). A MOV whose SOURCE is user memory
# may fault on the path it is hoisted onto: `c ? *p : *q` must never touch
# *p when c selects q, whatever p is — including NULL. Before the
# hoistable-source guard, lccc compiled sel(0, NULL, &valid) to
#
#     testl %edi, %edi
#     movl (%rsi), %edi        # dereferenced unconditionally
#     jne ...
#
# and the program segfaulted on a pointer the source never reads. The
# guard admits register, immediate, and plain displacement-only stack
# sources; everything else stays inside its arm.
#
# This gate executes the C-level contract on every branch polarity and
# arm-width mix, and checks that the safe (register-init) shapes still
# preinitialise — the optimisation must survive its own safety fix.
set -euo pipefail

CCC=${CCC:-target/fastbuild/lccc}
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT

cat >"$td/hoist.c" <<'EOF'
#include <stdio.h>
#include <stdint.h>

/* Every function returns `c ? left : right` with the LEFT pointer NULL
 * when the test drives c to select RIGHT: the load in the untaken arm
 * must never execute. Callers go through volatile pointers so the
 * functions are emitted standalone (inlining can reshape the arms). */
static int sel_load_load(int c, int *p, int *q) { return c ? *p : *q; }
static int sel_load_reg(int c, int *p, int r) { return c ? *p : r; }
static long long sel_load_load64(int c, long long *p, long long *q) { return c ? *p : *q; }
static int sel_load_load_swapped(int c, int *p, int *q) { return !c ? *q : *p; }
static int sel_indexed(int c, int *p, int i, int *q) { return c ? p[i] : *q; }
static void *sel_ptr(int c, void *p, void *q) { return c ? p : q; }

int main(void) {
    int a = 11, b = 22;
    long long x = 333, y = 444;
    int (*volatile fll)(int, int *, int *) = sel_load_load;
    int (*volatile flr)(int, int *, int) = sel_load_reg;
    long long (*volatile fll64)(int, long long *, long long *) = sel_load_load64;
    int (*volatile flls)(int, int *, int *) = sel_load_load_swapped;
    int (*volatile fidx)(int, int *, int, int *) = sel_indexed;
    void *(*volatile fptr)(int, void *, void *) = sel_ptr;

    int bad = 0;
    /* NULL in the UNTAKEN arm only: each call must not fault. */
    if (fll(0, (int *)0, &b) != 22) { puts("sel_load_load wrong"); bad = 1; }
    if (flr(0, (int *)0, 5) != 5) { puts("sel_load_reg wrong"); bad = 1; }
    if (fll64(0, (long long *)0, &y) != 444) { puts("sel_load_load64 wrong"); bad = 1; }
    /* !c ? *q : *p with c==1 selects *p: q is the untaken arm. */
    if (flls(1, &b, (int *)0) != 22) { puts("sel_load_load_swapped wrong"); bad = 1; }
    if (fidx(0, (int *)0, 3, &b) != 22) { puts("sel_indexed wrong"); bad = 1; }
    if (fptr(0, (void *)0, &b) != &b) { puts("sel_ptr wrong"); bad = 1; }
    /* Taken arms still read real memory. */
    if (fll(1, &a, (int *)0) != 11) { puts("sel_load_load taken wrong"); bad = 1; }
    if (fll64(1, &x, (long long *)0) != 333) { puts("sel_load_load64 taken wrong"); bad = 1; }
    if (fidx(1, &a, 0, (int *)0) != 11) { puts("sel_indexed taken wrong"); bad = 1; }
    if (bad) return 1;
    puts("phi-hoist safety runtime OK");
    return 0;
}
EOF

"$CCC" -O2 "$td/hoist.c" -o "$td/hoist" || { echo "FAIL: build" >&2; exit 1; }
"$td/hoist" || { echo "FAIL: untaken-arm load executed (fault or wrong value)" >&2; exit 1; }

# The optimisation itself must survive: a register-source init still
# preinitialises (the unconditional join disappears). Build the positive
# control from source that produces the register-phi shape.
cat >"$td/pos.c" <<'EOF'
int reg_phi(int c, int r, int s) {
    int t;
    if (c) { t = r; } else { t = s; }
    return t;
}
EOF
"$CCC" -O2 -S "$td/pos.c" -o "$td/pos.s"
if grep -qE '\bjmp[[:space:]]+\.L' "$(dirname "$td")/dev/null" 2>/dev/null; then :; fi
# Count unconditional jumps to local labels inside reg_phi: the join
# branch must be gone (at most the function's own return padding).
n_jmp=$(sed -n '/^reg_phi:/,/^\.size[[:space:]]*reg_phi/p' "$td/pos.s" | grep -cE '^[[:space:]]*jmp[[:space:]]+\.L' || true)
if [ "$n_jmp" -ne 0 ]; then
    echo "FAIL: reg_phi still has $n_jmp unconditional local jump(s); the" \
         "register-init preinitialisation regressed:" >&2
    sed -n '/^reg_phi:/,/^\.size[[:space:]]*reg_phi/p' "$td/pos.s" >&2
    exit 1
fi
echo "OK peephole_phi_hoist_safety"
