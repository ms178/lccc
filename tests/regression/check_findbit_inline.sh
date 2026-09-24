#!/usr/bin/env bash
# linux_find_bit inlining gate — pins the inliner's bounded-tier
# clone-growth budget on a real-workload kernel: Linux 6.18.42
# find_next_andnot_bit (static, THREE call sites — two cold self-test
# calls + one hot in-loop call).  Before the growth budget landed, the
# flat two-site cap kept this kernel outlined while GCC 16.2 / Clang
# 23.1 / ICX all inline it (godbolt oracle, pinned alias set); the
# per-call frame cost 6–12% in paired A/B.  The inlined shape must also
# keep the idiom distillations: the generic __ffs decision tree folded
# to tzcnt and the `& ~addr2` arm folded to andn.  Both hold WITHOUT an
# explicit -march because the absent-march baseline projects x86-64-v3
# (BMI1 carries ANDN, ABM carries TZCNT); `-march=x86-64-v3` must match.
#
# Runtime differentials belong to the benchmark-output gate (204 cases);
# this gate pins the CODEGEN contract and fails loudly if future
# inliner tuning silently re-outlines this kernel class.
set -u
CCC=${CCC:-target/fastbuild/lccc}
SRC=$(dirname "$0")/../benchmark/programs/linux_find_bit.c
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
GCC=${GCC_BIN:-gcc}
fails=0
note() { printf '%s\n' "$*"; }
bad()  { note "FAIL: $*"; fails=$((fails+1)); }

for CFG in "-O2" "-O3 -march=x86-64-v3"; do
  "$CCC" $CFG -S "$SRC" -o "$TMP/fb.s" 2>/dev/null \
    || { bad "$CFG: lccc compile"; continue; }
  CALLS=$(grep -c 'call linux_find_next_andnot_bit' "$TMP/fb.s" || true)
  [ "$CALLS" -eq 0 ] \
    || bad "$CFG: $CALLS outlined calls remain (growth budget must admit the 3-site clone)"
  TZ=$(grep -cE '^[[:space:]]*tzcnt[lq]' "$TMP/fb.s" || true)
  [ "$TZ" -ge 1 ] \
    || bad "$CFG: generic __ffs tree did not fold to tzcnt (got $TZ)"
  ANDN=$(grep -cE '^[[:space:]]*andn[lq]' "$TMP/fb.s" || true)
  [ "$ANDN" -ge 1 ] \
    || bad "$CFG: & ~addr2 arm did not fold to andn (got $ANDN)"

  # Bit-exact runtime differential vs the host C compiler.
  "$CCC" $CFG "$SRC" -o "$TMP/l.x" 2>/dev/null \
    || { bad "$CFG: lccc link"; continue; }
  L=$("$TMP/l.x"; echo "rc=$?")
  if "$GCC" $CFG "$SRC" -o "$TMP/g.x" 2>/dev/null; then
    G=$("$TMP/g.x"; echo "rc=$?")
    [ "$G" = "$L" ] || bad "$CFG: runtime differs (gcc=[$G] lccc=[$L])"
  else
    note "skip $CFG runtime differential (no host gcc)"
  fi
done

if [ "$fails" -eq 0 ]; then
  note "findbit inline gate: PASS"
  exit 0
fi
note "findbit inline gate: $fails FAILURES"
exit 1
