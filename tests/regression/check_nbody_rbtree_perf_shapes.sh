#!/usr/bin/env bash
# Worst-15 follow-up codegen gate — pins the three load-class wins of the
# session586 perf campaign so they cannot silently regress:
#
#   A. nbody pair-loop load CSE (alias: copy-web leaves + stride-period
#      field disjointness + VecStore modeling): exactly TWO mass loads
#      (48-offset fields) in the whole inlined advance — one per body of
#      the pair.  Pre-fix, each body's mass was re-loaded for the scalar
#      vz tail after the packed vx/vy stores (4+ loads).
#   B. same-value FP squares: no `movsd SLOT, %rX` immediately followed by
#      `vmulsd SLOT, %rX, %rX` reading the SAME slot twice (the register
#      already holds it — nbody's d² chain shape).
#   C. linux_rbtree's lookup hash: `i * 104729` is strength-reduced to the
#      derived recurrence (addl), never a per-iteration imull.
#
# Runtime differentials for both programs are the benchmark-output gate's
# job (204 cases); this gate pins the CODEGEN shapes.
set -u
CCC=${CCC:-target/fastbuild/lccc}
NB=$(dirname "$0")/../benchmark/programs/nbody.c
RB=$(dirname "$0")/../benchmark/programs/linux_rbtree.c
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
fails=0
note() { printf '%s\n' "$*"; }
bad()  { note "FAIL: $*"; fails=$((fails+1)); }

# ── A + B: nbody ────────────────────────────────────────────────────
"$CCC" -O2 -S "$NB" -o "$TMP/nb.s" 2>/dev/null || { echo "FAIL: nbody compile"; exit 1; }
N=$(grep -cE 'movsd +48\(' "$TMP/nb.s" || true)
[ "$N" -eq 2 ] || bad "nbody: expected exactly 2 mass loads (pair-loop CSE), got $N"

# Same-value double-read: `movsd SLOT, %rX` then `vmulsd SLOT, %rX, %rX`
# (or the fma spelling) with the SAME slot — python for the two-line window.
D=$(python3 - "$TMP/nb.s" <<'EOF'
import re, sys
lines = [l.strip() for l in open(sys.argv[1]) if l.strip()]
hits = 0
for i in range(len(lines) - 1):
    m = re.match(r'^movsd +([-0-9]+\(.*\)), +%xmm(\d+)$', lines[i])
    if not m:
        continue
    slot, reg = m.group(1), m.group(2)
    if re.match(r'^vmulsd +' + re.escape(slot) + r', +%xmm' + reg + r', +%xmm' + reg + r'$', lines[i+1]):
        hits += 1
    if re.match(r'^vfmadd231sd +' + re.escape(slot) + r', +%xmm' + reg + r', +%xmm(\d+)$', lines[i+1]):
        hits += 1
print(hits)
EOF
)
[ "$D" -eq 0 ] || bad "nbody: $D same-value squares re-read the slot (register already holds it)"

# ── C: rbtree hash strength reduction ───────────────────────────────
"$CCC" -O2 -S "$RB" -o "$TMP/rb.s" 2>/dev/null || { echo "FAIL: rbtree compile"; exit 1; }
I=$(grep -cE 'imull +\$104729' "$TMP/rb.s" || true)
[ "$I" -eq 0 ] || bad "rbtree: $D per-iteration imull \$104729 (derived IV regressed)"
A=$(grep -cE 'addl +\$104729' "$TMP/rb.s" || true)
[ "$A" -ge 1 ] || bad "rbtree: no addl \$104729 recurrence (derived IV missing)"

if [ "$fails" -eq 0 ]; then
  note "nbody/rbtree perf shapes: PASS"
else
  note "nbody/rbtree perf shapes: $fails failure(s)"
  exit 1
fi
