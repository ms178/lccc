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
exit 0
