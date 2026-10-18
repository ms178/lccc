#!/usr/bin/env bash
# AUD-VABI-2: 32- and 64-byte vectors across a compiler boundary.
#
# The sibling gate (check_vector_abi_boundary.sh) pins the 16-byte case
# (AUD-VABI-1: a `<4 x i32>` argument or return is ONE %xmm).  The wider
# case is the same rule one register class up: the psABI classifies a vector
# as SSE + one SSEUP per further eightbyte, so WITH AVX ENABLED a 32-byte
# vector is ONE %ymm, and with AVX-512F a 64-byte vector is ONE %zmm -- for
# arguments, for returns, and for a struct/union whose only member is such a
# vector.  Without the AVX ABI level the same types are MEMORY.
#
# History: this gate used to LATCH the debt (lccc had no SSEUP notion past 16
# bytes: lccc caller + gcc callee returned silent garbage, gcc caller + lccc
# callee crashed with SIGSEGV under -mavx2).  The fix landed with a sized
# vector-register carrier (`StructXmmReg { size }` / `F128SseReg { size }`,
# the hidden `%ymm0`/`%zmm0` return buffer, the variadic MEMORY demotion of
# unnamed wide vectors), so every leg below is now required to AGREE with
# gcc/gcc.  There is no latch left: a disagreement is a failure.
#
# Two reproducers:
#   * wide.c      -- the original `v8si`/`v4df` argument/return shapes plus
#                    the 32-byte STRUCT control (`struct { double a,b,c,d; }`
#                    is four SSE eightbytes, NOT SSEUP: it must keep the
#                    memory convention under every configuration);
#   * wide_avx.c  -- the full rule set: 32/64-byte vectors, one-member
#                    structs/unions, the two-member and 40/96-byte structs
#                    that stay MEMORY, the transactional SSE overflow (ninth
#                    vector on the stack at its 32-byte alignment, the scalar
#                    after it still in a GPR), mixed scalar/vector walks,
#                    unnamed vectors of variadic calls (MEMORY, %al counts
#                    only the SSE registers in use), RVO through %ymm0,
#                    function-pointer calls and by-pointer/global state.
#
# Each reproducer is built in four combinations (gcc/gcc reference, lccc/lccc,
# lccc caller + gcc callee, gcc caller + lccc callee) per configuration and
# every binary's output must equal the reference's.  Configurations:
#   -O0/-O2 x { -mavx2, -march=x86-64-v3, -mavx512f, (none), -mno-avx }
# The -mavx512f configuration is run only on a host whose CPU executes
# AVX-512F (the objects are still compiled and linked everywhere).  The
# no-AVX configurations are also the gate's liveness proof: the same diffs
# that are silent when the two compilers agree do fire on a convention
# mismatch, and the memory convention must keep interoperating.
set -euo pipefail

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
ccc=${CCC:-$repo/target/fastbuild/lccc}
# The oracle must be GCC 16: GCC 12's callee for `c_u32` (a union of
# 32-byte vectors returned in %ymm0) returns only the low 128 bits
# (`u32 9 0` for `u32 9 9`); GCC 16 and lccc agree.  ci_local.sh and the
# hosted workflow both pin `GCC=gcc-16`.
gcc_bin=${GCC:-gcc}
dir=$repo/tests/regression/vector_abi_boundary
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

[ -x "$ccc" ] || { echo "SKIP: $ccc not built" >&2; exit 0; }
command -v "$gcc_bin" >/dev/null 2>&1 || {
    echo "SKIP: oracle unavailable ($gcc_bin not found)" >&2; exit 0; }

fail() { echo "FAIL: $*" >&2; exit 1; }

host_has_avx512f() {
    grep -qw avx512f /proc/cpuinfo 2>/dev/null
}

# one_config <src-basename> <tag> <flags...>: build the four combinations
# and compare every binary's output with the gcc/gcc reference.
one_config() {
    local base=$1 tag=$2
    shift 2
    local src=$dir/$base.c
    local flags=("$@")
    local p=$tmp/${base}_$tag

    # -Wno-psabi: the no-AVX configurations ARE the memory convention, and
    # gcc's "AVX vector argument without AVX enabled changes the ABI" note is
    # exactly the property under test there -- expected, not a finding.
    "$gcc_bin" -Wno-psabi "${flags[@]}" -DCALLEE_ONLY -c "$src" -o "${p}_callee_gcc.o"
    "$ccc" "${flags[@]}" -DCALLEE_ONLY -c "$src" -o "${p}_callee_lccc.o" ||
        fail "$base $tag: lccc failed to compile the callee TU"
    "$gcc_bin" -Wno-psabi "${flags[@]}" -c "$src" -o "${p}_caller_gcc.o"
    "$ccc" "${flags[@]}" -c "$src" -o "${p}_caller_lccc.o" ||
        fail "$base $tag: lccc failed to compile the caller TU"

    "$gcc_bin" "${p}_caller_gcc.o" "${p}_callee_gcc.o" -o "${p}_ref"
    "${p}_ref" >"${p}_ref.out"
    # The oracle's own output must be the arithmetic answer: if gcc/gcc were
    # itself broken, every comparison below would be against garbage.
    grep -q "^${ref_probe}\$" "${p}_ref.out" ||
        { cat "${p}_ref.out" >&2;
          fail "$base $tag: the gcc/gcc reference did not produce the expected answer"; }

    local leg caller callee
    for leg in "lccc lccc" "lccc gcc" "gcc lccc"; do
        read -r caller callee <<<"$leg"
        "$gcc_bin" "${p}_caller_$caller.o" "${p}_callee_$callee.o" -o "${p}_${caller}_$callee" ||
            fail "$base $tag: linking the $caller caller against the $callee callee failed"
        set +e
        "${p}_${caller}_$callee" >"${p}_${caller}_$callee.out" 2>&1
        local rc=$?
        set -e
        [ "$rc" -eq 0 ] ||
            fail "$base $tag: $caller caller + $callee callee exited with $rc (signal $((rc - 128)) if >128)"
        diff -u "${p}_ref.out" "${p}_${caller}_$callee.out" >&2 ||
            fail "$base $tag: $caller caller + $callee callee disagrees with gcc/gcc"
    done
    echo "ok: $base [${flags[*]}]: all four combinations agree with gcc/gcc"
}

configs=(
    "avx2|-mavx2"
    "v3|-march=x86-64-v3"
    "avx512f|-mavx512f"
    "mem|"
    "noavx|-mno-avx"
)
ran512=0
for opt in -O0 -O2; do
    for cfg in "${configs[@]}"; do
        name=${cfg%%|*}
        extra=${cfg#*|}
        if [ "$name" = avx512f ] && ! host_has_avx512f; then
            echo "skip: $opt -mavx512f: host CPU has no AVX-512F (objects not run)"
            continue
        fi
        [ "$name" = avx512f ] && ran512=1
        flags=("$opt")
        [ -n "$extra" ] && flags+=("$extra")
        ref_probe='add: 42 42 42 42 42 42 42 42' \
            one_config wide "$name${opt#-}" "${flags[@]}"
        ref_probe='acc 42579.375' \
            one_config wide_avx "$name${opt#-}" "${flags[@]}"
    done
done

echo "PASS: AUD-VABI-2: 32-byte (%ymm) and 64-byte (%zmm) vector ABI agrees with"
echo "      gcc in both call directions under AVX2/x86-64-v3$([ $ran512 = 1 ] && echo /AVX-512F),"
echo "      and the memory convention agrees without the AVX ABI level"
