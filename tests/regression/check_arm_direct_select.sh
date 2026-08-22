#!/usr/bin/env bash
# Unfused AArch64 Select must use register-direct csel (levkropp 8a052b9a
# adapted onto the current fused-select helper), not a 5-move x0/x1/x2 chain.
set -euo pipefail
CCC_ARM=${CCC_ARM:-./target/fastbuild/lccc-arm}
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT
cat >"$td/sel.c" <<'C'
__attribute__((noinline))
int sel(int cond, int a, int b) {
    return cond ? a : b;
}
C
"$CCC_ARM" -O2 -S -o "$td/sel.s" "$td/sel.c"
body=$(sed -n '/^sel:/,/^\.size sel/p' "$td/sel.s")
grep -q 'csel' <<<"$body"
# The old path always emitted mov x1,x0 ; mov x2,x0 around the csel.
if grep -q 'mov x1, x0' <<<"$body" && grep -q 'mov x2, x0' <<<"$body"; then
    echo 'Select still stages both arms through x0' >&2
    echo "$body" >&2
    exit 1
fi
