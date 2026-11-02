#!/usr/bin/env bash
# C `volatile` access semantics (C11 5.1.2.3): every volatile load and store is
# an observable side effect.  Guards the whole pipeline: IR flags, lowering,
# mem2reg flag propagation, and the optimizer gates (store-load forwarding,
# load CSE/forwarding, GVN, DCE, LICM, loop memory promotion).
#
#  1. store-then-load of a volatile global must re-read memory (no forwarding)
#  2. two volatile loads must not be CSE'd (loop executes N reads)
#  3. a dead-result volatile load must survive DCE
#  4. *p through a pointer-to-volatile parameter must load
#  5. volatile locals keep their RMW shape (no mem2reg promotion)
#  6. a volatile or _Atomic object read-modify-written in a loop keeps its
#     per-iteration access: no pass may collapse N read+write pairs into one
#     accumulated update.  This is the PR #716 review's "gate the policy where
#     volatility is still visible" item: the peephole RMW fold (one read, one
#     write, no other access in between) is sound for a single-threaded
#     observer, but nothing downstream may multiply that into a whole loop.
#     The check is SHAPE + ORACLE, and says so: the loop must still contain a
#     reference to the object and a back edge.
set -euo pipefail

CCC=${CCC:-./target/fastbuild/lccc}
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT

cat >"$td/vol.c" <<'EOF'
volatile int counter = 5;
volatile int sink;
int read_after_store(void) { counter = 7; return counter; }   /* want: load */
int reads_in_loop(int n) { int t = 0; for (int i = 0; i < n; i++) t += counter; return t; }
int dead_read(void) { counter; sink = 1; return 0; }          /* load must survive */
int deref_param(volatile int *p) { return *p; }
int volatile_local(void) { volatile int loc = 3; loc = loc + 1; return loc; }
_Atomic int atomic_sum = 0;
volatile int vol_sum = 0;
void atomic_acc(int n) { for (int i = 0; i < n; i++) atomic_sum += 3; }
void vol_acc(int n) { for (int i = 0; i < n; i++) vol_sum += 3; }
EOF

cat >"$td/acc.c" <<'EOF'
#include <stdio.h>
_Atomic int atomic_sum = 0;
volatile int vol_sum = 0;
void atomic_acc(int n) { for (int i = 0; i < n; i++) atomic_sum += 3; }
void vol_acc(int n) { for (int i = 0; i < n; i++) vol_sum += 3; }
int main(void) { atomic_acc(1000); vol_acc(1000); printf("%d %d\n", (int)atomic_sum, vol_sum); return 0; }
EOF

rc=0
# Accept register-indirect and direct RIP-relative memory operands, plus the
# signed-widening form selected for an i32 value on x86-64. RA-01 deliberately
# turns ordinary PIE globals into `symbol(%rip)` accesses.
load_pat='mov(l|slq) +[^,]*\(%[re]?[a-z0-9]+'
# A volatile READ is an observable access whatever instruction performs it, so
# the loop check cannot be spelled "there is a mov": `t += counter` selects the
# tighter fused read-modify-write `addl counter(%rip), %esi` (one instruction
# where GCC emits `movl counter(%rip), %ecx` + `addl %ecx, %edx`), and a
# mov-only pattern reported that correct code as "volatile load eliminated".
# What must not happen is the access disappearing from the loop, so the pattern
# accepts any ALU instruction with a trailing memory operand — the memory
# operand is last in AT&T, which is what keeps a *store* (`movl %eax,
# counter(%rip)`) out of the match.
read_pat='(mov[lbwlq]?|movs(bl|bq|wl|wq|lq)|add[lbwlq]?|sub[lbwlq]?|and[lbwlq]?|or[lbwlq]?|xor[lbwlq]?|cmp[lbwlq]?) +[^,]*\(%[re]?[a-z0-9]+'
check() { # check <fn> <grep-pattern> <description>
    local fn=$1 pat=$2 desc=$3
    local body
    body=$(awk -v f="$fn" '$0==f":"{ins=1} ins{print} /^\.size/{if(ins)exit}' "$td/vol.s")
    if [ -z "$body" ]; then echo "FAIL: $fn not found"; rc=1; return; fi
    if echo "$body" | grep -Ec "$pat" >/dev/null; then
        echo "ok: $desc"
    else
        echo "FAIL: $desc (pattern '$pat' not in $fn)"; rc=1
    fi
}

