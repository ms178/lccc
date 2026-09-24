#!/usr/bin/env bash
# CH/MAJ codegen gate — pins the four-oracle consensus shapes for
# SHA-256's CH (mux) and MAJ (majority) under BMI1 and the register-only
# forms everywhere, plus the exposed-forward elimination the
# acc-resident consumption fixes.
#
# Checks:
#   A. runtime differential, bit-exact vs gcc over the opt x march matrix
#      (incl. both kill switches: CCC_NO_ANDN_FUSION, CCC_NO_BOOL_ALGEBRA)
#   B. the andn census, region-scoped to sha256_transform and
#      mnemonic-anchored (a substring grep over the whole TU would count
#      labels/symbols): exactly one 3-operand andn under -mbmi -O2 AND
#      under the DEFAULT baseline (no -march: the code-generation floor
#      is x86-64-v3, which grants BMI1); zero under the explicit
#      baseline (-march=x86-64) and under the explicit -mno-bmi denial
#   C. the 64-round compression loop is slot-write free (the park class)
#      and the MAJ region is register-only (no slot reads between the
#      kept And and its consumer)
#   D. zero exposed store-to-load forwards in the whole function at the
#      measured configurations (a slot read within 3 lines of its park)
set -u
CCC=${CCC:-target/fastbuild/lccc}
SRC=$(dirname "$0")/../benchmark/programs/sha256_transform.c
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
GCC=${GCC_BIN:-gcc}
GCCINC=$("$GCC" -print-file-name=include 2>/dev/null)
[ -d "$GCCINC" ] && GCCINC="-I$GCCINC" || GCCINC=""
fails=0
note() { printf '%s\n' "$*"; }
bad()  { note "FAIL: $*"; fails=$((fails+1)); }
# Correctness gate: a broken reference build must FAIL, not silently skip
# the differential (explicit opt-out only; CI never sets it).
ALLOW_GCC_SKIP=${LCCC_ALLOW_GCC_SKIP:-0}

# ── A. runtime differential ─────────────────────────────────────────
for MARCH in "" "-mbmi" "-march=x86-64-v3"; do
  for OPT in -O1 -O2 -O3 -Os; do
    TAG="$MARCH $OPT"
    "$GCC" $MARCH $OPT -o "$TMP/g.x" "$SRC" -lm 2>/dev/null \
      || { if [ "$ALLOW_GCC_SKIP" = 1 ]; then note "skip $TAG (gcc; LCCC_ALLOW_GCC_SKIP=1)"; else bad "$TAG gcc build failed (no silent skip: set LCCC_ALLOW_GCC_SKIP=1 to opt out)"; fi; continue; }
    "$CCC" $GCCINC $MARCH $OPT -o "$TMP/l.x" "$SRC" -lm 2>/dev/null \
      || { bad "$TAG lccc compile"; continue; }
    G=$("$TMP/g.x"; echo "rc=$?")
    L=$("$TMP/l.x"; echo "rc=$?")
    [ "$G" = "$L" ] || bad "$TAG runtime differs (gcc=[$G] lccc=[$L])"
  done
done
for OPT in -O2 -O3; do
  for VAR in CCC_NO_ANDN_FUSION CCC_NO_BOOL_ALGEBRA; do
    env "$VAR=1" "$CCC" $GCCINC -mbmi $OPT -o "$TMP/l.x" "$SRC" -lm 2>/dev/null \
      || { bad "$VAR $OPT compile"; continue; }
    L=$("$TMP/l.x"; echo "rc=$?")
    "$GCC" -march=x86-64-v3 $OPT -o "$TMP/g.x" "$SRC" -lm 2>/dev/null
    G=$("$TMP/g.x"; echo "rc=$?")
    [ "$G" = "$L" ] || bad "$VAR $OPT runtime differs"
  done
done
note "A. runtime differential done (fails so far: $fails)"

# ── B. andn census (sha256_transform region, mnemonic-anchored) ─────
# Counts ONLY `andn[lq]` mnemonics inside the sha256_transform function
# body: labels, symbol names and comments elsewhere in the TU can no
# longer inflate the census.
region_andn_count() {
  awk '/^sha256_transform:/{f=1; next} f && /^[A-Za-z_][A-Za-z0-9_]*:$/{f=0} f' "$1" \
    | grep -cE '^[[:space:]]*andn[lq]?[[:space:]]' || true
}
"$CCC" $GCCINC -mbmi -O2 -S "$SRC" -o "$TMP/bmi.s" 2>/dev/null
N=$(region_andn_count "$TMP/bmi.s")
[ "$N" -eq 1 ] || bad "-mbmi: expected exactly 1 andn in sha256_transform (CH), got $N"
# Default baseline: the project floor is x86-64-v3, so BMI1 (and the CH
# andn) is granted without any -march/-mbmi.
"$CCC" $GCCINC -O2 -S "$SRC" -o "$TMP/default.s" 2>/dev/null
N=$(region_andn_count "$TMP/default.s")
[ "$N" -eq 1 ] || bad "default (v3 baseline): expected exactly 1 andn in sha256_transform, got $N"
# Explicit baseline profile: the v3 grant is denied — zero andn.
"$CCC" $GCCINC -march=x86-64 -O2 -S "$SRC" -o "$TMP/base.s" 2>/dev/null
N=$(region_andn_count "$TMP/base.s")
[ "$N" -eq 0 ] || bad "-march=x86-64: expected 0 andn in sha256_transform, got $N"
# Explicit denial over the default grant: -mno-bmi wins.
"$CCC" $GCCINC -mno-bmi -O2 -S "$SRC" -o "$TMP/nobmi.s" 2>/dev/null
N=$(region_andn_count "$TMP/nobmi.s")
[ "$N" -eq 0 ] || bad "-mno-bmi: expected 0 andn in sha256_transform, got $N"

