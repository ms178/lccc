#!/usr/bin/env bash
# Worst-15 follow-up codegen gate — pins the load-class wins of the
# session586 perf campaign so they cannot silently regress:
#
#   A. nbody pair-loop load CSE: at most two proven mass-read operands
#      in the actual natural velocity-update loop. Track Body+48 through
#      hoisted pointers/copies/frame spills and count folded operands too.
#      Missing workloads, unknown addresses and ambiguous CFG facts fail;
#      zero displacement-form matches cannot manufacture a pass.
#      This is a workload-specific codegen contract, not a runtime metric.
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
python3 "$(dirname "$0")/../../scripts/nbody_mass_loads.py" "$TMP/nb.s" \
    || bad "nbody: semantic mass-read proof failed"

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