for lvl in O0 O1 O2 Os; do
    echo "== $lvl"
    "$CCC" -$lvl -S "$td/vol.c" -o "$td/vol.s" || { echo "FAIL: compile at -$lvl"; rc=1; continue; }
    # 1. a real load of counter between the store and the return
    body=$(awk '/^read_after_store:/,/^\.size/' "$td/vol.s")
    # `grep -c` exits 1 on a zero count, which under `set -e` would abort the
    # whole script instead of reporting the one failed check.
    loads=$(echo "$body" | grep -Ec "$load_pat" || true)
    case $loads in ''|*[!0-9]*) echo "FAIL: load count unreadable ('$loads')"; rc=1; loads=0 ;; esac
    if [ "$loads" -ge 1 ]; then echo "ok: store-then-load reloads memory"; else echo "FAIL: volatile load forwarded/eliminated"; rc=1; fi
    # 2. the volatile read must happen INSIDE the loop, once per iteration.
    #
    # The property is position, not mnemonic. lccc folds the read into
    # `addl counter(%rip), %esi`, which is ONE instruction instead of a
    # `movl` plus an `addl` -- better code that a regex demanding a separate
    # load reports as a failure. So slice out the loop (from the backward
    # branch's target label to the branch itself) and require the volatile
    # object to be referenced in it. Hoisting it out would leave the loop
    # with no reference at all.
    fn_body=$(awk '/^reads_in_loop:/,/^\.size/' "$td/vol.s")
    tail_ln=$(echo "$fn_body" | grep -nE '^[[:space:]]*j[a-z]+ +\.[A-Za-z]' | tail -1 | cut -d: -f1)
    if [ -z "$tail_ln" ]; then
        echo "FAIL: reads_in_loop has no backward branch (loop unrolled?)"; rc=1
    else
        tail_lbl=$(echo "$fn_body" | sed -n "${tail_ln}p" | grep -oE '\.[A-Za-z][A-Za-z0-9_]*' | tail -1)
        head_ln=$(echo "$fn_body" | grep -n "^${tail_lbl}:" | sed -n '1,1p' | cut -d: -f1)
        loop=$(echo "$fn_body" | sed -n "${head_ln},${tail_ln}p")
        if echo "$loop" | grep -c 'counter' >/dev/null; then
            echo "ok: loop keeps the volatile read in the body"
        else
            echo "FAIL: volatile read hoisted out of the loop body"
            echo "      loop was: $loop"
            rc=1
        fi
        # NEGATIVE CONTROL: the loop must still be a loop. If the read
        # vanished because the whole loop was optimised away, the check
        # above would pass for the wrong reason.
        if ! echo "$loop" | grep -cE '^[[:space:]]*j[a-z]+ +\.[A-Za-z]' >/dev/null; then
            echo "FAIL: no back edge in the extracted loop region"; rc=1
        fi
    fi
    # 3. dead read survives
    check dead_read "$load_pat" "dead-result volatile load survives DCE"
    # 4. deref through pointer param loads
    check deref_param "$load_pat" "*volatile-ptr param loads"
    # 5. volatile local: store;load;store sequence
    body=$(awk '/^volatile_local:/,/^\.size/' "$td/vol.s")
    n=$(echo "$body" | grep -Ec 'mov[a-z]* +[^#]*\(%(rsp|rbp|esp|ebp)' || true)
    case $n in ''|*[!0-9]*) echo "FAIL: mem access count unreadable ('$n')"; rc=1; n=0 ;; esac
    if [ "$n" -ge 3 ]; then echo "ok: volatile local keeps RMW"; else echo "FAIL: volatile local promoted (mem access count $n < 3)"; rc=1; fi
done

