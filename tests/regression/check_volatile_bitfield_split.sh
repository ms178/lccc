#!/usr/bin/env bash
# check_volatile_bitfield_split.sh — a volatile bitfield access must survive, and stay put.
#
# WHY THIS IS SEPARATE FROM THE OTHER VOLATILE GATES
# --------------------------------------------------
# check_volatile_spin_loop.sh and check_volatile_pointer_subscript.sh cover
# accesses that lower to a SINGLE `Load`/`Store`.  A bitfield does not: it
# lowers to a read-modify-write (load the storage unit, mask, or, store), and
# a bitfield that straddles its storage unit lowers to TWO of them, in two
# different functions of `src/ir/lowering/expr_assign.rs`
# (`store_bitfield`/`store_bitfield_split`) and one of the read side
# (`extract_bitfield_from_addr`).  Those three emit their own instructions, so
# they each need their own volatility answer, and a gate that only inspects
# straight-line `Store`s cannot see them.
#
# The three paths, and what they lower to:
#
#   struct P { unsigned int small : 5; };          one unit  -> load+store
#   struct F { unsigned long long all : 64; };     full unit -> store only
#   struct __attribute__((packed)) S {             two units -> load+store,
#       unsigned char lo : 6; unsigned char hi : 6;             twice
#   };
#
# MEASURED FAILURE MODE (the reason this file exists)
# ---------------------------------------------------
# Before the fix, every one of those accesses was constructed with
# `volatile: false`, and at -O2 that is not a missed optimisation but a
# wrong answer:
#
#   void vol_full_loop(unsigned long long x, int n)   LCCC (before)   GCC 14
#   -------------------------------------------------------------  ---------
#   for (int i = 0; i < 4; i++) vf.all = x;            1 store       4 stores
#
# and the same shape non-unrolled hoisted a volatile load out of its loop.
# C11 5.1.2.3: "Every access to such an object is done as a single access, in
# program order."  N accesses collapsing to 1 is exactly what it forbids.
#
# THE CONTROLS ARE THE POINT
# --------------------------
# Every volatile probe is paired with a NON-volatile twin of the same shape.
# The twin's accesses are redundant by construction, so the compiler is
# *supposed* to merge them, and the gate asserts that it did.  Without the
# pair, a compiler that simply never merged anything would pass the volatile
# half for the wrong reason.
set -euo pipefail

CCC=${CCC:-./target/fastbuild/lccc}
[[ -x $CCC ]] || { echo "check_volatile_bitfield_split: lccc not found at $CCC" >&2; exit 1; }
tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-vol-bitfield.XXXXXX")
trap 'rm -rf "$tmp"' EXIT

cat > "$tmp/v.c" <<'EOF'
typedef unsigned long long u64;

struct P { unsigned int small : 5; };
struct F { u64 all : 64; };
struct __attribute__((packed)) S { unsigned char lo : 6; unsigned char hi : 6; };

/* ---- the contract: object-level volatile ---------------------------------- */
volatile struct P vp;
volatile struct F vf;
volatile struct S vs;

void vol_plain(unsigned x)  { vp.small = x; vp.small = x; }
void vol_full(u64 x)        { vf.all = x; vf.all = x; }
void vol_spread(unsigned x) { vs.lo = x; vs.lo = x; vs.hi = x; vs.hi = x; }
unsigned vol_read_spread(void) { return vs.lo + vs.hi; }

/* ---- the controls: identical shapes, non-volatile, MUST be merged ---------- */
struct P np;
struct F nf;
struct S ns;

void nv_plain(unsigned x)  { np.small = x; np.small = x; }
void nv_full(u64 x)        { nf.all = x; nf.all = x; }
void nv_spread(unsigned x) { ns.lo = x; ns.lo = x; ns.hi = x; ns.hi = x; }

