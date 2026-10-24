#!/usr/bin/env bash
# Differential gate for folding a redundant self-TEST into a sign-only
# conditional move (x86-64) or conditional branch (i686).
#
#   arithmetic result; test R,R; cmovge/cmovl   -> cmovns/cmovs, TEST gone
#   arithmetic result; test R,R; jl/jge          -> js/jns, TEST gone
#
# TEST clears OF, so `ge`/`l` after it are exactly `ns`/`s` on the *wrapped*
# result.  The arithmetic producer has the same SF at equal width, so the
# rewrite is valid even when the operation overflows.  A flag-for-flag exact
# logical producer additionally admits the strict `g`/`le` predicates.
#
# The gate is deliberately split from check_self_test_after_arith.sh (a ZF-only
# counter-loop fold) and is wired directly into both CI mirrors.  All oracle,
# fastbuild-binary, skip-group and harness paths are overridable so the same
# script runs in the research sandbox and on a hosted runner.
set -euo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
ROOT=$(cd -- "$SCRIPT_DIR/../.." && pwd)
FIXTURE="$ROOT/tests/regression/sign_test_cmov.c"

lccc=${CCC:-${LCCC_FASTBUILD_BIN:-target/fastbuild/lccc}}
gcc16=${GCC_SIGN_CMOV:-${GCC16:-gcc-16}}
clang=${CLANG_SIGN_CMOV:-${CLANG:-clang}}
icx=${ICX_SIGN_CMOV:-${LCCC_ICX_BIN:-${ICX:-}}}
icx_prefix=${LCCC_ICX_PREFIX:-$HOME/.cache/intel-icx-2026.2}

# Local researchers can opt into the authenticated snapshot provisioner.  A
# hosted runner already exports LCCC_ICX_BIN from its dedicated ICX job, so the
# default never attempts a network download behind the differential gate.
if [ -z "$icx" ] && [ "${LCCC_SIGN_CMOV_PROVISION_ICX:-0}" = 1 ]; then
    if icx=$(bash "$ROOT/scripts/ensure_icx_2026.sh" "$icx_prefix" 2>/dev/null); then
        :
    else
        icx=""
    fi
fi

for tool in "$lccc" "$gcc16" "$clang"; do
    if ! command -v "$tool" >/dev/null 2>&1 && [ ! -x "$tool" ]; then
        echo "FAIL: required tool '$tool' is not executable" >&2
        exit 1
    fi
done

if [ -n "$icx" ]; then
    if ! command -v "$icx" >/dev/null 2>&1 && [ ! -x "$icx" ]; then
        echo "FAIL: ICX_SIGN_CMOV/LCCC_ICX_BIN is set but '$icx' is not executable" >&2
        exit 1
    fi
    icx_macros=$($icx -E -dM -x c /dev/null 2>/dev/null || true)
    if ! grep -q '^#define __INTEL_LLVM_COMPILER ' <<<"$icx_macros"; then
        echo "FAIL: '$icx' does not report the Intel LLVM compiler macro" >&2
        exit 1
    fi
fi

harness=${LCCC_SIGN_CMOV_HARNESS_DIR:-}
if [ -z "$harness" ]; then
    harness=$(mktemp -d "${TMPDIR:-/tmp}/lccc-sign-cmov.XXXXXX")
    trap 'rm -rf "$harness"' EXIT
else
    mkdir -p "$harness"
fi

fail() {
    echo "FAIL: $*" >&2
    exit 1
}

compile_with() {
    local label=$1 compiler=$2 binary=$3
    shift 3
    if ! timeout 120 "$compiler" "$@" "$FIXTURE" -o "$binary" \
        >"$harness/$label.compile.log" 2>&1; then
        cat "$harness/$label.compile.log" >&2
        fail "$label compilation failed"
    fi
}

run_binary() {
    local label=$1 binary=$2 output=$3
    shift 3
    if ! timeout 60 "$@" "$binary" >"$output" 2>"$harness/$label.run.log"; then
        cat "$harness/$label.run.log" >&2
        fail "$label execution failed"
    fi
}

count_sites() {
    grep -c 'testl' "$1" || true
}

# Inspect one function body in LCCC's assembly.  A whole-file grep would pass
# even when the pass died, because the strict-positive negative control in the
# same fixture legitimately keeps its own TEST.
function_has() {
    local file=$1 function=$2 pattern=$3
    awk -v fn="$function" -v pat="$pattern" '
        $0 ~ "^[[:space:]]*" fn ":[[:space:]]*$" { inside = 1 }
        inside && /^[[:space:]]*\.size[[:space:]]/ { exit }
        inside && index($0, pat) { found = 1 }
        END { exit(found ? 0 : 1) }
    ' "$file"
}