# 6. the accumulated-RMW loop, for the two spellings whose accesses are
# observable.  `gcc -O0` is the oracle for the VALUE; the pattern is for the
# SHAPE, because access COUNT is not observable from stdout in a
# single-threaded program -- an interpreter of the asm is what would count it,
# and the honest static claim is "the object is still accessed every iteration
# and the loop still loops".
cat >"$td/acc_oracle.c" <<'EOF'
#include <stdio.h>
_Atomic int atomic_sum = 0;
volatile int vol_sum = 0;
void atomic_acc(int n) { for (int i = 0; i < n; i++) atomic_sum += 3; }
void vol_acc(int n) { for (int i = 0; i < n; i++) vol_sum += 3; }
int main(void) { atomic_acc(1000); vol_acc(1000); printf("%d %d\n", (int)atomic_sum, vol_sum); return 0; }
EOF
if command -v gcc >/dev/null 2>&1; then
    gcc -O0 "$td/acc_oracle.c" -o "$td/acc.ref" && ref=$("$td/acc.ref")
    "$CCC" -O2 "$td/acc_oracle.c" -o "$td/acc.got" && got=$("$td/acc.got")
    if [ "$ref" = "$got" ]; then echo "ok: _Atomic/volatile accumulate matches gcc -O0 ($got)"
    else echo "FAIL: _Atomic/volatile accumulate lccc='$got' oracle='$ref'"; rc=1; fi
fi
"$CCC" -O2 -S "$td/acc.c" -o "$td/acc.s" || { echo "FAIL: compile acc.c"; rc=1; }
for fn in atomic_acc vol_acc; do
    obj=$([ "$fn" = atomic_acc ] && echo atomic_sum || echo vol_sum)
    body=$(awk -v f="$fn" '$0==f":"{ins=1} ins{print} /^\.size/{if(ins)exit}' "$td/acc.s")
    if [ -z "$body" ]; then echo "FAIL: $fn not found"; rc=1; continue; fi
    # the loop region: from the backward branch's target to the branch itself
    tail_ln=$(echo "$body" | grep -nE '^[[:space:]]*j[a-z]+ +\.[A-Za-z]' | tail -1 | cut -d: -f1)
    tail_lbl=$(echo "$body" | sed -n "${tail_ln}p" | grep -oE '\.[A-Za-z][A-Za-z0-9_]*' | tail -1)
    head_ln=$(echo "$body" | grep -n "^${tail_lbl}:" | sed -n '1,1p' | cut -d: -f1)
    loop=$(echo "$body" | sed -n "${head_ln},${tail_ln}p")
    n_ref=$(echo "$loop" | grep -c "$obj" || true)
    case $n_ref in ''|*[!0-9]*) n_ref=0 ;; esac
    # Here-string, not `echo | grep -q`: under `set -o pipefail` the early-exiting
# consumer SIGPIPEs the producer and the pipeline reports failure even when the
# pattern matched (the pipefail-sigpipe gate exists for exactly this).
if [ "$n_ref" -ge 1 ] && grep -qE '^[[:space:]]*j[a-z]+ +\.' <<<"$loop"; then
        echo "ok: $fn keeps its per-iteration $obj access inside a real loop"
    else
        echo "FAIL: $fn collapsed to a closed form (refs=$n_ref)"; echo "      loop was: $loop"; rc=1
    fi
done

