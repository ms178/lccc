#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
ccc=${CCC:-target/fastbuild/lccc}
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT
for opt in -O1 -O2 -O3; do
    for test in va_arg_pack_len_inline fortify_open_va_pack; do
        echo "VA pack: $test $opt"
        "$ccc" "$opt" tests/regression/"$test".c -o "$td/vapack"
        "$td/vapack"
    done
done
