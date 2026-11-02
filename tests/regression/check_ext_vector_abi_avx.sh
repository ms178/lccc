#!/usr/bin/env bash
# By-value ext_vector_type ABI under the AVX / AVX-512F ABI level, against
# the Clang 23.1.4 oracle (x86-64 only).
#
# The main grid (check_ext_vector_abi_grid.sh) measures the no-AVX
# convention, where a vector over 16 bytes is MEMORY for Clang and the
# larger returns come back in register sequences lccc rejects.  With the
# AVX ABI level on, Clang's `classify` puts a NAMED vector of up to the
# native register width (256 bits under -mavx, 512 under -mavx512f; a
# three- or five-lane type counts at its power-of-two-rounded size, which
# is also its sizeof) in SSE + SSEUP: ONE %ymm / %zmm for arguments and for
# returns.  lccc implements that carrier for `vector_size` (AUD-VABI-2) and
# routes `ext_vector_type` through the same classification, so here every
# shape of that class must AGREE with Clang on all four provider/caller
# legs, returns included -- the rejection the no-AVX grid expects for these
# shapes must NOT fire.  Unnamed (variadic) vectors stay MEMORY under both
# compilers (`hashva` checks that leg).
#
# -mavx512f legs run only on a host whose CPU executes AVX-512F.
set -euo pipefail
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
ccc=${CCC:-$root/target/fastbuild/lccc}
gcc_bin=${GCC:-gcc}
# shellcheck source=tests/regression/lib/clang_oracle.sh
. "$root/tests/regression/lib/clang_oracle.sh"
require_clang_oracle check_ext_vector_abi_avx || exit 1
[ -x "$ccc" ] || { echo "FAIL: compiler not built: $ccc" >&2; exit 1; }
src=$root/tests/regression/ext_vector_abi/abi_grid.c
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

fails=0
fail() {
    echo "FAIL: $*" >&2
    fails=$((fails + 1))
}

elem_of() {
    case ${1%x*} in
        u8) echo "unsigned char" ;;
        u16) echo "unsigned short" ;;
        u32) echo "unsigned" ;;
        u64) echo "unsigned long long" ;;
        f32) echo "float" ;;
        f64) echo "double" ;;
        *) echo "FAIL: unknown shape $1" >&2; exit 1 ;;
    esac
}

# One %ymm under -mavx2: every shape whose rounded size is 32 bytes.
shapes_256="u8x32 u16x16 u32x8 u64x4 f32x8 f64x4 u64x3 f64x3 f32x5 u32x5"
# One %zmm under -mavx512f: 64-byte shapes (the 32-byte ones stay %ymm).
shapes_512="u32x16 u64x8 f32x16 f64x8"

checked=0
run_level() {
    local mflag=$1 shapes=$2 opt shape elem lanes flags who side cc def leg prov call rc
    for opt in -O0 -O2; do
        for shape in $shapes; do
            elem=$(elem_of "$shape")
            lanes=${shape##*x}
            flags=(-m64 "$mflag" "$opt" -no-pie -w "-DELEM=$elem" "-DLANES=$lanes")
            local ok=1
            for who in lccc clang; do
                cc=$ccc
                [[ $who == clang ]] && cc=$CLANG_ORACLE_BIN
                for side in p c; do
                    def=()
                    [[ $side == p ]] && def=(-DPROVIDER)
                    if ! "$cc" "${flags[@]}" "${def[@]}" -c "$src" -o "$tmp/$side-$who.o" \
                        >"$tmp/cc.log" 2>&1; then
                        fail "$mflag $opt $shape: $who failed to compile the $side side"
                        cat "$tmp/cc.log" >&2
                        ok=0
                    fi
                done
            done
            [[ $ok == 1 ]] || continue
            for leg in clang:clang clang:lccc lccc:clang lccc:lccc; do
                prov=${leg%:*}
                call=${leg#*:}
                if ! "$gcc_bin" -m64 -no-pie "$tmp/p-$prov.o" "$tmp/c-$call.o" -o "$tmp/t" \
                    >"$tmp/ld.log" 2>&1; then
                    fail "$mflag $opt $shape: link $prov -> $call"
                    cat "$tmp/ld.log" >&2
                    continue
                fi
                rc=0
                timeout 10 "$tmp/t" >"$tmp/run.log" 2>&1 || rc=$?
                checked=$((checked + 1))
                if [[ $rc -ne 0 ]]; then
                    fail "$mflag $opt $shape $leg: exit $rc: $(tr '\n' ' ' <"$tmp/run.log")"
                fi
            done
        done
    done
}

run_level -mavx2 "$shapes_256"
ran512=0
if grep -qw avx512f /proc/cpuinfo 2>/dev/null; then
    ran512=1
    run_level -mavx512f "$shapes_256 $shapes_512"
else
    echo "skip: -mavx512f legs: host CPU has no AVX-512F"
fi

if [[ $fails -ne 0 ]]; then
    echo "FAIL: ext-vector AVX-level ABI: $fails failure(s) ($checked legs run)" >&2
    exit 1
fi
echo "PASS: ext-vector AVX-level ABI vs Clang $CLANG_ORACLE_VERSION: $checked legs agree" \
    "(-mavx2$([ $ran512 = 1 ] && echo ', -mavx512f'))"