# 7. An extern declaration still describes a volatile object or pointee.
# Both file- and block-scope declarations used to lose their qualifier before
# IR construction, so even discarded reads disappeared at -O2. The symbol
# references count accesses in these simple straight-line functions; the
# pointer-target cases additionally count the two indirect loads.
cat >"$td/extern.c" <<'EOF'
extern volatile int g_flag;
extern int *volatile g_ptr;
extern volatile int *g_target;
extern volatile int g_arr[4];
struct regs { int x; };
extern volatile struct regs g_regs;
extern _Thread_local volatile int g_tls;
extern int plain;
int global_sum(void) { return g_flag + g_flag; }
void global_discard(void) { g_flag; g_flag; }
void global_stores(void) { g_flag = 1; g_flag = 2; }
void global_ptr(void) { g_ptr; g_ptr; }
void global_target(void) { *g_target; *g_target; }
void global_array(void) { g_arr[0]; g_arr[0]; }
void global_member(void) { g_regs.x; g_regs.x; }
void global_tls(void) { g_tls; g_tls; }
void block_discard(void) { extern volatile int b_flag; b_flag; b_flag; }
void block_ptr(void) { extern int *volatile b_ptr; b_ptr; b_ptr; }
void block_target(void) { extern volatile int *b_target; *b_target; *b_target; }
void block_tls(void) { extern _Thread_local volatile int b_tls; b_tls; b_tls; }
void plain_discard(void) { plain; plain; }
EOF
"$CCC" -O2 -S "$td/extern.c" -o "$td/extern.s" || { echo "FAIL: compile extern.c"; exit 1; }
for probe in global_sum:g_flag:2 global_discard:g_flag:2 global_stores:g_flag:2 \
             global_ptr:g_ptr:2 global_array:g_arr:2 global_member:g_regs:2 \
             block_discard:b_flag:2 block_ptr:b_ptr:2 plain_discard:plain:0; do
    IFS=: read -r fn symbol want <<<"$probe"
    body=$(awk -v f="$fn" '$0==f":"{ins=1} ins{print} ins && /^[[:space:]]*\.size/{exit}' "$td/extern.s")
    n=$(grep -cE "(^|[^[:alnum:]_])${symbol}\\(%rip\\)" <<<"$body" || true)
    if [[ -n $body && $n == "$want" ]]; then
        echo "ok: $fn keeps $want observable accesses"
    else
        echo "FAIL: $fn has $n references to $symbol, expected $want"; rc=1
    fi
done
for fn in global_target block_target; do
    body=$(awk -v f="$fn" '$0==f":"{ins=1} ins{print} ins && /^[[:space:]]*\.size/{exit}' "$td/extern.s")
    n=$(grep -cE '^[[:space:]]*mov[a-z]*[[:space:]]+\(%r[a-z0-9]+\),' <<<"$body" || true)
    if [[ -n $body && $n == 2 ]]; then
        echo "ok: $fn preserves both volatile pointee loads"
    else
        echo "FAIL: $fn has $n indirect loads, expected 2"; rc=1
    fi
done
for fn in global_tls block_tls; do
    body=$(awk -v f="$fn" '$0==f":"{ins=1} ins{print} ins && /^[[:space:]]*\.size/{exit}' "$td/extern.s")
    n=$(grep -cE '%fs:\(' <<<"$body" || true)
    if [[ -n $body && $n == 2 ]]; then
        echo "ok: $fn preserves both volatile TLS loads"
    else
        echo "FAIL: $fn has $n TLS loads, expected 2"; rc=1
    fi
done

# The same extern metadata carries the pointee's constness for `_Generic`.
# Both controls are unevaluated; they need no definitions of the externs.
cat >"$td/extern_generic.c" <<'EOF'
extern const int *g_const;
int global_type(void) { return _Generic(g_const, const int *: 1, int *: 0, default: 2); }
int block_type(void) { extern const int *b_const; return _Generic(b_const, const int *: 1, int *: 0, default: 2); }
int main(void) { return global_type() != 1 || block_type() != 1; }
EOF
if "$CCC" -O2 "$td/extern_generic.c" -o "$td/extern_generic" && "$td/extern_generic"; then
    echo "ok: extern pointee constness selects the correct _Generic arm"
else
    echo "FAIL: extern pointee constness was lost"; rc=1
fi

if ! python3 "$(dirname "$0")/check_typeof_scalar_volatile.py" --compiler "$CCC"; then
    rc=1
fi

i686_cc=${I686_CCC:-"$(dirname "$CCC")/lccc-i686"}
if [[ -x "$i686_cc" ]]; then
    if ! python3 "$(dirname "$0")/check_i686_volatile_access.py" --compiler "$i686_cc"; then
        rc=1
    fi
else
    echo "SKIP: i686 volatile codegen ($i686_cc not built)"
fi

riscv_cc=${CCC_RISCV:-"$(dirname "$CCC")/lccc-riscv"}
if [[ -x "$riscv_cc" ]]; then
    if ! python3 "$(dirname "$0")/check_riscv_volatile_access.py" --compiler "$riscv_cc"; then
        rc=1
    fi
else
    echo "SKIP: RISC-V volatile codegen ($riscv_cc not built)"
fi

exit $rc
