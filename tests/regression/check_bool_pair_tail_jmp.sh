#!/usr/bin/env bash
# ============================================================================
# check_bool_pair_tail_jmp.sh — pin the fused AND-of-compares chain's
# fall-through contract.
# (Kept name: the gate predates the De Morgan split that superseded the
# bool-pair fusion; the contract it pins is unchanged.)
#
# The De Morgan split re-emits `cmp1; jcc(inv1) cold; cmp2; <jcc2>` at the
# branch. Control flow after jcc2 depends on layout:
#   - hot successor physically next:  jcc2 -> cold; fall into hot;
#   - cold successor physically next: jcc2 -> hot;  fall into cold;
#   - NEITHER successor next:         jcc2 -> hot; `jmp cold` is MANDATORY.
# The third shape once fell through into an unrelated block (real control
# flow corruption — this arm was the only branch emitter without the tail
# jump guard). This script pins, for every fused chain in the emitted asm,
# that the chain's fall-through is one of its own branch targets:
#   * first jcc target == second jcc target (hot-next shape): sound by
#     construction — both failure edges agree;
#   * targets differ: the next label must be the FIRST jcc's target (the
#     cold edge) or the chain must be followed by an explicit `jmp` to it.
# It also runs each input differentially (lccc vs gcc output).
# ============================================================================
set -euo pipefail

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
CCC=${CCC:-$here/../../target/fastbuild/lccc}
[[ -x $CCC ]] || { echo "check_bool_pair_tail_jmp: lccc not found at $CCC" >&2; exit 1; }

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

fail=0

# AND-of-compares chains in varied layouts: loop guards, if/else joins,
# sequences (a later if creates a fresh block after the fused branch).
cat > "$work/chains.c" <<'EOF'
#include <stdio.h>
unsigned char D[4096];
unsigned N;


static unsigned long run(unsigned seed) {
    unsigned long s = 0;
    for (unsigned i = 0; i < N; i++) {
        unsigned v = seed + i;
        /* The lz4_compress fill-loop shape: two pure compares ANDed into a
         * branch — the exact pattern the De Morgan split fires on. */
        if ((v & 15u) < 6u && i >= 128u) {
            s += (unsigned)D[i];
        } else {
            s += 1;
        }
    }
    return s;
}

int main(void) {
    unsigned ns[4] = {100, 1000, 2000, 31};
    unsigned long acc = 0;
    for (int k = 0; k < 4; k++) { N = ns[k]; acc += run((unsigned)(k * 37 + 1)); }
    printf("%lu\n", acc);
    return 0;
}
EOF

# --- differential correctness first ---
if ! gcc -O2 "$work/chains.c" -o "$work/chains.gcc" 2>"$work/cc.err"; then
    echo "FAIL: gcc reference compile"; head -5 "$work/cc.err"; exit 1
fi
if ! "$CCC" -O2 "$work/chains.c" -o "$work/chains.lccc" 2>"$work/cl.err"; then
    echo "FAIL: lccc compile"; head -5 "$work/cl.err"; exit 1
fi
"$work/chains.gcc" > "$work/out.gcc"
"$work/chains.lccc" > "$work/out.lccc"
if ! cmp -s "$work/out.gcc" "$work/out.lccc"; then
    echo "FAIL: differential output mismatch (lccc vs gcc)"
    diff "$work/out.gcc" "$work/out.lccc" | head -5
    fail=1
else
    echo "ok(differential): outputs identical"
fi

# --- structural invariant over the fused chains ---
"$CCC" -O2 -S -o "$work/chains.s" "$work/chains.c"
python3 - "$work/chains.s" <<'PYEOF'
import re, sys

jcc = re.compile(r"^[ \t]*(j[a-z]{2,3})[ \t]+(\.L\S+)$")
lbl = re.compile(r"^[ \t]*(\.L\S+):")

lines = open(sys.argv[1]).read().splitlines()
chains = 0
bad = 0
for i, l in enumerate(lines):
    m2 = jcc.match(l)
    if not m2:
        continue
    # fused chain: [jcc] (up to 2 insns, exactly one of them a cmp) [jcc]
    m1 = None
    for back in (2, 3, 4):
        if i < back:
            break
        between = lines[i - back + 1 : i]
        if any(lbl.match(x) or jcc.match(x) for x in between):
            break
        cmps = [x for x in between if re.match(r"^[ \t]*cmp", x)]
        c1 = jcc.match(lines[i - back])
        if c1 and len(cmps) == 1 and len(between) == 1:
            m1 = c1
            break
    if m1 is None:
        continue
    a, b = m1.group(2), m2.group(2)
    chains += 1
    if a == b:
        continue  # hot-next shape: both failure edges agree
    # else-shape: fall-through must be the first target, or an explicit jmp
    nxt = None
    jmp_after = None
    for j in range(i + 1, min(i + 6, len(lines))):
        lm = lbl.match(lines[j])
        if lm:
            nxt = lm.group(1)
            break
        jm = re.match(r"^[ \t]*jmp[ \t]+(\.L\S+)$", lines[j])
        if jm:
            jmp_after = jm.group(1)
            break
    if nxt != a and jmp_after != a:
        bad += 1
        print(
            f"FAIL(chain@line {i+1}): jcc1->{a} jcc2->{b} "
            f"fall-through={nxt} jmp={jmp_after} (neither is the cold target)"
        )
print(f"fused chains: {chains}, violations: {bad}")
if chains == 0:
    print("FAIL: no fused chain in the emitted asm - the test no longer exercises the fusion")
    sys.exit(1)
sys.exit(1 if bad else 0)
PYEOF
if [[ $? != 0 ]]; then fail=1; fi

if [[ $fail != 0 ]]; then echo "check_bool_pair_tail_jmp: FAILED"; exit 1; fi
echo "check_bool_pair_tail_jmp: contract holds"