gcc_version=$($gcc16 -dumpfullversion -dumpversion 2>/dev/null || echo unknown)
clang_version=$($clang --version 2>/dev/null || echo unknown)
clang_version=${clang_version%%$'\n'*}
oracle_desc=("GCC 16.2 ($gcc_version)" "Clang ($clang_version)")
if [ -n "$icx" ]; then
    icx_version=$($icx --version 2>/dev/null || echo unknown)
    icx_version=${icx_version%%$'\n'*}
    oracle_desc+=("ICX ($icx_version)")
else
    echo "SKIP [ICX differential: no authenticated ICX binary; the hosted icx-oracle job runs this fixture]"
fi

# ---------------------------------------------------------------- x86-64 ----
for opt in O0 O1 O2 O3 Os; do
    tag=x86-$opt
    flag=-$opt
    compile_with "lccc-$tag" "$lccc" "$harness/lccc-$tag" "$flag" -Wall -Wextra -Werror
    run_binary "lccc-$tag" "$harness/lccc-$tag" "$harness/lccc-$tag.txt"
    compile_with "gcc-$tag" "$gcc16" "$harness/gcc-$tag" "$flag" -Wall -Wextra -Werror
    run_binary "gcc-$tag" "$harness/gcc-$tag" "$harness/gcc-$tag.txt"
    compile_with "clang-$tag" "$clang" "$harness/clang-$tag" "$flag" -Wall -Wextra -Werror
    run_binary "clang-$tag" "$harness/clang-$tag" "$harness/clang-$tag.txt"
    diff -u "$harness/lccc-$tag.txt" "$harness/gcc-$tag.txt" >/dev/null \
        || fail "LCCC/GCC output mismatch at $flag"
    diff -u "$harness/lccc-$tag.txt" "$harness/clang-$tag.txt" >/dev/null \
        || fail "LCCC/Clang output mismatch at $flag"
    if [ -n "$icx" ]; then
        compile_with "icx-$tag" "$icx" "$harness/icx-$tag" "$flag" -Wall -Wextra -Werror
        run_binary "icx-$tag" "$harness/icx-$tag" "$harness/icx-$tag.txt"
        diff -u "$harness/lccc-$tag.txt" "$harness/icx-$tag.txt" >/dev/null \
            || fail "LCCC/ICX output mismatch at $flag"
    fi
done
echo "PASS: wrapping-edge runtime output agrees at O0/O1/O2/O3/Os (${oracle_desc[*]})"

# Codegen: the self-TEST disappears and the CMOV condition matches GCC's sign
# test; strict `> 0` is the negative control because it also needs ZF.
if ! timeout 120 "$lccc" -O2 -Wall -Wextra -Werror -S "$FIXTURE" \
    -o "$harness/lccc-x86-O2.s" >"$harness/lccc-x86-O2.compile.log" 2>&1; then
    cat "$harness/lccc-x86-O2.compile.log" >&2
    fail "LCCC -O2 -S failed"
fi
if ! timeout 120 "$gcc16" -O2 -S "$FIXTURE" -o "$harness/gcc-x86-O2.s" \
    >"$harness/gcc-x86-O2.compile.log" 2>&1; then
    cat "$harness/gcc-x86-O2.compile.log" >&2
    fail "GCC -O2 -S failed"
fi

# A dedicated skip group -- not `self_test`, which owns a different fold --
# must restore the original TEST/CMOV shape.  This is the gate's own negative
# control: without it, a dead pass could still pass via the strict-positive
# TEST elsewhere in the same translation unit.
if ! timeout 120 env CCC_PEEPHOLE_SKIP=sign_test_cmov "$lccc" -O2 -S "$FIXTURE" \
    -o "$harness/lccc-x86-skip.s" >"$harness/lccc-x86-skip.compile.log" 2>&1; then
    cat "$harness/lccc-x86-skip.compile.log" >&2
    fail "LCCC skip-group compile failed"
fi
skip_tests=$(count_sites "$harness/lccc-x86-skip.s")
opt_tests=$(count_sites "$harness/lccc-x86-O2.s")
[ "$skip_tests" -gt "$opt_tests" ] \
    || fail "CCC_PEEPHOLE_SKIP=sign_test_cmov did not restore a self-TEST (skip=$skip_tests opt=$opt_tests)"
[ "$opt_tests" -ge 1 ] \
    || fail "strict-positive negative control lost its TEST entirely"

function_has "$harness/lccc-x86-skip.s" nonnegative_difference 'testl' \
    || fail "skip group did not restore nonnegative_difference's TEST"
function_has "$harness/lccc-x86-O2.s" nonnegative_difference 'cmovns' \
    || fail "nonnegative_difference must use cmovns"
function_has "$harness/lccc-x86-O2.s" negative_difference 'cmovs' \
    || fail "negative_difference must use cmovs"
