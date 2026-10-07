#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
ccc=${CCC:-target/fastbuild/lccc}
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT
inc=$(gcc -print-file-name=include)
for opt in -O0 -O1 -O2 -O3; do
    for test in ivsr_narrow_wrap ivsr_signedness_domain ivsr_signed_wrap_impldef ivsr_unsigned_sparse_wrap ivsr_address_add; do
        echo "IVSR: $test $opt"
        "$ccc" "$opt" tests/regression/"$test".c -o "$td/test"
        "$td/test"
        # i686 too: the address-Add arm is gated OFF on ILP32 (the loop-carried
        # pointer web is parked in a slot on a 6-GPR file), so the same source
        # must still be CORRECT there — a gate that only exists on one target
        # is not a gate.
        "$ccc" "$opt" -m32 -msse2 -I"$inc" tests/regression/"$test".c -o "$td/test32"
        "$td/test32"
    done
done