/* ---- loops: unknown trip count, so a loop survives to be asserted about ---- */
void vol_write_loop(u64 x, int n) { for (int i = 0; i < n; i++) vf.all = x; }
unsigned vol_read_loop(int n)     { unsigned s = 0; for (int i = 0; i < n; i++) s += vp.small; return s; }
void vol_write_split_loop(unsigned x, int n) { for (int i = 0; i < n; i++) vs.lo = x; }
unsigned vol_read_split_loop(int n) { unsigned s = 0; for (int i = 0; i < n; i++) s += vs.lo + vs.hi; return s; }

/* Constant trip count on purpose.  With a known trip count the loop is fully
 * unrolled, which is the shape that exposes the ORIGINAL bug: the unrolled
 * iterations each re-read the same volatile object with no intervening store,
 * so a non-volatile load is CSE'd down to one and 4 accesses become 1.  This is
 * the manifestation that a loop-position assertion cannot see (the loop no
 * longer exists by then), so it is counted over the whole function.  The
 * threshold is 3 rather than 4 so that a future partial-unroll decision is not
 * a spurious failure, while a collapse to a single access still trips it. */
unsigned vol_read_const_loop(void) { unsigned s = 0; for (int i = 0; i < 4; i++) s += vp.small; return s; }
void vol_write_const_loop(u64 x)   { for (int i = 0; i < 4; i++) vf.all = x; }
EOF
"$CCC" -O2 -S "$tmp/v.c" -o "$tmp/v.s"

# shellcheck source=tests/regression/lib_loop_bounds.sh
. "$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)/lib_loop_bounds.sh"

# count_mem_fn <asm> <fn> — memory-operand instructions of a whole function.
count_mem_fn() { count_mem_in_range "$1" "$2" 0 1000000; }

fail=0
check() { # check <label> <fn> <mode:ge|eq> <want>
    local got; got=$(count_mem_fn "$tmp/v.s" "$2")
    case "$3" in
        ge) [ "$got" -ge "$4" ] || { echo "FAIL $1: '$2' has $got memory access(es), need >= $4" >&2; return 1; } ;;
        eq) [ "$got" -eq "$4" ] || { echo "FAIL $1: '$2' has $got memory access(es), want exactly $4" >&2; return 1; } ;;
    esac
    echo "    ok   $1  ('$2': $got)"
}

echo "  volatile bitfield accesses are kept (>= one per source access):"
check "volatile single-unit write, twice"  vol_plain       ge 4   || fail=1
check "volatile full-width write, twice"   vol_full        ge 2   || fail=1
check "volatile packed split write, 4x"    vol_spread      ge 8   || fail=1
check "volatile packed split read"         vol_read_spread ge 2   || fail=1

echo "  non-volatile controls are merged (so the probe can tell the two apart):"
check "non-volatile single-unit write"     nv_plain        eq 2   || fail=1
check "non-volatile full-width write"      nv_full         eq 1   || fail=1

check "volatile read, 4 unrolled iterations"  vol_read_const_loop  ge 3 || fail=1
check "volatile full-width store x4 unrolled" vol_write_const_loop ge 3 || fail=1

echo "  and remain left in place (positional, not just counted):"
require_loop_accesses "volatile full-width store stays in its loop" "$tmp/v.s" vol_write_loop ge 1 || fail=1
require_loop_accesses "volatile bitfield load stays in its loop"    "$tmp/v.s" vol_read_loop  ge 1 || fail=1
require_loop_accesses "volatile SPLIT store stays in its loop"      "$tmp/v.s" vol_write_split_loop ge 1 || fail=1
require_loop_accesses "volatile SPLIT read stays in its loop"       "$tmp/v.s" vol_read_split_loop  ge 2 || fail=1

if [ "$fail" -ne 0 ]; then
    echo "--- asm ---" >&2
    grep -vE '^\s*\.(cfi|loc|file|size|type|globl|p2align|ident|section|weak)' "$tmp/v.s" >&2
    exit 1
fi
echo "check_volatile_bitfield_split: ok"
