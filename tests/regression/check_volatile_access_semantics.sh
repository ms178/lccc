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
    if echo "$body" | grep -Eq "$pat"; then
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
        head_ln=$(echo "$fn_body" | grep -n "^${tail_lbl}:" | head -1 | cut -d: -f1)
        loop=$(echo "$fn_body" | sed -n "${head_ln},${tail_ln}p")
        if echo "$loop" | grep -q 'counter'; then
            echo "ok: loop keeps the volatile read in the body"
        else
            echo "FAIL: volatile read hoisted out of the loop body"
            echo "      loop was: $loop"
            rc=1
        fi
        # NEGATIVE CONTROL: the loop must still be a loop. If the read
        # vanished because the whole loop was optimised away, the check
        # above would pass for the wrong reason.
        if ! echo "$loop" | grep -qE '^[[:space:]]*j[a-z]+ +\.[A-Za-z]'; then
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

exit $rc
