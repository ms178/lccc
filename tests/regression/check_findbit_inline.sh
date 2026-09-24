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
# Both shapes must keep the idiom distillations: the generic __ffs
# decision tree folded to tzcnt and the `& ~addr2` arm folded to andn.
# Both hold WITHOUT an explicit -march because the absent-march baseline
# projects x86-64-v3 (BMI1 carries ANDN, ABM carries TZCNT).
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
  "$CCC" $CFG -S "$SRC" -o "$TMP/fb.s" 2>/dev/null \
    || { bad "$CFG: lccc compile"; return; }
  CALLS=$(grep -c 'call linux_find_next_andnot_bit' "$TMP/fb.s" || true)
  [ "$CALLS" -eq "$WANT_CALLS" ] \
    || bad "$CFG: $CALLS outlined calls (want $WANT_CALLS: $WHY)"
  TZ=$(grep -cE '^[[:space:]]*tzcnt[lq]' "$TMP/fb.s" || true)
  [ "$TZ" -ge 1 ] \
    || bad "$CFG: generic __ffs tree did not fold to tzcnt (got $TZ)"
  ANDN=$(grep -cE '^[[:space:]]*andn[lq]' "$TMP/fb.s" || true)
  [ "$ANDN" -ge 1 ] \
    || bad "$CFG: & ~addr2 arm did not fold to andn (got $ANDN)"

  # Bit-exact runtime differential vs the host C compiler.
  "$CCC" $CFG "$SRC" -o "$TMP/l.x" 2>/dev/null \
    || { bad "$CFG: lccc link"; return; }
  L=$("$TMP/l.x"; echo "rc=$?")
  if "$GCC" $CFG "$SRC" -o "$TMP/g.x" 2>/dev/null; then
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

check_cfg "-O2" 3 "historical 2-site cap holds: GCC outlines at -O2"
check_cfg "-O3 -march=x86-64-v3" 3 "measured divergence: inline loses 3-8% at -O3"

if [ "$fails" -eq 0 ]; then
  note "findbit inline gate: PASS"
  exit 0
fi
note "findbit inline gate: $fails FAILURES"
exit 1
