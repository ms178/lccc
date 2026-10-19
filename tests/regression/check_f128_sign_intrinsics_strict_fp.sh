#!/usr/bin/env bash
# _Float128 fabs/copysign are sign-bit operations and remain valid under a
# strict dynamic FP environment.  They must never escape as compiler-runtime
# references: LCCC intentionally does not link __fabstf2/__copysigntf3, and
# glibc's libm shared link therefore cannot satisfy such references.
set -euo pipefail
CCC=${CCC:-target/fastbuild/lccc}
NM=${NM:-nm}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
cat >"$tmp/sign.c" <<'EOF'
_Float128 abs128(_Float128 x) { return __builtin_fabsf128(x); }
_Float128 copy128(_Float128 x, _Float128 y) { return __builtin_copysignf128(x, y); }
EOF
for opt in -O0 -O1 -O2 -O3; do
    obj="$tmp/sign-${opt#-}.o"
    "$CCC" "$opt" -frounding-math -ftrapping-math -c "$tmp/sign.c" -o "$obj"
    undefined=$("$NM" -u "$obj")
    if grep -Eq '__fabstf2|__copysigntf3' <<<"$undefined"; then
        echo "FAIL: $opt strict-FP object retained a binary128 sign helper" >&2
        printf '%s\n' "$undefined" >&2
        exit 1
    fi
done
echo 'strict-FP binary128 sign intrinsics: PASS'
