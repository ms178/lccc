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
echo "== use =="
$CCC -O2 -fprofile-use="$PGD" "$SRC" -o /tmp/r7_pu 2>/tmp/r7_pu.log || { echo "USE-FAIL"; exit 1; }
/tmp/r7_pu > /tmp/r7_pu.out || { echo "USE-RUN-FAIL"; exit 1; }
cmp -s /tmp/r7_pg.out /tmp/r7_pu.out || { echo "OUTPUT-DIFFERS"; exit 1; }
NPRO=$(grep -c "promoted" /tmp/r7_pu.log)
echo "roundtrip OK (promoted sites: $NPRO)"
[ "$NPRO" -gt 0 ] || { echo "NO-PROMOTION"; exit 1; }
exit 0