function_has "$harness/lccc-x86-O2.s" positive_difference 'testl' \
    || fail "strict-positive control must keep its TEST"
function_has "$harness/lccc-x86-O2.s" positive_difference 'cmovg' \
    || fail "strict-positive control must keep cmovg"
! function_has "$harness/lccc-x86-O2.s" nonnegative_difference 'testl' \
    || fail "nonnegative_difference still contains a redundant TEST"
! function_has "$harness/lccc-x86-O2.s" negative_difference 'testl' \
    || fail "negative_difference still contains a redundant TEST"
if grep -qE 'cmov(ge|l|g|le)' "$harness/gcc-x86-O2.s"; then
    echo "PASS: sign-only SUB/TEST/CMOV codegen matches GCC's condition; strict-positive control is refused"
else
    fail "GCC reference did not emit a conditional move for this fixture"
fi

# ------------------------------------------------------------------ i686 ----
# The 32-bit backend lowers the same source to branches rather than CMOVs.
# The identical algebraic law applies to `jl`/`jge`, with a CFG proof that no
# later path reads the producer's divergent CF/OF/AF before a complete writer.
if timeout 120 "$lccc" -O2 -m32 -S "$FIXTURE" -o "$harness/lccc-i686-O2.s" \
    >"$harness/lccc-i686-O2.compile.log" 2>&1; then
    if timeout 120 env CCC_NO_I686_SIGN_JCC=1 "$lccc" -O2 -m32 -S "$FIXTURE" \
        -o "$harness/lccc-i686-off.s" >"$harness/lccc-i686-off.compile.log" 2>&1; then
        function_has "$harness/lccc-i686-off.s" nonnegative_difference 'testl' \
            || fail "i686 kill switch did not restore nonnegative_difference's TEST"
        ! function_has "$harness/lccc-i686-O2.s" nonnegative_difference 'testl' \
            || fail "i686 nonnegative_difference still contains a redundant TEST"
        # This backend lowers both the `>= 0` and `< 0` selects to a `jl` with
        # swapped arms, so both consume the sign flag as `js`.  The `jge` ->
        # `jns` spelling is pinned separately by the i686 peephole unit test.
        function_has "$harness/lccc-i686-O2.s" nonnegative_difference 'js ' \
            || fail "i686 nonnegative_difference must use js"
        function_has "$harness/lccc-i686-O2.s" negative_difference 'js ' \
            || fail "i686 negative_difference must use js"
        function_has "$harness/lccc-i686-O2.s" positive_difference 'testl' \
            || fail "i686 strict-positive control must keep its TEST"
    else
        cat "$harness/lccc-i686-off.compile.log" >&2
        fail "i686 kill-switch compile failed"
    fi

    # Runtime parity is opportunistic: some runners lack the 32-bit libc or a
    # qemu-i386 user binary.  Codegen parity above is still mandatory.
    if timeout 120 "$gcc16" -O2 -m32 -Wall -Wextra -Werror "$FIXTURE" \
        -o "$harness/gcc-i686" >"$harness/gcc-i686.compile.log" 2>&1; then
        if timeout 120 env -u CCC_NO_I686_SIGN_JCC "$lccc" -O2 -m32 -Wall -Wextra -Werror \
            "$FIXTURE" -o "$harness/lccc-i686" >"$harness/lccc-i686.compile.log" 2>&1; then
            runner=()
            runnable=1
            if timeout 60 "$harness/gcc-i686" >"$harness/gcc-i686.txt" 2>/dev/null; then
                :
            elif command -v qemu-i386 >/dev/null 2>&1 \
                && timeout 60 qemu-i386 "$harness/gcc-i686" >"$harness/gcc-i686.txt" 2>/dev/null; then
                runner=(qemu-i386)
            else
                runnable=0
            fi
            if [ "$runnable" -eq 1 ]; then
                run_binary "lccc-i686" "$harness/lccc-i686" "$harness/lccc-i686.txt" \
                    "${runner[@]}"
                diff -u "$harness/lccc-i686.txt" "$harness/gcc-i686.txt" >/dev/null \
                    || fail "i686 LCCC/GCC runtime mismatch"
                echo "PASS: i686 runtime and branch-condition parity (GCC 16.2 $gcc_version)"
            else
                echo "SKIP [i686 runtime: host cannot execute -m32 binaries and qemu-i386 is absent]"
            fi
        else
            cat "$harness/lccc-i686.compile.log" >&2
            fail "LCCC -m32 link failed"
        fi
    else
        echo "SKIP [i686 runtime: GCC cannot link -m32 on this host]"
    fi
else
    cat "$harness/lccc-i686-O2.compile.log" >&2
    fail "LCCC -m32 -S failed"
fi
