#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
ccc=${CCC:-target/fastbuild/lccc}
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT
inc=$(gcc -print-file-name=include)
for opt in -O0 -O1 -O2 -O3; do
    # i686 is covered because `result_type()` for scalar lane extracts is an
    # UNCONDITIONAL change: it drives compact_i686_values, is_wide_on_32bit and
    # the prologue's compaction veto, not just the x86-64 small-slot class.
    for isa in x86-64 x86-64-v3 i686; do
        arch=(-march="$isa")
        extra=()
        if [ "$isa" = i686 ]; then arch=(-m32); extra=(-msse2); fi
        for test in slp_fp_extract_home fp_extract_slot_boundary; do
            for mode in default homes wide-slots; do
                echo "FP lane: $test $opt $isa $mode"
                knobs=()
                case $mode in
                    homes) knobs+=(CCC_FP_EXTRACT_HOMES=1);;
                    wide-slots) knobs+=(CCC_NO_SMALL_SLOTS=1);;
                esac
                env -u CCC_FP_EXTRACT_HOMES -u CCC_NO_SMALL_SLOTS "${knobs[@]}" \
                    "$ccc" "$opt" "${arch[@]}" "${extra[@]}" -I"$inc" \
                    tests/regression/"$test".c -o "$td/fp"
                "$td/fp"
            done
        done
    done
done
