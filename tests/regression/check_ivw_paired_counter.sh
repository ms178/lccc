#!/usr/bin/env bash
# Executed regression for the iv_widen paired-counter (sibling mirror) proof.
#
#  1. tests/regression/ivw_paired_counter.c: runtime-bound loops at the narrow
#     INT_MIN/INT_MAX edges, including shapes the proof must refuse. Output
#     must equal the gcc oracle at -O0..-O3 with -fwrapv, both with the
#     default pipeline and with the CCC_NO_IV_WIDEN=1 kill switch.
#  2. tests/regression/ivw_paired_const.c: constant-threshold counter. The
#     proof must FIRE here, so default and kill-switch assembly must differ.
#     A silent refusal would otherwise pass every semantic check unnoticed.
set -euo pipefail
cd "$(dirname "$0")/../.."
ccc=${CCC:-target/fastbuild/lccc}
ref_cc=gcc-16
command -v "$ref_cc" >/dev/null 2>&1 || ref_cc=gcc
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT

check_arm() { # <label> <expected-output> <binary>
    local rc=0 got
    got=$("$3") || rc=$?
    if [ "$rc" != 0 ] || [ "$got" != "$2" ]; then
        echo "FAIL $1: rc=$rc got [$got] want [$2]" >&2
        exit 1
    fi
}

src=tests/regression/ivw_paired_counter.c
for opt in -O0 -O1 -O2 -O3; do
    "$ref_cc" "$opt" -fwrapv -w "$src" -o "$td/ref"
    want=$("$td/ref")
    echo "IVW-PAIRED: default $opt"
    "$ccc" "$opt" -fwrapv -w "$src" -o "$td/on$opt"
    check_arm "default $opt" "$want" "$td/on$opt"
    echo "PASS"
    echo "IVW-PAIRED: no-iv-widen $opt"
    CCC_NO_IV_WIDEN=1 "$ccc" "$opt" -fwrapv -w "$src" -o "$td/off$opt"
    check_arm "no-iv-widen $opt" "$want" "$td/off$opt"
    echo "PASS"
done

const=tests/regression/ivw_paired_const.c
"$ref_cc" -O2 -fwrapv -w "$const" -o "$td/const-ref"
want_const=$("$td/const-ref")
for opt in -O0 -O2 -O3; do
    "$ccc" "$opt" -fwrapv -w "$const" -o "$td/const$opt"
    check_arm "paired-const $opt" "$want_const" "$td/const$opt"
done
echo "IVW-PAIRED: proof fires on constant-threshold shape"
# Compare only the paired_const body: the kill switch disables every iv_widen
# proof, so unrelated loops elsewhere in the file would also differ.
"$ccc" -O2 -fwrapv -w -S "$const" -o "$td/const-on.s"
CCC_NO_IV_WIDEN=1 "$ccc" -O2 -fwrapv -w -S "$const" -o "$td/const-off.s"
body() { awk '/^paired_const:/{on=1} on{print} /\.size[ \t]+paired_const,/{exit}' "$1"; }
body "$td/const-on.s" > "$td/const-on.fn"
body "$td/const-off.s" > "$td/const-off.fn"
if [ ! -s "$td/const-on.fn" ] || cmp -s "$td/const-on.fn" "$td/const-off.fn"; then
    echo "FAIL: paired-counter proof no longer changes the constant-threshold loop" >&2
    exit 1
fi
echo "PASS"
echo "IVW-PAIRED-COUNTER: PASS ($ref_cc oracle)"
