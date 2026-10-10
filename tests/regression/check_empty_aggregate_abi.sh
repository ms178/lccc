#!/usr/bin/env bash
# Cross-compiler ABI oracle for zero-size GNU aggregate parameters. A same-
# compiler caller/callee can agree on the SAME wrong convention, so check both
# directions against GCC at every normal level, on both native x86 targets.
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
SRC=$ROOT/tests/regression/i686_empty_aggregate_params.c
INC=$("$GCC" -print-file-name=include)
printf 'int main(void) { return 0; }\n' >"$WORK/preflight.c"
count=0
for bits in -m64 -m32; do
    # Missing 32-bit headers/runtime are infrastructure failures, not skips.
    timeout "$TIMEOUT" "$GCC" "$bits" "$WORK/preflight.c" -o "$WORK/preflight"
    timeout "$TIMEOUT" "$WORK/preflight"
    for opt in -O0 -O1 -O2 -O3 -Os; do
        for direction in lccc-caller lccc-callee; do
            caller=$CCC; callee=$GCC
            if [[ $direction == lccc-callee ]]; then caller=$GCC; callee=$CCC; fi
            timeout "$TIMEOUT" "$caller" "$bits" "$opt" -w -std=gnu17 -I "$INC" \
                -DCALLER_ONLY -c "$SRC" -o "$WORK/caller.o"
            timeout "$TIMEOUT" "$callee" "$bits" "$opt" -w -std=gnu17 -I "$INC" \
                -DCALLEE_ONLY -c "$SRC" -o "$WORK/callee.o"
            timeout "$TIMEOUT" "$GCC" "$bits" "$WORK/caller.o" "$WORK/callee.o" -o "$WORK/mixed"
            timeout "$TIMEOUT" "$WORK/mixed"
            printf 'PASS %s %s %s\n' "$bits" "$opt" "$direction"
            count=$((count + 1))
        done
    done
done
printf 'empty-aggregate ABI: %d cross-compiler executions passed\n' "$count"
