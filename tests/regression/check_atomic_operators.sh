#!/usr/bin/env bash
# The ordinary operators on `_Atomic` objects are atomic operations.
#
# C11 6.5.2.4p2 and 6.5.16.2p3: `x++` and `x += v` on an atomic object are
# one read-modify-write; 6.2.6.1p9 and 7.17.3p6: a plain read and `x = v`
# are a load and a store with `memory_order_seq_cst`. lccc lowered all four
# as the accesses of a volatile object -- `x++` on an `atomic_int` was
# `mov; lea; mov` -- so an update another thread made in between was lost
# (tests/regression/atomic_operators.c: 229,349 of 400,000 increments
# survived). This gate pins the instruction each operator becomes, on every
# backend, and runs the program at the optimization levels the corpus runner
# does not.
#
# x86-64 and i686, measured against GCC 16.2 and Clang 23.1.5 at -O2:
#   * `x++;` `x -= v;` `x |= v;` with the result unused: one locked ALU
#     instruction on the object (`lock add`, `lock sub`, `lock or`). The
#     same holds for `atomic_fetch_or` with its result dropped, which ran a
#     five-instruction compare-exchange loop.
#   * `++x` with the result used: `lock xadd`.
#   * `x = v`: `xchg`. A read: one plain `mov` (an aligned load is atomic,
#     and `xchg`'s implied lock is the store's barrier).
#   * `x *= v`, `d += v` on a double, and every other update no single
#     instruction performs: a compare-exchange loop whose retry branch reads
#     ZF from `lock cmpxchg` itself (no `sete; movzbl; test`).
# AArch64: exclusive load/store pairs, `stlr` for a store, `ldar` for a
# load. RISC-V: `amoadd`, `lr`/`sc`, and fenced loads and stores.
set -euo pipefail

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
bindir=$(dirname "${LCCC_BIN:-$repo/target/fastbuild/lccc}")
ccc=${LCCC_BIN:-$bindir/lccc}
arm=$bindir/lccc-arm
riscv=$bindir/lccc-riscv
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

[ -x "$ccc" ] || { echo "SKIP: $ccc not built" >&2; exit 0; }
fail() { echo "FAIL: $*" >&2; exit 1; }

cat >"$tmp/probe.c" <<'PROBE'
_Atomic int x;
_Atomic long y;
_Atomic double d;
_Atomic unsigned char c;
_Atomic long long w;
struct S { int plain; _Atomic int refs; };
void inc(void) { x++; }
void dec(struct S *s) { s->refs--; }
int pre(void) { return ++x; }
void orl(long v) { y |= v; }
void mul(int v) { x *= v; }
void st(int v) { x = v; }
int ld(void) { return x; }
void dadd(double v) { d += v; }
void cadd(int v) { c += v; }
void wadd(long long v) { w += v; }
void wst(long long v) { w = v; }
void fo_unused(long v) { __atomic_fetch_or(&y, v, __ATOMIC_SEQ_CST); }
void plain_inc(struct S *s) { s->plain++; }
PROBE

# The body of one function of an assembly file.
body() {
    sed -n "/^$2:/,/^\\.size $2,/p" "$1"
}
# has FILE FUNCTION REGEX: the function contains a line matching REGEX.
has() {
    local text
    text=$(body "$1" "$2")
    grep -qE "$3" <<<"$text" || fail "$1: $2 has no '$3'"$'\n'"$text"
}
# lacks FILE FUNCTION REGEX: the function contains no such line.
lacks() {
    local text
    text=$(body "$1" "$2")
    if grep -qE "$3" <<<"$text"; then
        fail "$1: $2 contains '$3'"$'\n'"$text"
    fi
}

# ---------------------------------------------------------------- x86-64 ---
for opt in -O1 -O2 -O3 -Os; do
    s="$tmp/x86$opt.s"
    "$ccc" "$opt" -S "$tmp/probe.c" -o "$s" || fail "lccc $opt build of the probe"
    has "$s" inc 'lock addl \$1,'
    lacks "$s" inc 'xadd|cmpxchg'
    has "$s" dec 'lock subl \$1, 4\(%rdi\)|lock subl \$1,'
    has "$s" pre 'lock xaddl'
    has "$s" orl 'lock orq'
    lacks "$s" orl 'cmpxchg'
    has "$s" mul 'lock cmpxchgl'
    has "$s" mul '^\s+jn?e '
    lacks "$s" mul 'sete|setne'
    has "$s" st 'xchgl'
    lacks "$s" ld 'lock|xchg'
    has "$s" dadd 'lock cmpxchgq'
    lacks "$s" dadd 'sete|setne'
    has "$s" cadd 'lock addb'
    has "$s" wadd 'lock addq'
    has "$s" wst 'xchgq'
    has "$s" fo_unused 'lock orq'
    lacks "$s" fo_unused 'cmpxchg'
    # A member that is not atomic beside one that is keeps its plain update.
    lacks "$s" plain_inc 'lock|xchg'