# ── C. compression-loop slot discipline ─────────────────────────────
python3 - "$TMP/bmi.s" <<'PYEOF'
import sys
lines = [l.strip() for l in open(sys.argv[1]) if l.strip()]
try:
    start = next(i for i, l in enumerate(lines)
                 if l.startswith("rorl $") or l.startswith("rorxl $"))
    end = next(i for i in range(start + 1, len(lines)) if lines[i].startswith("jl "))
except StopIteration:
    # Shape drift, NOT a contract break: the loop no longer has the
    # ror(l/x) ... jl skeleton this census is written against.
    print("codegen shape changed: expected compression loop shape not found")
    sys.exit(2)
loop = lines[start:end + 1]
def plain_slot_ref(opnd):
    # "N(%rsp)" with no index register inside the parentheses.
    return opnd.endswith("(%rsp)") and "," not in opnd
bad_lines = []
for l in loop:
    toks = l.split()
    if len(toks) == 3 and toks[0] in ("movl", "movq") and toks[1].startswith("%") \
       and plain_slot_ref(toks[2].rstrip(",")):
        bad_lines.append(("slot write", l))
    if len(toks) == 3 and toks[0] in ("andl", "andq", "xorl", "xorq", "orl", "orq") \
       and plain_slot_ref(toks[1].rstrip(",")):
        bad_lines.append(("slot-fold ALU", l))
if bad_lines:
    print(bad_lines[:3]); sys.exit(1)
PYEOF
rc_shape=$?
if [ "$rc_shape" -eq 1 ]; then
  bad "compression loop slot/MAJ shape"
elif [ "$rc_shape" -eq 2 ]; then
  bad "compression loop shape drifted (census needs re-anchoring, not a contract break)"
elif [ "$rc_shape" -ne 0 ]; then
  bad "compression loop census crashed (rc=$rc_shape)"
fi
note "B/C. shape census done (fails so far: $fails)"

# ── D. exposed store-to-load forwards ───────────────────────────────
for CFG in "-mbmi -O2" "-mbmi -O3" "-march=x86-64-v3 -O2" "-march=x86-64-v3 -O3"; do
  "$CCC" $GCCINC $CFG -S "$SRC" -o "$TMP/f.s" 2>/dev/null
  python3 - "$TMP/f.s" "$CFG" <<'PYEOF'
import sys
raw = [l.rstrip() for l in open(sys.argv[1])]
# Scope to sha256_transform itself: the benchmark driver's outer loop
# counter keeps one sound phi-copy forward by design (the MachInst
# window-defs gate); the compression function must have none.
try:
    f0 = next(i for i, l in enumerate(raw) if l.strip() == "sha256_transform:")
    f1 = next(i for i in range(f0 + 1, len(raw))
              if raw[i].strip().endswith(":") and raw[i].strip().startswith("main"))
except StopIteration:
    # A silent whole-file re-scope would mix the benchmark driver's
    # outer-loop phi-copy forward into the census and misreport; fail as
    # shape drift instead.
    print(f"{sys.argv[2]}: sha256_transform region not found — codegen shape changed")
    sys.exit(2)
lines = [l.strip() for l in raw[f0:f1] if l.strip()]
forwards = 0; stores = {}
def park(l):
    parts = l.split(", ")
    if len(parts) == 2 and parts[0].split(" ")[0] in ("movl", "movq") \
       and parts[0].split(" ")[1].startswith("%") \
       and parts[1].endswith("(%rsp)") and "," not in parts[1]:
        return parts[1][: parts[1].index("(%rsp)")]
    return None
for i, l in enumerate(lines):
    s = park(l)
    if s is not None:
        stores[s] = i; continue
    for slot, si in list(stores.items()):
        if ("(%s(%%rsp))" % slot).replace("%%", "%") in l.replace(", ", ",") \
           and 0 < i - si <= 3 and park(l) != slot + "(%rsp)":
            if (" " + slot + "(%rsp)") in l or ("," + slot + "(%rsp)") in l:
                forwards += 1
    for slot in list(stores):
        if (" " + slot + "(%rsp)") in l or ("," + slot + "(%rsp)") in l:
            if park(l) is None:
                del stores[slot]
if forwards:
    print(f"{sys.argv[2]}: {forwards} exposed store-to-load forwards"); sys.exit(1)
PYEOF
  rc_fwd=$?
  if [ "$rc_fwd" -eq 1 ]; then
    bad "exposed forwards under $CFG"
  elif [ "$rc_fwd" -eq 2 ]; then
    bad "exposed-forward census lost its function region under $CFG (shape drift)"
  elif [ "$rc_fwd" -ne 0 ]; then
    bad "exposed-forward census crashed under $CFG (rc=$rc_fwd)"
  fi
done

if [ "$fails" -eq 0 ]; then
  note "ch/maj codegen gate: PASS"
  exit 0
fi
note "ch/maj codegen gate: $fails FAILURES"
exit 1
