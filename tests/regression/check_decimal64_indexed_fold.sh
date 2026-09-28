#!/usr/bin/env bash
# D64 indexed-fold gate: _Decimal64 loop traffic folds into SIB movsd.
#
# The x86-64 indexed path must accept D64 on both halves — the decider's
# type allowlist (`indexed_fold_ok`), the emitter's SSE arms
# (`emit_load/store_indexed_common`), and the store-side staging proof —
# so a `_Decimal64 a[i]` loop emits scale-8 `movsd` SIB traffic, not LEA
# rematerialisation plus unfolded accesses. Pins the fold structurally
# (scale-8 SIB movsd present on the load AND store side) and
# differentially (the binary's integer-domain checksums agree with gcc -O2;
# integer truncation, so legal quantum drift cannot fail a correct build).
set -eu

CCC=${CCC:-./target/fastbuild/lccc}
dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
tmp="${TMPDIR:-/tmp}/lccc-d64-indexed-fold.$$"
trap 'rm -rf "$tmp"' EXIT HUP INT TERM
mkdir -p "$tmp"

"$CCC" -O2 -S "$dir/decimal64_indexed_fold.c" -o "$tmp/out.s"

# Structural: scale-8 SIB movsd on the load side (optional PF-06 disp) ...
if ! grep -qE 'movsd -?[0-9]*\(%r[a-z0-9]+, *%r[a-z0-9]+, *8\), %xmm' "$tmp/out.s"; then
    echo "FAIL: no scale-8 SIB movsd load in $tmp/out.s" >&2
    grep -E 'movsd' "$tmp/out.s" >&2 | head -10
    exit 1
fi
# ... and on the store side (optional PF-06 displacement).
if ! grep -qE 'movsd %xmm[0-9]+, +-?[0-9]*\(%r[a-z0-9]+, *%r[a-z0-9]+, *8\)' "$tmp/out.s"; then
    echo "FAIL: no scale-8 SIB movsd store in $tmp/out.s" >&2
    grep -E 'movsd' "$tmp/out.s" >&2 | head -10
    exit 1
fi

# Differential: integer-domain checksums agree with gcc -O2.
timeout 120 gcc -O2 -o "$tmp/bin-gcc" "$dir/decimal64_indexed_fold.c"
timeout 300 "$CCC" -O2 -o "$tmp/bin-lccc" "$dir/decimal64_indexed_fold.c"
"$tmp/bin-gcc" > "$tmp/expect.out"
"$tmp/bin-lccc" > "$tmp/got.out"
if ! cmp -s "$tmp/expect.out" "$tmp/got.out"; then
    echo "FAIL: D64 indexed-fold stdout differs from gcc" >&2
    diff "$tmp/expect.out" "$tmp/got.out" >&2 | head -10
    exit 1
fi
echo "PASS: D64 indexed fold (SIB movsd load+store, sums agree with gcc)"
