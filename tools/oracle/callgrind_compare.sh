#!/usr/bin/env bash
# Dynamic own-object Callgrind comparison for standalone oracle programs.
# Ir is simulated instruction work, NOT cycles or a native speedup.
# Usage: callgrind_compare.sh [--opt -O2] [--ref gcc,clang]
#                            [--only program] [--json output.json]
# Fail closed on build/execution/instrumentation/counter/parity failures.
set -euo pipefail
export LC_ALL=C
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
LCCC=${LCCC:-$ROOT/target/fastbuild/lccc}
OPT=-O2
REFS=""
ONLY=""
JSON=""
die() { echo "FAIL: $*" >&2; exit 1; }
while [ $# -gt 0 ]; do
  case "$1" in
    --opt|--ref|--only|--json)
      [ $# -ge 2 ] || die "missing operand for $1"
      case "$1" in
        --opt) OPT=$2;; --ref) REFS=$2;; --only) ONLY=$2;; --json) JSON=$2;;
      esac
      shift 2;;
    *) die "unknown option: $1";;
  esac
done
[ -x "$LCCC" ] || die "lccc not built at $LCCC"
if [ -z "$REFS" ]; then
  command -v gcc >/dev/null && REFS=gcc
  if command -v clang >/dev/null; then REFS="${REFS:+$REFS,}clang"; fi
fi
[ -n "$REFS" ] || die "no reference compiler found"
OUT=$(mktemp -d "${TMPDIR:-/tmp}/cgcmp.XXXXXX")
trap 'rm -rf "$OUT"' EXIT
IR_TOOL=$ROOT/tools/oracle/callgrind_own_ir.py
cc_of() { case "$1" in lccc) echo "$LCCC";; *) command -v "$1";; esac; }
IFS=, read -r -a refarr <<< "$REFS"
for r in "${refarr[@]}"; do
  [ -n "$r" ] && [ "$r" != lccc ] || die "invalid reference: $r"
  cc_of "$r" >/dev/null || die "compiler $r missing"
done
printf '%-22s %10s' program lccc
for r in "${refarr[@]}"; do printf ' %10s' "$r"; done
printf ' %9s %8s %8s\n' lccc/ref D1mr LLmr
: > "$OUT/rows.jsonl"
matched=0
for src in "$ROOT"/tests/oracle/programs/*.c; do
  p=$(basename "$src" .c)
  case "$p" in *_kernel) continue;; esac
  [ -z "$ONLY" ] || [ "$ONLY" = "$p" ] || continue
  matched=$((matched+1))
  declare -A IR=() MISS=() JSONROW=()
  for tool in lccc "${refarr[@]}"; do
    cc=$(cc_of "$tool") || die "compiler $tool missing"
    # Numbered paths also work for reference executable names containing '/'.
    bin="$OUT/program"
    "$cc" "$OPT" -g "$src" -o "$bin" 2>"$OUT/build.err" || {
      cat "$OUT/build.err" >&2; die "$p: $tool build failed"; }
    timeout 300 "$bin" > "$OUT/native.stdout" || die "$p: $tool execution failed"
    cg="$OUT/counters"
    # A successful process may still fail to write its profile. Never consume
    # the preceding compiler's counters when that happens.
    rm -f "$cg"
    timeout 300 valgrind --tool=callgrind --cache-sim=yes --error-exitcode=97 \
      --callgrind-out-file="$cg" "$bin" > "$OUT/instrumented.stdout" 2>"$OUT/valgrind.err" || {
      cat "$OUT/valgrind.err" >&2; die "$p: $tool instrumentation failed"; }
    cmp -s "$OUT/native.stdout" "$OUT/instrumented.stdout" || die "$p: $tool instrumented output differs"
    row=$(python3 "$IR_TOOL" --json --show Ir,D1mr,DLmr "$cg") || die "$p: counter parser failed"
    metrics=$(python3 - "$row" <<'PY'
import json,sys
t=json.loads(sys.argv[1])["totals"]
v=[t[k] for k in ("Ir","D1mr","DLmr")]
if not all(type(n) is int and n >= 0 for n in v) or v[0] == 0:
    raise SystemExit("missing/invalid/empty own-object counters")
print(*v)
PY
    ) || die "$p: invalid counters"
    read -r ir d1 ll <<< "$metrics"
    IR[$tool]=$ir; MISS[$tool]="$d1 $ll"; JSONROW[$tool]=$row
    if [ "$tool" = lccc ]; then
      cp "$OUT/native.stdout" "$OUT/expected.stdout"
    else
      cmp -s "$OUT/expected.stdout" "$OUT/native.stdout" || die "$p: $tool stdout mismatch"
    fi
  done
  first=${refarr[0]}
  ratio=$(awk -v a="${IR[lccc]}" -v b="${IR[$first]}" 'BEGIN{printf "%.6f", a/b}')
  printf '%-22s %10s' "$p" "${IR[lccc]}"
  for r in "${refarr[@]}"; do printf ' %10s' "${IR[$r]}"; done
  read -r d1 ll <<< "${MISS[lccc]}"
  printf ' %9s %8s %8s\n' "$ratio" "$d1" "$ll"
  python3 - "$p" "$first" "$ratio" "${JSONROW[lccc]}" >> "$OUT/rows.jsonl" <<'PY'
import json,sys
p,ref,ratio,raw=sys.argv[1:]
row=json.loads(raw)
print(json.dumps({"program":p,"ir":row["totals"]["Ir"],"ratio_vs_"+ref:float(ratio),"own":row}))
PY
done
[ "$matched" -gt 0 ] || die "selection matched no standalone programs"
if [ -n "$JSON" ]; then
  python3 - "$ROOT" "$JSON" "$OPT" "$REFS" "$OUT/rows.jsonl" <<'PY'
import json,sys
from pathlib import Path
sys.path.insert(0,sys.argv[1])
from tools.corpus import publication
_,dest,opt,refs,rows=sys.argv[1:]
data={"opt":opt,"reference":refs,"rows":[json.loads(s) for s in Path(rows).read_text().splitlines()]}
publication.atomic_bytes(Path(dest),(json.dumps(data,indent=2)+"\n").encode())
PY
  echo "wrote $JSON"
fi
