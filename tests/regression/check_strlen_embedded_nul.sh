#!/usr/bin/env bash
# End-to-end contract for narrow strlen folding: embedded NUL ends the C
# string, while the literal object's trailing bytes remain part of sizeof.
set -euo pipefail
root=$(cd -- "$(dirname -- "$0")/../.." && pwd)
CCC=${CCC:-"$root/target/fastbuild/lccc"}
CC=${CC:-gcc}
[[ -x "$CCC" ]] || { echo "missing LCCC compiler: $CCC" >&2; exit 1; }
command -v "$CC" >/dev/null 2>&1 || { echo "missing C oracle: $CC" >&2; exit 1; }
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

for opt in -O0 -O2; do
    tag=${opt#-}
    "$CCC" "$opt" "$root/tests/regression/strlen_embedded_nul.c" -o "$work/lccc-$tag"
    "$CC" "$opt" "$root/tests/regression/strlen_embedded_nul.c" -o "$work/oracle-$tag"
    "$work/lccc-$tag" >"$work/lccc-$tag.out"
    "$work/oracle-$tag" >"$work/oracle-$tag.out"
    cmp "$work/lccc-$tag.out" "$work/oracle-$tag.out"
    grep -Fxq '6 2 2 2' "$work/lccc-$tag.out"
done

# The optimized TU has a direct builtin, a direct libc call, and a call that
# becomes constant only when the wrapper is inlined. Require the optimization
# result itself, not merely a correct runtime libc fallback.
"$CCC" -O2 -S "$root/tests/regression/strlen_embedded_nul.c" -o "$work/optimized.s"
if grep -Eq 'call[[:space:]].*strlen' "$work/optimized.s"; then
    echo 'optimized embedded-NUL fixture still calls strlen; literal folding did not cover the path' >&2
    exit 1
fi

echo 'embedded-NUL strlen: PASS (literal size 6; all strlen forms return 2 at -O0/-O2; O2 calls folded)'
