#!/usr/bin/env bash
# check_volatile_pointer_subscript.sh — volatile qualifier must survive
# array subscripts and pointer arithmetic on pointer-to-volatile.
#
# Bug class (fixed in src/ir/lowering/lvalue.rs, expr_access_is_volatile /
# pointee_expr_is_volatile): `volatile u32 *regs; regs[i]` and `*(regs + i)`
# were lowered with `volatile: false` (only the plain `*regs` form kept the
# qualifier). The consequences are exactly the MMIO idioms the Linux kernel
# relies on:
#   * `while (!regs[STATUS]) ;`  → load hoisted, infinite loop;
#   * `regs[CTRL] = 1; regs[CTRL] = 2;` → first write dead-store-eliminated;
#   * `x = regs[DATA] + regs[DATA];` → two reads CSE'd into one.
#
# Each probe below is compiled at -O2 and the emitted instruction count for
# the access is asserted. Mutation-verified: reverting the lvalue.rs fix
# makes every probe report a single access (or none for the spin loop).
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-volatile-subscript.XXXXXX")
trap 'rm -rf "$tmp"' EXIT

cat > "$tmp/v.c" <<'EOF'
typedef unsigned int u32;
u32 rd_subscript(volatile u32 *p)            { return p[1] + p[1]; }
u32 rd_subscript_var(volatile u32 *p, long i){ return p[i] + p[i]; }
u32 rd_arith(volatile u32 *p, long i)        { return *(p + i) + *(p + i); }
u32 rd_swapped(volatile u32 *p, long i)      { return i[p] + i[p]; }
u32 rd_sub(volatile u32 *p)                  { return *(p - 1) + *(p - 1); }
void wr_subscript(volatile u32 *p)           { p[2] = 1; p[2] = 2; }
void wr_subscript_var(volatile u32 *p, long i){ p[i] = 1; p[i] = 2; }
void wr_arith(volatile u32 *p, long i)       { *(p + i) = 1; *(p + i) = 2; }
void wr_cond(volatile u32 *p, volatile u32 *q, int c) { *(c ? p : q) = 1; *(c ? p : q) = 2; }
void spin(volatile u32 *regs)                { while (!regs[4]) { } }
u32 arr_of_ptr(void) { static volatile u32 *tbl[4]; return *tbl[1] + *tbl[1]; }
/* Negative control: a pointer to NON-volatile must still CSE to one load. */
u32 plain(u32 *p)                            { return p[1] + p[1]; }
EOF
"$CCC" -O2 -S "$tmp/v.c" -o "$tmp/v.s"

# Count memory-operand instructions (any mnemonic except lea — folded
# `addl 4(%rdi), %edx` is a real second access) inside one function body.
count_mem_movs() {
    awk -v fn="$1" '
        $0 ~ "^"fn":" { inside = 1; next }
        inside && /^[A-Za-z_.][A-Za-z0-9_.]*:/ && $0 !~ /^\.L/ { inside = 0 }
        inside && /^[[:space:]]*[a-z]+[[:space:]]/ && /\(/ && $1 !~ /^lea/ { n++ }
        END { print n + 0 }' "$tmp/v.s"
}

fail=0
expect() { # expect <fn> <op> <n>
    local got; got=$(count_mem_movs "$1")
    if ! [ "$got" "$2" "$3" ]; then
        echo "FAIL $1: expected memory movs $2 $3, got $got" >&2
        fail=1
    fi
}
for fn in rd_subscript rd_subscript_var rd_arith rd_swapped rd_sub \
          wr_subscript wr_subscript_var wr_arith wr_cond arr_of_ptr; do
    expect "$fn" -ge 2
done
expect plain -le 1

# The spin loop must re-load inside the loop: a backward branch whose body
# contains a memory read. Assert the load count is >= 1 AND there is a
# conditional branch (an empty infinite loop would have neither).
spin_loads=$(count_mem_movs spin)
spin_branch=$(awk '/^spin:/{i=1;next} i&&/^[A-Za-z_][A-Za-z0-9_]*:/{i=0} i&&/^[[:space:]]*j(e|ne|z|nz)/{n++} END{print n+0}' "$tmp/v.s")
if [ "$spin_loads" -lt 1 ] || [ "$spin_branch" -lt 1 ]; then
    echo "FAIL spin: load hoisted out of the volatile spin loop (loads=$spin_loads branches=$spin_branch)" >&2
    fail=1
fi

if [ "$fail" -ne 0 ]; then
    echo "--- asm ---" >&2
    grep -vE '^\s*\.' "$tmp/v.s" >&2
    exit 1
fi
echo "volatile pointer subscript / arithmetic: ok"
