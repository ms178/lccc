#!/usr/bin/env bash
# check_volatile_spin_loop.sh — a volatile access must stay inside its loop.
#
# WHY THIS IS A SEPARATE GATE FROM check_volatile_pointer_subscript.sh
# ---------------------------------------------------------------------
# That script asserts the *access count* per function.  It is the right
# instrument for "the access was eliminated" and provably the WRONG one for
# "the access was moved": hoisting a load out of a loop still leaves exactly
# one memory-operand instruction in the function, so an access-count
# assertion passes on a miscompiled spin loop.  It was measured passing while
# `while (!regs[4]) { }` emitted
#
#     spin:
#         movl 16(%rdi), %edx      <-- hoisted: read once, loop forever
#     .LBB1:
#         testl %edx, %edx
#     je .LBB1
#
# which is the archetypal MMIO poll and a hang on real hardware.  The
# assertion this file makes is positional instead: the access must be between
# the loop header and the backward branch that closes it.
#
# THE CONTROL IS THE POINT
# ------------------------
# Every probe here is paired with a non-volatile twin of the same shape.  The
# twin is a loop-invariant load, so LICM is *supposed* to hoist it, and the
# gate asserts that it did.  Without that pair the volatile assertion could
# pass for the wrong reason -- a compiler that hoisted nothing at all, or a
# helper that cannot see loops -- and the gate would be decoration.  With it,
# a green run means the two cases were distinguished, which is the only
# property that makes the volatile half meaningful.
#
# C11 5.1.2.3: "An object that has volatile-qualified type may be modified in
# ways unknown to the implementation or by other unknown agents. ... Every
# access to such an object is done as a single access, in program order."
# Hoisting turns N accesses into one.
set -euo pipefail

CCC=${CCC:-./target/fastbuild/lccc}
[[ -x $CCC ]] || { echo "check_volatile_spin_loop: lccc not found at $CCC" >&2; exit 1; }
tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-volatile-loop.XXXXXX")
trap 'rm -rf "$tmp"' EXIT

cat > "$tmp/v.c" <<'EOF'
typedef unsigned int u32;

/* ---- the contract: volatile accesses re-execute every iteration -------- */
u32 spin_index(volatile u32 *regs)   { while (!regs[4]) { } return regs[5]; }
u32 spin_ptr(volatile u32 *regs)     { volatile u32 *p = regs; while (!*p) { } return 0; }
u32 spin_two(volatile u32 *regs)     { while (!regs[4] && !regs[5]) { } return 0; }
u32 spin_local(volatile u32 *regs)   { volatile u32 s; do { s = regs[4]; } while (!s); return s; }
u32 spin_ptrd(volatile u32 *regs, int d) { volatile u32 *p = regs + d; while (!*p) { } return 0; }

/* ---- the controls: identical shapes, non-volatile, MUST be hoisted ----- */
u32 plain_index(u32 *regs)           { while (!regs[4]) { } return regs[5]; }
u32 plain_ptr(u32 *regs)             { u32 *p = regs; while (!*p) { } return 0; }
u32 plain_local(u32 *regs)           { u32 s; do { s = regs[4]; } while (!s); return s; }

/* ---- stores: a volatile store must stay in the loop too ---------------- */
/* Unknown trip count on purpose: a constant-trip store loop is fully unrolled
 * at -O2, which leaves no loop to assert about and proves nothing. */
void store_spin(volatile u32 *regs, int n) { for (int i = 0; i < n; i++) regs[0] = (u32)i; }
EOF
"$CCC" -O2 -S "$tmp/v.c" -o "$tmp/v.s"

# The position primitives live in lib_loop_bounds.sh so that this gate and
# check_loop_preheader.sh measure "inside the loop" with exactly the same
# definition.  Two instruments that drift apart on the one question they exist
# to answer is how a suite ends up with a gate that can see a hoist and one
# that cannot.
# shellcheck source=tests/regression/lib_loop_bounds.sh
. "$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)/lib_loop_bounds.sh"

fail=0

echo "  volatile accesses stay in the loop body:"
for fn in spin_index spin_ptr spin_two spin_local spin_ptrd store_spin; do
    require_loop_accesses "volatile $fn" "$tmp/v.s" "$fn" ge 1 \
        || fail=1
done
echo "  non-volatile controls are hoisted (so the probe can tell the two apart):"
for fn in plain_index plain_ptr plain_local; do
    require_loop_accesses "control $fn" "$tmp/v.s" "$fn" eq 0 \
        || fail=1
done

if [ "$fail" -ne 0 ]; then
    echo "--- asm ---" >&2
    grep -vE '^\s*\.(cfi|loc|file|size|type|globl|p2align|ident|section|weak)' "$tmp/v.s" >&2
    exit 1
fi
echo "check_volatile_spin_loop: ok"
