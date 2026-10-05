#!/usr/bin/env bash
# Dynamic cross-compiler comparison for tests/oracle/programs/*.c.
#
# The Godbolt oracle (tools/oracle/godbolt_oracle.py) compares *static*
# instruction counts against GCC 16 / Clang 23 / ICX through the Compiler
# Explorer API.  Static counts say nothing about how often each instruction
# runs, so this harness measures *executed* instructions with Callgrind:
#
#   * Ir restricted to the program's own object (loader/libc excluded) via
#     tools/oracle/callgrind_own_ir.py -- the number to optimize,
#   * D1 read misses and LL read misses for the same object,
#   * stdout comparison across compilers, so a "faster" number can never come
#     from a program that computes something else.
#
# Usage:
#   tools/oracle/callgrind_compare.sh [--opt -O2] [--ref gcc[,clang,...]]
#                                     [--only prog] [--json out.json]
#
# Reference compilers default to whatever is installed locally (gcc, then
# clang); lccc is always target/fastbuild/lccc unless $LCCC says otherwise.
set -u

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
LCCC=${LCCC:-$ROOT/target/fastbuild/lccc}
OPT=-O2
REFS=""
ONLY=""
JSON=""
while [ $# -gt 0 ]; do
  case "$1" in
    --opt) OPT=$2; shift 2;;
    --ref) REFS=$2; shift 2;;
    --only) ONLY=$2; shift 2;;
    --json) JSON=$2; shift 2;;
    *) echo "unknown option: $1" >&2; exit 2;;
  esac
done
[ -x "$LCCC" ] || { echo "lccc not built at $LCCC" >&2; exit 1; }
if [ -z "$REFS" ]; then
  REFS=""
  command -v gcc >/dev/null && REFS=gcc
  command -v clang >/dev/null && REFS="${REFS:+$REFS,}clang"
fi
[ -n "$REFS" ] || { echo "no reference compiler found" >&2; exit 1; }

OUT=$(mktemp -d /tmp/cgcmp.XXXXXX)
trap 'rm -rf "$OUT"' EXIT
IR_TOOL=$ROOT/tools/oracle/callgrind_own_ir.py

cc_of() { case "$1" in lccc) echo "$LCCC";; *) command -v "$1" || return 1;; esac; }

printf '%-22s %10s' program lccc
IFS=, read -r -a refarr <<< "$REFS"
for r in "${refarr[@]}"; do printf ' %10s' "$r"; done
printf ' %9s' "lccc/ref"; printf ' %8s %8s\n' D1mr LLmr

json_rows=""
for src in "$ROOT"/tests/oracle/programs/*.c; do
  p=$(basename "$src" .c)
  case "$p" in *_kernel) continue;; esac
  [ -n "$ONLY" ] && [ "$ONLY" != "$p" ] && continue
  declare -A IR MISS JSONROW OUTS
  failed=""
  for tool in lccc "${refarr[@]}"; do
    cc=$(cc_of "$tool") || { failed="$p: compiler $tool missing"; break; }
    bin="$OUT/${p}.$tool"
    "$cc" "$OPT" -g "$src" -o "$bin" 2>"$OUT/${p}.$tool.err" || {
      failed="$p: $tool build failed (see $OUT/${p}.$tool.err)"; break; }
    "$bin" > "$OUT/${p}.$tool.stdout" 2>/dev/null || {
      failed="$p: $tool program failed"; break; }
    cg="$OUT/${p}.$tool.cg"
    if ! valgrind --tool=callgrind --cache-sim=yes --callgrind-out-file="$cg" \
             "$bin" >/dev/null 2>"$OUT/${p}.$tool.vg"; then
      failed="$p: Callgrind failed for $tool"; break;
    fi
    row=$(python3 "$IR_TOOL" --json --show Ir,D1mr,DLmr "$cg") || {
      failed="$p: callgrind_own_ir.py failed on $cg"; break; }
    IR[$tool]=$(printf '%s' "$row" | python3 -c 'import json,sys;print(json.load(sys.stdin)["totals"]["Ir"])')
    MISS[$tool]="$(printf '%s' "$row" | python3 -c 'import json,sys;t=json.load(sys.stdin)["totals"];print(t["D1mr"],t["DLmr"])')"
    JSONROW[$tool]=$row
    OUTS[$tool]=$(md5sum < "$OUT/${p}.$tool.stdout" | cut -d' ' -f1)
  done
  [ -z "$failed" ] || { echo "FAIL: $failed" >&2; exit 1; }
  # Every compiler must produce the same stdout before any ratio is reported.
  for tool in "${refarr[@]}"; do
    if [ "${OUTS[$tool]}" != "${OUTS[lccc]}" ]; then
      echo "MISMATCH: $p stdout differs between lccc and $tool" >&2
      diff "$OUT/${p}.lccc.stdout" "$OUT/${p}.${tool}.stdout" | head -5 >&2
      exit 1
    fi
  done
  first=${refarr[0]}
  ratio=$(awk -v a="${IR[lccc]}" -v b="${IR[$first]}" 'BEGIN{printf "%.4f", a/b}')
  printf '%-22s %10s' "$p" "${IR[lccc]}"
  for r in "${refarr[@]}"; do printf ' %10s' "${IR[$r]}"; done
  printf ' %9s' "$ratio"
  set -- ${MISS[lccc]}; printf ' %8s %8s\n' "$1" "$2"
  json_rows="$json_rows$(printf '{"program":"%s","ir":%s,"ratio_vs_%s":%s,"own":%s},' \
      "$p" "${IR[lccc]}" "$first" "$ratio" "${JSONROW[lccc]}")"
done

if [ -n "$JSON" ]; then
  { printf '{"opt":"%s","reference":"%s","rows":[' "$OPT" "$REFS"
    printf '%s' "$json_rows" | sed 's/,$//'
    printf ']}\n'; } > "$JSON"
  echo "wrote $JSON"
fi
