#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
ccc=${CCC:-target/fastbuild/lccc}
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT
inc=$(gcc -print-file-name=include)
# Attributing the host, not the compiler.  This gate *asserts* its 32-bit leg (it is
# not one of the gates that skips when i686 is unavailable), so on a machine without a
# 32-bit libc the gate has to fail -- but the failing command below is lccc, so
# without this probe a reader sees our compiler accused of a preprocessor error
# ("ccc: error: 9 preprocessor error(s)") when the actual cause is that `gcc -m32`
# cannot even link `int main() { return 0; }` here.  Ask the oracle first, and name
# what to install.
if ! printf 'int main(void) { return 0; }\n' >"$td/i386-host-probe.c" \
   || ! gcc -m32 "$td/i386-host-probe.c" -o "$td/i386-host-probe" 2>"$td/i386-host-probe.err" \
   || ! "$td/i386-host-probe" >/dev/null 2>&1; then
    echo "FAIL (host, not lccc): gcc -m32 cannot build and run a trivial program here; install gcc-multilib and libc6-dev-i386 (the runner step in .github/workflows/ci.yml does exactly this).  This gate asserts its i386 legs by design rather than skipping them, so nothing is measured silently." >&2
    sed -n '1,2p' "$td/i386-host-probe.err" >&2
    exit 1
fi

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
