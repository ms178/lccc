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
/* Second shape: a volatile GLOBAL read in a counted loop.
 *
 * Measured, not assumed: this one is currently kept correct by LICM's
 * must-execute rule rather than by its volatile guard — `CCC_DEBUG_LICM=1`
 * reports "load v10 in block 2 not hoisted: its block does not dominate every
 * loop block", because the counted loop's header is the guard block and the
 * read sits in the body.  `spin` is the shape whose read IS in the header, so
 * the volatile guard is its only defence and the mutation this gate exists to
 * catch.  Both are pinned: `spin` guards the volatile rule directly, and
 * `spin_global` guarantees the global-address path gets exercised the moment
 * the must-execute rule is ever relaxed. */
volatile u32 gate_counter;
u32 spin_global(int n) { u32 t = 0; for (int i = 0; i < n; i++) t += gate_counter; return t; }
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

# The spin loop must RE-LOAD INSIDE THE LOOP.
#
# The assertion here used to be "the function contains >= 1 memory load and
# >= 1 conditional branch", which is satisfied by the buggy code too: when
# LICM hoisted the volatile load, the emitted shape was
#
#     movl 16(%rdi), %edx     <-- hoisted OUT, still inside `spin`
#   .LBB1:
#     testl %edx, %edx        <-- loop body reads NO memory
#     je .LBB1                <-- spins forever on a stale value
#
# so a function-wide count saw one load, one branch, and passed. That is why
# this regression (a dropped `!*volatile` term in `src/passes/licm.rs`)
# reached a pull request with this file present in the tree. The property
# that matters is *where* the load is, so the check is now scoped to the
# backward-branch body: the load must sit between the branch's target label
# and the branch itself, which is exactly the set of instructions that re-run
# per iteration.
#
# Mutation-verified both ways: reverting the LICM volatile guard makes this
# report `spin: no memory access inside the loop body`, and restoring it
# makes the span hold `movl 16(%rdi), %edx`.

# Number of memory accesses strictly inside `LABEL ... j?? LABEL`, or -1 when
# the function holds no backward branch at all.
#
# `best`/`loops` are initialised in BEGIN on purpose.  mawk 1.3.4 — the
# default awk on Debian/Ubuntu, hence on the CI image — types an *uninitialised*
# variable as the empty string, so `print (loops > 0 ? best : -1)` with a
# never-assigned `best` emits a blank line instead of `0`, and the shell test
# that consumes it dies with "integer expression expected".  Every awk in this
# repo that prints a computed number initialises it first.
mem_accesses_in_loop_body() {
    awk -v fn="$1" '
        # Collect the function body, then compute the loop span in END.  A
        # streaming "count between a label and the branch that targets it" rule
        # is wrong for the rotated shape LCCC emits for a guarded loop, where
        # the back-edge is an unconditional `jmp` at the BOTTOM and the body
        # sits ABOVE its target: the span has to run from the back-edge target
        # to the back-edge, which is only knowable once the whole function has
        # been read.  Over all loops in the function the MAXIMUM is reported,
        # because the property under test is "some loop body still performs the
        # observable access".
        BEGIN { inside = 0; n = 0 }
        $0 ~ "^"fn":" { inside = 1; next }
        inside && /^[A-Za-z_.][A-Za-z0-9_.]*:/ && $0 !~ /^\.L/ { inside = 0 }
        inside { body[++n] = $0 }
        END {
            if (n == 0) { print -1; exit }
            loops = 0; best = 0
            for (i = 1; i <= n; i++) {
                line = body[i]
                if (line ~ /^[[:space:]]*\.L[A-Za-z0-9_]*:/) {
                    lbl = line
                    sub(/^[[:space:]]*/, "", lbl); sub(/:.*$/, "", lbl)
                    if (!(lbl in pos)) pos[lbl] = i
                    continue
                }
                if (line ~ /^[[:space:]]*j[a-z]+[[:space:]]+\.L/) {
                    tgt = line
                    sub(/^[[:space:]]*j[a-z]+[[:space:]]+/, "", tgt)
                    sub(/[[:space:]]*$/, "", tgt)
                    if (tgt in pos && pos[tgt] <= i) {
                        loops++
                        c = 0
                        for (j = pos[tgt]; j <= i; j++) {
                            l = body[j]
                            if (l ~ /^[[:space:]]*[a-z]+[[:space:]]/ && l ~ /\(/ \
                                && l !~ /^[[:space:]]*lea/) c++
                        }
                        if (c > best) best = c
                    }
                }
            }
            if (loops > 0) print best; else print -1
        }' "$tmp/v.s"
}

for fn in spin spin_global; do
    in_loop=$(mem_accesses_in_loop_body "$fn")
    # Fail loud rather than vacuous: a non-numeric answer means the parser and
    # the assembler output disagree, which must never read as "no problem
    # found".  (mawk types an uninitialised variable as "", see the BEGIN
    # block above — that combination produced exactly this silent pass once.)
    case $in_loop in
        ''|*[!0-9-]*)
            echo "FAIL $fn: loop-body parser returned '$in_loop' (not a number)" >&2
            fail=1
            continue
            ;;
    esac
    if [ "$in_loop" -lt 1 ]; then
        if [ "$in_loop" -eq -1 ]; then
            echo "FAIL $fn: no backward branch found — the volatile loop vanished" >&2
        else
            echo "FAIL $fn: no memory access inside the loop body — the volatile load was hoisted (C11 5.1.2.3)" >&2
        fi
        fail=1
    fi
done

if [ "$fail" -ne 0 ]; then
    echo "--- asm ---" >&2
    grep -vE '^\s*\.' "$tmp/v.s" >&2
    exit 1
fi
echo "volatile pointer subscript / arithmetic: ok"
