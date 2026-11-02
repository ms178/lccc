#!/usr/bin/env bash
# Instruction-count evidence for the __builtin_cpu_supports runtime model.
#
# Usage: tests/workloads/dispatch/run.sh [compiler ...]   (default: lccc gcc clang)
# Env:   LCCC=/path/to/lccc   GCC_BIN=...   CLANG_BIN=...   VALGRIND=valgrind
#        ITERS=50000000   loop trips of two_way_dispatch (kept identical for
#                         every compiler, so the counts are comparable)
#        KEEP=1           keep the raw callgrind outputs under OUT=<dir>
#
# For every compiler the script prints callgrind `PROGRAM TOTALS`
# (= retired instructions, a PMU-free proxy that is immune to host load):
#
#   probe    two_way_dispatch.c              one __builtin_cpu_supports per call
#   control  two_way_dispatch.c -DNO_PROBE   one volatile flag load per call
#   delta    (probe - control) / ITERS       marginal cost of the probe itself
#   init     two_way_dispatch.c -DINIT_ONLY  one-time CPUID initialisation
#   crc32    crc32_dispatch.c                dispatch site over a CRC32 loop
#
# A binary that does not run, or prints the wrong answer, is not evidence: the
# script refuses to report a count for it.
set -uo pipefail
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo=$(CDPATH= cd -- "$here/../../.." && pwd)
LCCC=${LCCC:-$repo/target/fastbuild/lccc}
VALGRIND=${VALGRIND:-valgrind}
ITERS=${ITERS:-50000000}
out=${OUT:-$(mktemp -d)}
trap '[ -n "${KEEP:-}" ] || rm -rf "$out"' EXIT
mkdir -p "$out"

compilers=("$@")
[ ${#compilers[@]} -gt 0 ] || compilers=(lccc gcc clang)

pick_cc() {
    case $1 in
        lccc) printf '%s' "$LCCC" ;;
        gcc)  printf '%s' "${GCC_BIN:-gcc}" ;;
        clang) printf '%s' "${CLANG_BIN:-clang}" ;;
        *)   printf '%s' "$1" ;;
    esac
}

count() {  # binary, label -> executed instructions (printable)
    local bin=$1 label=$2
    "$VALGRIND" --tool=callgrind --callgrind-out-file="$out/cg.$label" \
        --quiet "$bin" >"$out/stdout.$label" 2>"$out/err.$label"
    callgrind_annotate "$out/cg.$label" 2>/dev/null |
        awk '/PROGRAM TOTALS/ {gsub(/,/, "", $1); print $1; exit}'
}

for ccname in "${compilers[@]}"; do
    cc=$(pick_cc "$ccname")
    command -v "$cc" >/dev/null 2>&1 || { echo "$ccname: not found"; continue; }
    src="$here/two_way_dispatch.c"
    build() {  # output-suffix, defs...
        local suffix=$1; shift
        "$cc" -O2 -DITERS="$ITERS" "$@" "$src" -o "$out/$suffix.$ccname" \
            2>"$out/build.$ccname.err" || return 1
    }
    build two_way               || { echo "$ccname: build failed"; head -3 "$out/build.$ccname.err"; continue; }
    build control -DNO_PROBE    || { echo "$ccname: control build failed"; continue; }
    build init    -DINIT_ONLY   || { echo "$ccname: init build failed"; continue; }
    "$cc" -O2 "$here/crc32_dispatch.c" -o "$out/crc32.$ccname" 2>/dev/null \
        || { echo "$ccname: crc32 build failed"; continue; }
    [ "$("$out/two_way.$ccname")" = "1" ] || { echo "$ccname: two_way output wrong"; continue; }
    [ "$("$out/control.$ccname")" = "1" ] || { echo "$ccname: control output wrong"; continue; }
    [ -n "$("$out/init.$ccname")" ] || { echo "$ccname: init output wrong"; continue; }
    [ -n "$("$out/crc32.$ccname")" ] || { echo "$ccname: crc32 output wrong"; continue; }

    probe=$(count "$out/two_way.$ccname" "tw.$ccname")
    control=$(count "$out/control.$ccname" "ct.$ccname")
    init=$(count "$out/init.$ccname" "in.$ccname")
    crc32=$(count "$out/crc32.$ccname" "cr.$ccname")
    delta=$(( (probe - control) / ITERS ))
    printf '%-8s probe=%-12s control=%-12s delta=%-3s/call init=%-8s crc32=%s\n' \
        "$ccname" "$probe" "$control" "$delta" "$init" "$crc32"
done
echo "(callgrind PROGRAM TOTALS; delta = marginal per-call cost of the probe over one plain load)"
