#!/usr/bin/env bash
# Vector initializers inside aggregates, byte for byte against GCC 16.2.
#
# tests/regression/vector_static_init/vsi.c dumps the object representation of
# every static, static-local and automatic object it initializes.  lccc used to
# emit a vector member of a static aggregate as its first lane truncated to one
# byte (`struct { v2d v; } g = {{7.0, 10.0}}` -> `.byte 7; .zero 15`), accept
# brace elision into vector members only as written, and fold no vector cast.
# The program is built by the oracle and by lccc at -O0 and -O2; every line
# must match.  A missing oracle fails the gate; it is never skipped.
set -euo pipefail
. tests/regression/lib/gcc_oracle.sh
require_gcc_oracle check_vector_static_init || exit 1

CCC=${CCC:-target/fastbuild/lccc}
src=tests/regression/vector_static_init/vsi.c
[[ -x $CCC ]] || { echo "check_vector_static_init: FAIL: compiler not found at $CCC" >&2; exit 1; }

tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-vsi.XXXXXX")
trap 'rm -rf "$tmp"' EXIT

"$GCC_ORACLE_BIN" -O2 -std=gnu17 -w "$src" -o "$tmp/ref"
"$tmp/ref" > "$tmp/ref.txt"
rows=$(wc -l < "$tmp/ref.txt")
(( rows >= 40 )) || { echo "check_vector_static_init: FAIL: oracle printed only $rows rows" >&2; exit 1; }

fail=0
for opt in -O0 -O2; do
    if ! "$CCC" "$opt" -std=gnu17 -w "$src" -o "$tmp/t$opt" 2> "$tmp/err$opt"; then
        echo "FAIL $opt: lccc rejected the program" >&2
        sed -n '1,5p' "$tmp/err$opt" >&2
        fail=$((fail + 1)); continue
    fi
    "$tmp/t$opt" > "$tmp/out$opt.txt" || true
    if ! diff -q "$tmp/ref.txt" "$tmp/out$opt.txt" > /dev/null; then
        diff "$tmp/ref.txt" "$tmp/out$opt.txt" | grep '^>' | sed "s/^/FAIL $opt: /" >&2 || true
        fail=$((fail + 1))
    fi
done

if (( fail > 0 )); then
    echo "check_vector_static_init: FAILED" >&2
    exit 1
fi
echo "check_vector_static_init: PASSED ($rows objects x 2 opt levels vs GCC 16)"
