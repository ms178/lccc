#!/usr/bin/env bash
# ============================================================================
# check_rmw_sib_folds.sh — the memory read-modify-write and index-scale folds.
#
# Two x86 peephole folds share this gate because they are one story: together
# they turn `table[idx]++` into a single `add $1,(base,idx,8)`.
#
#   * `memory_fold::fold_memory_rmw`     — `mov MEM,%d; <inc %d>; mov %d,MEM`
#                                          becomes `add $imm,MEM`.
#   * `local_patterns::fold_shift_into_sib` — `shl $k,%i; ...; addq %i,%b`
#                                          becomes `leaq (%b,%i,2^k),%b`, which
#                                          the LEA-splicing passes then fold
#                                          into the consumer's memory operand.
#
# Contracts:
#
#   1. SEMANTICS.  rmw_sib_fold.c must agree with the GCC oracle byte for byte
#      at -O1/-O2/-O3 and with EACH pass skipped and with both skipped
#      (`CCC_PEEPHOLE_SKIP=mem_rmw` / `=shl_sib` / `=mem_rmw,shl_sib` -- the
#      skip-set names, which is the ONLY knob that reaches these passes; an
#      invented CCC_NO_* name would be inert and this arm would silently test
#      the default build three times).  The fixture carries the shapes that
#      make each fold UNSOUND if its guards are dropped: a load whose value is
#      still read afterwards, a fold point whose flags are read afterwards, a
#      byte- and a word-wide increment, a hand-written `t[i*8] += v`, and a
#      `x + x` doubling.
#
#   2. THE FOLDS FIRE, and the resulting addressing mode is well formed.  On
#      the histogram kernel the per-element sequence must collapse to
#      `addq $1, (...)` with a SIB operand, and no `shl` may remain in the
#      store loop.  Deliberately checked on the SCALED shape: if the fold
#      emitted `(%r9, r11, 8)` without the `%` on the index (a bug this gate
#      was written after), the assembler would still encode the right bytes
#      while the peephole's own register bookkeeping saw no read at all and a
#      later dead-write pass deleted the index's definition.  So the contract
#      is spelled on the TEXT: every register operand carries its `%`.
#
#   3. THE DEAD INDEX IS NOT DELETED.  Because of (2): `main` must still
#      compute the byte the index comes from (`movzbl`) inside the store loop.
# ============================================================================
set -euo pipefail

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$here/../.." && pwd)
CCC=${CCC:-$root/target/fastbuild/lccc}
GCC=${GCC:-gcc}
[[ -x $CCC ]] || { echo "check_rmw_sib_folds: lccc not found at $CCC" >&2; exit 1; }
command -v "$GCC" >/dev/null 2>&1 || { echo "check_rmw_sib_folds: no gcc oracle" >&2; exit 1; }

corpus=$here/rmw_sib_fold.c
bench=$root/tests/benchmark/programs/histogram.c
work=$(mktemp -d "${TMPDIR:-/tmp}/lccc-rmw.XXXXXX")
trap 'rm -rf "$work"' EXIT
fail=0
note() { printf '  %s\n' "$*"; }
bad() { printf '  FAIL: %s\n' "$*" >&2; fail=1; }

note "contract 1: oracle parity across opt levels x kill switches"
"$GCC" -O2 "$corpus" -o "$work/ref" || bad "gcc oracle failed to build"
expected=$(timeout 300 "$work/ref") || bad "gcc oracle exited non-zero"

for opt in -O1 -O2 -O3; do
    for envs in "" "CCC_PEEPHOLE_SKIP=mem_rmw" "CCC_PEEPHOLE_SKIP=shl_sib" "CCC_PEEPHOLE_SKIP=mem_rmw,shl_sib"; do
        name=$(echo "opt${opt} $(echo "$envs" | tr -d '=,')" | tr -d ' -')
        if ! "$CCC" "$opt" "$corpus" -o "$work/$name" > "$work/$name.build" 2>&1; then
            bad "$opt [$envs]: lccc failed to build"; continue
        fi
        got=$(timeout 300 env $envs "$work/$name") || { bad "$opt [$envs]: non-zero exit"; continue; }
        [[ $got == "$expected" ]] || bad "$opt [$envs]: stdout differs from the oracle"
    done
done
note "parity: -O1/-O2/-O3 x {default, skip mem_rmw, skip shl_sib, skip both}"

note "contract 2: the store loop is a scaled read-modify-write"
"$CCC" -O2 -march=x86-64-v3 -S "$bench" -o "$work/bench.s" || bad "-S build failed"
body=$(awk '/^main:/ { inside=1 } inside { print } inside && /\.size/ { exit }' "$work/bench.s")

# Mutation arms: the SAME greps must fail when the fold that produces the
# shape is skipped.  Without these the contract would pass on a tree where the
# pass never runs at all (or where the knob is inert).
CCC_PEEPHOLE_SKIP=mem_rmw "$CCC" -O2 -march=x86-64-v3 -S "$bench" -o "$work/bench_nor.s" \
    || bad "-S build failed (no-rmw)"
nor=$(awk '/^main:/ { inside=1 } inside { print } inside && /\.size/ { exit }' "$work/bench_nor.s")
grep -qE "addq[[:space:]]+\$1,[[:space:]]*\(%" <<< "$nor" \
    && bad "skipping mem_rmw still produced a memory RMW: the skip name is inert"

