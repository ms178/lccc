#!/usr/bin/env bash
# Byte-boundary and bulk-copy ABI oracle. The same fixture probes mmap guard
# pages, two aggregate arguments, scalar/F64 neighbors, regparm -> cdecl,
# fastcall, and over-aligned local sources. Both GCC/LCCC directions matter.
set -euo pipefail
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
CCC=${CCC:-$ROOT/target/fastbuild/lccc}
GCC=${GCC:-gcc}
TIMEOUT=${TIMEOUT:-30}
[[ $TIMEOUT =~ ^[1-9][0-9]*$ ]] || { echo 'invalid TIMEOUT' >&2; exit 2; }
[[ -x $CCC ]] || { echo "missing compiler: $CCC" >&2; exit 2; }
command -v "$GCC" >/dev/null
command -v timeout >/dev/null
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
SRC=$ROOT/tests/regression/large_aggregate_call_copy.c
INC=$("$GCC" -print-file-name=include)
printf 'int main(void) { return 0; }\n' >"$WORK/preflight.c"
count=0
for bits in -m64 -m32; do
    timeout "$TIMEOUT" "$GCC" "$bits" "$WORK/preflight.c" -o "$WORK/preflight"
    timeout "$TIMEOUT" "$WORK/preflight"
    for opt in -O0 -O1 -O2 -O3 -Os; do
        # Every partial stack-word size plus both sides of the bulk threshold.
        for size in 16 17 18 19 20 21 22 23 31 2047 2048 2049 4097; do
            for key in gcc lccc; do
                cc=$GCC; [[ $key != lccc ]] || cc=$CCC
                timeout "$TIMEOUT" "$cc" "$bits" "$opt" -w -std=gnu17 -I "$INC" \
                    -DBLOB_BYTES="$size" -DCALLER_ONLY -c "$SRC" -o "$WORK/$key-caller.o"
                timeout "$TIMEOUT" "$cc" "$bits" "$opt" -w -std=gnu17 -I "$INC" \
                    -DBLOB_BYTES="$size" -DCALLEE_ONLY -c "$SRC" -o "$WORK/$key-callee.o"
            done
            for pair in lccc:lccc lccc:gcc gcc:lccc; do
                caller=${pair%:*}; callee=${pair#*:}
                timeout "$TIMEOUT" "$GCC" "$bits" "$WORK/$caller-caller.o" \
                    "$WORK/$callee-callee.o" -o "$WORK/mixed"
                if ! timeout "$TIMEOUT" "$WORK/mixed"; then
                    echo "FAIL $bits $opt size=$size caller=$caller callee=$callee" >&2
                    exit 1
                fi
                count=$((count + 1))
            done
            printf 'PASS %s %s size=%s (three compiler pairs)\n' "$bits" "$opt" "$size"
        done
    done
done
printf 'aggregate-call ABI: %d native executions passed\n' "$count"