done
# Unoptimized code is atomic too: what -O0 must not have is the plain
# read-modify-write, in whatever instructions.
s="$tmp/x86-O0.s"
"$ccc" -O0 -S "$tmp/probe.c" -o "$s" || fail "lccc -O0 build of the probe"
has "$s" inc 'lock (addl|xaddl)'
has "$s" orl 'lock (orq|cmpxchgq)'
has "$s" mul 'lock cmpxchgl'
has "$s" st 'xchgl'
lacks "$s" plain_inc 'lock|xchg'
echo "PASS: x86-64 operators are locked instructions at -O0, -O1, -O2, -O3 and -Os"

# ------------------------------------------------------------------ i686 ---
# -S needs no 32-bit headers or libraries, so this leg runs everywhere.
s="$tmp/i686.s"
"$ccc" -m32 -O2 -fno-pic -S "$tmp/probe.c" -o "$s" || fail "lccc -m32 build of the probe"
has "$s" inc 'lock (addl|xaddl)'
has "$s" pre 'lock xaddl'
has "$s" mul 'lock cmpxchgl'
has "$s" st 'xchgl'
has "$s" wadd 'lock cmpxchg8b'
has "$s" wst 'lock cmpxchg8b'
has "$s" dadd 'lock cmpxchg8b'
lacks "$s" plain_inc 'lock|xchg'
echo "PASS: i686 operators are locked instructions, 64-bit objects through cmpxchg8b"

# --------------------------------------------------------------- AArch64 ---
if [ -x "$arm" ]; then
    s="$tmp/arm.s"
    "$arm" -O2 -S "$tmp/probe.c" -o "$s" || fail "lccc-arm build of the probe"
    has "$s" inc 'ldaxr'
    has "$s" inc 'stlxr'
    has "$s" mul 'stlxr'
    has "$s" st 'stlr'
    has "$s" ld 'ldar'
    has "$s" wadd 'stlxr'
    lacks "$s" plain_inc 'ldaxr|stlxr|ldar|stlr'
    echo "PASS: AArch64 operators are exclusive pairs, ldar and stlr"
else
    echo "SKIP: $arm not built"
fi

# ---------------------------------------------------------------- RISC-V ---
if [ -x "$riscv" ]; then
    s="$tmp/riscv.s"
    "$riscv" -O2 -S "$tmp/probe.c" -o "$s" || fail "lccc-riscv build of the probe"
    has "$s" inc 'amoadd\.w'
    has "$s" mul 'sc\.w'
    has "$s" st 'fence'
    has "$s" ld 'fence'
    has "$s" wadd 'amoadd\.d'
    lacks "$s" plain_inc 'amo|lr\.|sc\.|fence'
    echo "PASS: RISC-V operators are AMOs, lr/sc pairs and fenced accesses"
else
    echo "SKIP: $riscv not built"
fi

# --------------------------------------------------------------- runtime ---
# The corpus runner builds atomic_operators.c at -O2 and compares it with
# GCC. Here it runs at the other levels; the program checks its own thread
# totals (exit status), and the values every operator yields must not depend
# on the level.
src="$repo/tests/regression/atomic_operators.c"
ref=
for opt in -O0 -O1 -O2 -O3 -Os; do
    "$ccc" "$opt" "$src" -o "$tmp/run$opt" || fail "lccc $opt build of atomic_operators.c"
    out=$("$tmp/run$opt") || fail "atomic_operators.c at $opt lost an update:"$'\n'"$out"
    if [ -z "$ref" ]; then
        ref=$out
    elif [ "$out" != "$ref" ]; then
        fail "atomic_operators.c at $opt differs from -O0:"$'\n'"$(diff <(echo "$ref") <(echo "$out"))"
    fi
done
echo "PASS: atomic_operators.c keeps every update at -O0, -O1, -O2, -O3 and -Os"