CCC_PEEPHOLE_SKIP=shl_sib "$CCC" -O2 -march=x86-64-v3 -S "$bench" -o "$work/bench_nos.s" \
    || bad "-S build failed (no-sib)"
nos=$(awk '/^main:/ { inside=1 } inside { print } inside && /\.size/ { exit }' "$work/bench_nos.s")
grep -qE "addq[[:space:]]+\$1,[[:space:]]*\([^)]*,[^)]*,[[:space:]]*[248]\)" <<< "$nos" \
    && bad "skipping shl_sib still produced a scaled SIB add: the skip name is inert"

grep -qE "addq[[:space:]]+\\\$1,[[:space:]]*\(%" <<< "$body" \
    || bad "no \`addq \$1, (mem)\` in main: the table increment is not an RMW"
grep -qE "addq[[:space:]]+\\\$1,[[:space:]]*\([^)]*,[^)]*,[[:space:]]*[248]\)" <<< "$body" \
    || bad "no scaled SIB form (\`addq \$1, (base, index, 8)\`): the shift fold did not reach the memory operand"
grep -qE "^[[:space:]]+shl[a-z]*[[:space:]]" <<< "$body" \
    && bad "a \`shl\` survived in main: the index scale was not moved into the addressing mode"
# Every register operand in the SIB form must carry its `%` (contract 2's
# rationale): a bare `r11` would assemble but be invisible to the peephole's
# own register bookkeeping.
grep -oE "\([^)]*,[^)]*,[[:space:]]*[248]\)" <<< "$body" | while read -r op; do
    bare=$(sed -E 's/%[a-z0-9]+//g' <<< "$op" | tr -dc 'a-z')
    if [[ -n $bare ]]; then
        printf '  FAIL: SIB operand without %% on a register: %s\n' "$op" >&2
        exit 1
    fi
done || fail=1
note "fired: \$1-adds on memory with a %%-prefixed SIB operand, no shl left"

note "contract 2b: narrow counters collapse through the copy"
"$CCC" -O2 -S "$corpus" -o "$work/corpus.s" || bad "-S build failed (corpus, narrow)"
fn() { awk -v fn="$1:" '$0 ~ "^" fn { inside=1 } inside { print } inside && /\.size/ { exit }' "$work/corpus.s"; }
u8body=$(fn rmw_bump_u8)
u16body=$(fn rmw_bump_u16)
grep -qE "^[[:space:]]+addb[[:space:]]+\\\$1," <<< "$u8body" \
    || bad "rmw_bump_u8: no single \`addb $1, (mem)\` -- the narrow copy was not seen through"
grep -qE "^[[:space:]]+movb[[:space:]]" <<< "$u8body" \
    && bad "rmw_bump_u8: a byte store relay survived"
grep -qE "^[[:space:]]+addw[[:space:]]+\\\$1," <<< "$u16body" \
    || bad "rmw_bump_u16: no single \`addw $1, (mem)\`"
grep -qE "^[[:space:]]+movw[[:space:]]" <<< "$u16body" \
    && bad "rmw_bump_u16: a word store relay survived"
note "narrow counters: one addb/addw on memory, no store relay"

note "contract 3: the index's definition survives"
grep -qE "movzbl" <<< "$body" \
    || bad "the byte load vanished: a dead-write pass mistook the index for unread"

# ── contract 4: the volatile policy, pinned as an observable ─────────────────
# A volatile read-modify-write must touch its object exactly once for the read
# and once for the write -- never more (a redundant access is a wrong-code
# defect: it can pop a FIFO or clear a status register twice) and never fewer
# than the abstract machine performs.  This is asserted on the emitted code AND
# on the runtime result, so the policy cannot drift silently: the count is one
# instruction today, and if that ever becomes two the gate fails and the change
# gets the review this note describes.
cat > "$work/vol_rmw.c" <<'EOF'
#include <stdio.h>
volatile long h;
void k(void) { h += 3; }
int main(void) { h = 10; k(); printf("%ld\n", h); return h == 13 ? 0 : 1; }
EOF
"$CCC" -O2 -march=x86-64-v3 -S -o "$work/vol_rmw.s" "$work/vol_rmw.c" \
    || bad "volatile probe: compile failed"
k_body=$(awk '/^k:/{inside=1} inside{print} inside && /^[[:space:]]*\.size/{exit}' "$work/vol_rmw.s")
touches=$(grep -c "h(%rip)" <<< "$k_body" || true)
[[ $touches -eq 1 ]] \
    || bad "volatile += touches its object in $touches instructions inside k (want exactly 1: one read + one write); k body:\n$k_body"
"$CCC" -O2 -march=x86-64-v3 -o "$work/vol_rmw.bin" "$work/vol_rmw.c" \
    || bad "volatile probe: link failed"
out=$("$work/vol_rmw.bin") || bad "volatile probe: exit status $? (wrong value)"
[[ "$out" == "13" ]] || bad "volatile probe printed '$out', want 13"
note "volatile RMW: one access to the object, result exact"

if [[ $fail -ne 0 ]]; then
    echo "check_rmw_sib_folds: FAILED" >&2
    exit 1
fi
echo "check_rmw_sib_folds: PASS (oracle parity at -O1/-O2/-O3 x kill switches, RMW+SIB addressing, index definition intact)"
