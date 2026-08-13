#!/usr/bin/env bash
# v7 PGO round-trip regression: generate -> train -> use, verify identical
# behavior and that devirtualization actually fired.
set -u
CCC=${CCC:-./target/release/lccc}
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SRC="$DIR/regr_v7_value_prof.c"
PGD=/tmp/pgd_roundtrip
rm -rf "$PGD" && mkdir -p "$PGD"
echo "== generate =="
$CCC -O2 -fprofile-generate="$PGD" "$SRC" -o /tmp/r7_pg || { echo "GEN-FAIL"; exit 1; }
/tmp/r7_pg > /tmp/r7_pg.out || { echo "GEN-RUN-FAIL"; exit 1; }
grep -q "^ok " /tmp/r7_pg.out || { echo "GEN-OUT-FAIL: $(cat /tmp/r7_pg.out)"; exit 1; }
[ -n "$(ls "$PGD"/*.profraw 2>/dev/null)" ] || { echo "NO-PROFILE"; exit 1; }
echo "== env override =="
rm -f /tmp/r7_custom.profraw
LCCC_PROFILE_FILE=/tmp/r7_custom.profraw /tmp/r7_pg > /dev/null || { echo "ENV-RUN-FAIL"; exit 1; }
[ -s /tmp/r7_custom.profraw ] || { echo "ENV-NO-FILE"; exit 1; }
echo "== switch lowering =="
$CCC -O2 -fprofile-generate="$PGD" "$DIR/regr_v8_switch_table.c" -o /tmp/r8_pg 2>/dev/null || { echo "SW-GEN-FAIL"; exit 1; }
/tmp/r8_pg > /dev/null || { echo "SW-TRAIN-FAIL"; exit 1; }
$CCC -O2 -fprofile-use="$PGD" "$DIR/regr_v8_switch_table.c" -o /tmp/r8_pu 2>/dev/null || { echo "SW-USE-FAIL"; exit 1; }
/tmp/r8_pu > /dev/null || { echo "SW-RUN-FAIL"; exit 1; }
$CCC -O2 -fprofile-use="$PGD" -S "$DIR/regr_v8_switch_table.c" -o /tmp/r8_pu.s 2>/dev/null || { echo "SW-ASM-FAIL"; exit 1; }
NJT=$(grep -c "\\.long .*\\.LBB" /tmp/r8_pu.s)
echo "jump tables emitted: $NJT"
[ "$NJT" -gt 0 ] || { echo "NO-JUMP-TABLE"; exit 1; }
echo "== use =="
$CCC -O2 -fprofile-use="$PGD" "$SRC" -o /tmp/r7_pu 2>/tmp/r7_pu.log || { echo "USE-FAIL"; exit 1; }
/tmp/r7_pu > /tmp/r7_pu.out || { echo "USE-RUN-FAIL"; exit 1; }
cmp -s /tmp/r7_pg.out /tmp/r7_pu.out || { echo "OUTPUT-DIFFERS"; exit 1; }
NPRO=$(grep -c "promoted" /tmp/r7_pu.log)
echo "roundtrip OK (promoted sites: $NPRO)"
[ "$NPRO" -gt 0 ] || { echo "NO-PROMOTION"; exit 1; }

# ── v10: flat-profile gate ───────────────────────────────────────────────
# A workload whose profile has a TIED hot function set (no dominant hot path)
# must not have its hot functions bloated by profile-driven inlining. The
# has_spread() dominance gate should make the -fprofile-use build match plain
# structurally for the hot functions.
FLAT="$DIR/regr_v10_pgo_flat.c"
rm -rf /tmp/pgd_flat && mkdir -p /tmp/pgd_flat
$CCC -O2 -fprofile-generate=/tmp/pgd_flat "$FLAT" -o /tmp/flat_pg 2>/dev/null || { echo "FLAT-GEN-FAIL"; exit 1; }
/tmp/flat_pg > /tmp/flat_pg.out || { echo "FLAT-TRAIN-FAIL"; exit 1; }
grep -q "^ok " /tmp/flat_pg.out || { echo "FLAT-OUT-FAIL"; exit 1; }
# plain assembly reference
$CCC -O2 "$FLAT" -S -o /tmp/flat_plain.s 2>/dev/null || { echo "FLAT-PLAIN-ASM-FAIL"; exit 1; }
# use build must round-trip identically and not bloat hot functions
$CCC -O2 -fprofile-use=/tmp/pgd_flat "$FLAT" -o /tmp/flat_pu 2>/dev/null || { echo "FLAT-USE-FAIL"; exit 1; }
/tmp/flat_pu > /tmp/flat_pu.out || { echo "FLAT-USE-RUN-FAIL"; exit 1; }
cmp -s /tmp/flat_pg.out /tmp/flat_pu.out || { echo "FLAT-OUTPUT-DIFFERS"; exit 1; }
$CCC -O2 -fprofile-use=/tmp/pgd_flat "$FLAT" -S -o /tmp/flat_use.s 2>/dev/null
instr() { awk "/^${1}:/,/^\.size ${1}/" "${2}" | grep -cE "^[[:space:]]+[a-z]"; }
for fn in step_even step_odd; do
  P=$(instr "$fn" /tmp/flat_plain.s)
  U=$(instr "$fn" /tmp/flat_use.s)
  echo "flat-profile ${fn}: plain ${P} insns, use ${U} insns"
  # flat profile -> gate off -> hot functions should match plain closely;
  # allow +40% for benign scheduling/encoding variance, flag real bloat.
  if [ "$U" -gt $(( (P * 14) / 10 )) ]; then
    echo "FLAT-BLOAT (${fn}: ${P} -> ${U} instructions)"; exit 1;
  fi
done
echo "flat-profile roundtrip OK (no hot-function bloat)"

# ── v11: cost-aware devirtualization ────────────────────────────────────
# A SINGLE, stable, loop-invariant indirect target is already predicted
# perfectly by the BTB; devirtualizing it would only add a per-call compare +
# branch (a regression). The cost-aware rule (top-share >= 95% -> skip) must
# therefore NOT promote this site.
SINGLE="$DIR/regr_v11_costaware_devirt.c"
rm -rf /tmp/pgd_single && mkdir -p /tmp/pgd_single
$CCC -O2 -fprofile-generate=/tmp/pgd_single "$SINGLE" -o /tmp/single_pg 2>/dev/null || { echo "SINGLE-GEN-FAIL"; exit 1; }
/tmp/single_pg > /tmp/single_pg.out || { echo "SINGLE-TRAIN-FAIL"; exit 1; }
grep -q "^ok " /tmp/single_pg.out || { echo "SINGLE-OUT-FAIL"; exit 1; }
$CCC -O2 -fprofile-use=/tmp/pgd_single "$SINGLE" -o /tmp/single_pu 2>/tmp/single_pu.log || { echo "SINGLE-USE-FAIL"; exit 1; }
/tmp/single_pu > /tmp/single_pu.out || { echo "SINGLE-USE-RUN-FAIL"; exit 1; }
cmp -s /tmp/single_pg.out /tmp/single_pu.out || { echo "SINGLE-OUTPUT-DIFFERS"; exit 1; }
NPROM=$(grep -c "promoted" /tmp/single_pu.log)
echo "cost-aware devirt: promoted sites = ${NPROM}"
# A single-stable-target site must NOT be promoted (would add hot-path overhead).
[ "$NPROM" -gt 0 ] && { echo "SINGLE-TARGET-PROMOTED-REGRESSION"; exit 1; }
echo "cost-aware devirtualization roundtrip OK (single target not promoted)"
exit 0
