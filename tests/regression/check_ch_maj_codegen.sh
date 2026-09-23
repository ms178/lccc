#!/usr/bin/env bash
# CH/MAJ codegen gate — pins the four-oracle consensus shapes for
# SHA-256's CH (mux) and MAJ (majority) under BMI1 and the register-only
# forms everywhere, plus the exposed-forward elimination the
# acc-resident consumption fixes.
#
# Checks:
#   A. runtime differential, bit-exact vs gcc over the opt x march matrix
#      (incl. both kill switches: CCC_NO_ANDN_FUSION, CCC_NO_BOOL_ALGEBRA)
#   B. under -mbmi -O2: exactly one 3-operand andn in sha256_transform
#      (CH = the GCC/Clang/ICX shape), zero andn without BMI
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

# ── A. runtime differential ─────────────────────────────────────────
for MARCH in "" "-mbmi" "-march=x86-64-v3"; do
  for OPT in -O1 -O2 -O3 -Os; do
    TAG="$MARCH $OPT"
    "$GCC" $MARCH $OPT -o "$TMP/g.x" "$SRC" -lm 2>/dev/null \
      || { note "skip $TAG (gcc)"; continue; }
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

# ── B. andn census ──────────────────────────────────────────────────
"$CCC" $GCCINC -mbmi -O2 -S "$SRC" -o "$TMP/bmi.s" 2>/dev/null
N=$(grep -c 'andn' "$TMP/bmi.s" || true)
[ "$N" -eq 1 ] || bad "-mbmi: expected exactly 1 andn (CH), got $N"
"$CCC" $GCCINC -O2 -S "$SRC" -o "$TMP/base.s" 2>/dev/null
N=$(grep -c 'andn' "$TMP/base.s" || true)
[ "$N" -eq 0 ] || bad "baseline: expected 0 andn, got $N"

# ── C. compression-loop slot discipline ─────────────────────────────
python3 - "$TMP/bmi.s" <<'PYEOF' || bad "compression loop slot/MAJ shape"
import sys
lines = [l.strip() for l in open(sys.argv[1]) if l.strip()]
start = next(i for i, l in enumerate(lines)
             if l.startswith("rorl $") or l.startswith("rorxl $"))
end = next(i for i in range(start + 1, len(lines)) if lines[i].startswith("jl "))
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
note "B/C. shape census done (fails so far: $fails)"

# ── D. exposed store-to-load forwards ───────────────────────────────
for CFG in "-mbmi -O2" "-mbmi -O3" "-march=x86-64-v3 -O2" "-march=x86-64-v3 -O3"; do
  "$CCC" $GCCINC $CFG -S "$SRC" -o "$TMP/f.s" 2>/dev/null
  python3 - "$TMP/f.s" "$CFG" <<'PYEOF' || bad "exposed forwards under $CFG"
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
    f0, f1 = 0, len(raw)
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
done

if [ "$fails" -eq 0 ]; then
  note "ch/maj codegen gate: PASS"
  exit 0
fi
note "ch/maj codegen gate: $fails FAILURES"
exit 1
