#!/usr/bin/env bash
# Worst-15 follow-up codegen gate — pins the load-class wins of the
# session586 perf campaign so they cannot silently regress:
#
#   A. nbody pair-loop load CSE (alias: copy-web leaves + stride-period
#      field disjointness + VecStore modeling): at most TWO mass loads
#      (48-offset fields) in the whole inlined advance — one per body of
#      the pair.  Pre-fix, each body's mass was re-loaded for the scalar
#      vz tail after the packed vx/vy stores (4+ loads, ALL in
#      `movsd 48(...)` displacement form).  Session 586 pinned exactly 2;
#      the IVSR marching-pointer recurrence (2026-10-12) improved one of
#      the two to a hoisted field-pointer form (`leaq 48(ptr)` once per
#      i-iteration, then `movsd (reg)`), so the displacement-form count
#      dropped to 1.  The UPPER BOUND is the regression class: any return
#      of the vz-tail reloads re-introduces displacement-form mass loads
#      and trips it.  The deep pair-loop shape pin (zero stride
#      multiplies, marching pointers) lives in
#      check_ivsr_vecaddr_struct_walk.sh.
#   B. same-value FP squares: no `movsd SLOT, %rX` immediately followed by
#      `vmulsd SLOT, %rX, %rX` reading the SAME slot twice (the register
#      already holds it — nbody's d² chain shape).
#
# REMOVED (2026-09-23, the S46 CI RED post-mortem): the rbtree
# derived-IV recurrence check.  The scalar derived-IV flavor was measured
# as a net LOSS (linux_rbtree runtime +1.2%, expat +4%, glibc_strstr +4.3%,
# and 5 golden workloads past the codegen-quality gate's bands) and is now
# opt-in via CCC_IVSR_SCALAR_DERIVED=1 — see iv_strength_reduce.rs for the
# data.  Pinning its asm shape here would pin a measured regression; the
# knob's own unit tests keep the machinery honest.
#
# Runtime differentials are the benchmark-output gate's job (204 cases);
# this gate pins the CODEGEN shapes.
set -u
CCC=${CCC:-target/fastbuild/lccc}
NB=$(dirname "$0")/../benchmark/programs/nbody.c
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
fails=0
note() { printf '%s\n' "$*"; }
bad()  { note "FAIL: $*"; fails=$((fails+1)); }

# ── A + B: nbody ────────────────────────────────────────────────────
"$CCC" -O2 -S "$NB" -o "$TMP/nb.s" 2>/dev/null || { echo "FAIL: nbody compile"; exit 1; }
N=$(grep -cE 'movsd +48\(' "$TMP/nb.s" || true)
[ "$N" -le 2 ] || bad "nbody: expected at most 2 displacement-form mass loads (pair-loop CSE), got $N"

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

if [ "$fails" -eq 0 ]; then
  note "nbody perf shapes: PASS"
else
  note "nbody perf shapes: $fails failure(s)"
  exit 1
fi
