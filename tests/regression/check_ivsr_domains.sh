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
    for test in ivsr_narrow_wrap ivsr_signedness_domain ivsr_signed_wrap_impldef ivsr_unsigned_sparse_wrap \
            ivsr_address_add ivsr_scale_ring_wrap ivsr_affine_add_wrap; do
        echo "IVSR: $test $opt"
        "$ccc" "$opt" tests/regression/"$test".c -o "$td/test"
        "$td/test"
        # i686 too: the address-Add arm is gated OFF on ILP32 (the loop-carried
        # pointer web is parked in a slot on a 6-GPR file), so the same source
        # must still be CORRECT there — a gate that only exists on one target
        # is not a gate.
        "$ccc" "$opt" -m32 -msse2 -I"$inc" tests/regression/"$test".c -o "$td/test32"
        "$td/test32"
        # Correctness UNDER THE KILL SWITCH. A gate that only ever runs the
        # default configuration cannot tell you whether the escape hatch still
        # produces a correct program, and an escape hatch nobody exercises is
        # how a switch rots into a no-op or a crash. The unit test
        # `ptr_add_parameter_is_a_real_kill_switch` covers the parameter; this
        # covers the environment spelling a user would actually type.
        CCC_NO_IVSR_PTR_ADD=1 "$ccc" "$opt" tests/regression/"$test".c -o "$td/test-off"
        "$td/test-off"
    done
done

# The driver used to silently ignore -fwrapv, allowing signed-UB widening of
# a defined modular address product. Keep the four independent domains and
# both pointer widths, with fixed expected results from arithmetic + oracles.
for opt in -O0 -O1 -O2 -O3 -Os -Oz; do
    for width in native i686; do
        flags=()
        if [[ $width == i686 ]]; then flags=(-m32 -msse2 -I"$inc"); fi
        "$ccc" "$opt" -fwrapv "${flags[@]}" tests/regression/wrapv_domains.c -o "$td/wrapv"
        for case in 0 1 2 3; do
            case $case in
                0) expected=28;; 1) expected=-1073665362;; 2) expected=1;; 3) expected=10;;
            esac
            result=$("$td/wrapv" "$case")
            [[ $result == "$expected" ]] || { echo "FAIL wrapv $opt $width case=$case: $result != $expected" >&2; exit 1; }
        done
        echo "wrapv: $opt $width PASS (4 domains)"
    done
done

for opt in -O0 -O1 -O2 -O3 -Os -Oz; do
    for width in native i686; do
        flags=()
        if [[ $width == i686 ]]; then flags=(-m32 -msse2 -I"$inc"); fi
        "$ccc" "$opt" -fwrapv "${flags[@]}" tests/regression/ivw_wrapv_range.c -o "$td/wrapv-range"
        "$td/wrapv-range"
    done
done
