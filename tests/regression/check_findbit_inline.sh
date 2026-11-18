#!/usr/bin/env bash
# linux_find_bit inlining gate — pins the inliner's opt-level gradient on a
# real-workload kernel: Linux 6.18.42 find_next_andnot_bit (static, THREE
# call sites — two cold self-test calls + one hot in-loop call).
#
# Policy (godbolt pinned oracles + LCCC-side A/B, 2026-09-24): at -O2
# GCC 14.2 AND GCC 16.2 keep the 60-insn kernel OUTLINED (only the LLVM
# family inlines); LCCC-side the inline measures as a tie (paired A/B
# n=7 spans 1; layout-averaged k-sweep n=16 splits 9–7). At -O3 the
# oracles unanimously inline — but LCCC-side the inline measures as a
# 3–8% LOSS (k-sweep wins 3–13, p≈0.01): the oracle backends exploit
# the inline and LCCC's does not. So the bounded tier holds the
# historical 2-site cap at EVERY level: a deliberate, measured
# divergence from the -O3 oracle row. Revisit with backend evidence.
#
# Both shapes must keep the idiom distillations.  The generic __ffs
# decision tree folds to a tzcnt-SPELLED scan at every tier: __ffs has a
# nonzero precondition, and on that domain TZCNT's encoding (F3 0F BC =
# `rep bsf`) runs as BSF on pre-BMI1 CPUs with the same result, so the
# spelling needs no ISA grant (GCC emits the same bytes at plain -O2).
# ANDN is different: a VEX encoding that #UDs without BMI1, so the
# `& ~addr2` arm may fold to andn only under an explicit v3 -march.
#
# Runtime differentials belong to the benchmark-output gate (204 cases);
# this gate pins the CODEGEN contract and fails loudly if future
# inliner tuning silently moves this kernel class across the gradient.
set -u
CCC=${CCC:-target/fastbuild/lccc}
SRC=$(dirname "$0")/../benchmark/programs/linux_find_bit.c
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
GCC=${GCC_BIN:-gcc}
# Host-gcc include path: a missing system header must surface as a
# compile failure here, not as a silent floor failure.
GCCINC=$("$GCC" -print-file-name=include 2>/dev/null)
[ -d "$GCCINC" ] && GCCINC="-I$GCCINC" || GCCINC=""
# The runtime differential is a correctness check: a broken reference
# build must FAIL, not silently skip (explicit opt-out only).
ALLOW_GCC_SKIP=${LCCC_ALLOW_GCC_SKIP:-0}
fails=0
note() { printf '%s\n' "$*"; }
bad()  { note "FAIL: $*"; fails=$((fails+1)); }

check_cfg() {
  CFG=$1
  WANT_CALLS=$2
  WHY=$3
  TIER=$4   # bmi: explicit v3 (BMI1/ABM granted); base: x86-64 baseline
  "$CCC" $GCCINC $CFG -S "$SRC" -o "$TMP/fb.s" 2>/dev/null \
    || { bad "$CFG: lccc compile"; return; }
  # ANCHORED census (leading whitespace + `call` + whitespace + symbol):
  # a comment or symbol merely CONTAINING the substring would deflate
  # `grep -c` to 0 and silently pass a "must be 0" style check.
  CALLS=$(grep -cE '^[[:space:]]*call[[:space:]]+linux_find_next_andnot_bit' "$TMP/fb.s" || true)
  [ "$CALLS" -eq "$WANT_CALLS" ] \
    || bad "$CFG: $CALLS outlined calls (want $WANT_CALLS: $WHY)"
  TZ=$(grep -cE '^[[:space:]]*tzcnt[lq]' "$TMP/fb.s" || true)
  ANDN=$(grep -cE '^[[:space:]]*andn[lq]' "$TMP/fb.s" || true)
  BSF=$(grep -cE '^[[:space:]]*bsf[lq]' "$TMP/fb.s" || true)
  # The __ffs tree folds to a tzcnt-spelled scan at EVERY tier (nonzero
  # domain; the encoding is `rep bsf`, baseline-legal -- see header).
  [ "$TZ" -ge 1 ] || bad "$CFG: generic __ffs tree did not fold to tzcnt (got $TZ)"
  [ "$BSF" -eq 0 ] || bad "$CFG: nonzero scan left as plain bsf (slower on AMD, got $BSF)"
  if [ "$TIER" = bmi ]; then
    [ "$ANDN" -ge 1 ] || bad "$CFG: & ~addr2 arm did not fold to andn (got $ANDN)"
  else
    # Baseline: ANDN is a VEX encoding, #UD without BMI1 -- never baseline.
    [ "$ANDN" -eq 0 ] || bad "$CFG: baseline emitted $ANDN andn (post-baseline)"
  fi

  # Bit-exact runtime differential vs the host C compiler.
  "$CCC" $GCCINC $CFG "$SRC" -o "$TMP/l.x" 2>/dev/null \
    || { bad "$CFG: lccc link"; return; }
  L=$("$TMP/l.x"; echo "rc=$?")
  if "$GCC" $GCCINC $CFG "$SRC" -o "$TMP/g.x" 2>/dev/null; then
    G=$("$TMP/g.x"; echo "rc=$?")
    [ "$G" = "$L" ] || bad "$CFG: runtime differs (gcc=[$G] lccc=[$L])"
  else
    if [ "$ALLOW_GCC_SKIP" = 1 ]; then
      note "skip $CFG runtime differential (no host gcc; LCCC_ALLOW_GCC_SKIP=1)"
    else
      bad "$CFG gcc build failed (no silent skip: set LCCC_ALLOW_GCC_SKIP=1 to opt out)"
    fi
  fi
}

check_cfg "-O2" 3 "historical 2-site cap holds: GCC outlines at -O2" base
check_cfg "-O3 -march=x86-64-v3" 3 "measured divergence: inline loses 3-8% at -O3" bmi

if [ "$fails" -eq 0 ]; then
  note "findbit inline gate: PASS"
  exit 0
fi
note "findbit inline gate: $fails FAILURES"
exit 1
