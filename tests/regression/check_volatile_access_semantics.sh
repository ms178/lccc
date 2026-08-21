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
set -uo pipefail

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
    loads=$(echo "$body" | grep -Ec "$load_pat")
    if [ "$loads" -ge 1 ]; then echo "ok: store-then-load reloads memory"; else echo "FAIL: volatile load forwarded/eliminated"; rc=1; fi
    # 2. the loop body must contain a load
    body=$(awk '/^reads_in_loop:/,/^\.size/' "$td/vol.s" | awk '/\.LBB[0-9]*:/{blk=blk+1} {print}' )
    check reads_in_loop "$load_pat" "loop keeps volatile load in body"
    # 3. dead read survives
    check dead_read "$load_pat" "dead-result volatile load survives DCE"
    # 4. deref through pointer param loads
    check deref_param "$load_pat" "*volatile-ptr param loads"
    # 5. volatile local: store;load;store sequence
    body=$(awk '/^volatile_local:/,/^\.size/' "$td/vol.s")
    n=$(echo "$body" | grep -Ec 'mov[a-z]* +[^#]*\(%(rsp|rbp|esp|ebp)')
    if [ "$n" -ge 3 ]; then echo "ok: volatile local keeps RMW"; else echo "FAIL: volatile local promoted (mem access count $n < 3)"; rc=1; fi
done

exit $rc
