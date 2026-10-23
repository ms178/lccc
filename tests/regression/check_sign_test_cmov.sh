#!/usr/bin/env bash
# Differential/codegen oracle for the x86 SUB; TEST; sign-only CMOV fold.
set -euo pipefail

root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
source_file="$root/tests/regression/sign_test_cmov.c"
CCC=${CCC:-$root/target/fastbuild/lccc}
GCC=${GCC:-gcc-16}
CLANG=${CLANG:-clang}
ICX=${ICX:-}

for tool in "$CCC" "$GCC" "$CLANG"; do
    command -v "$tool" >/dev/null 2>&1 || { echo "FAIL: missing oracle/compiler $tool" >&2; exit 1; }
done
ccc_version=$("$CCC" --version)
gcc_version=$("$GCC" --version)
clang_version=$("$CLANG" --version)
grep -q 'lccc' <<<"$ccc_version" || { echo "FAIL: CCC is not lccc" >&2; exit 1; }
grep -Eq '16\.2(\.0)?' <<<"$gcc_version" || { echo "FAIL: GCC 16.2 required" >&2; exit 1; }
grep -q '23\.1\.3' <<<"$clang_version" || { echo "FAIL: Clang 23.1.3 required" >&2; exit 1; }
if [ -n "$ICX" ]; then
    command -v "$ICX" >/dev/null 2>&1 || { echo "FAIL: ICX not found: $ICX" >&2; exit 1; }
    icx_version=$("$ICX" --version 2>&1)
    grep -q '2026\.2' <<<"$icx_version" || { echo "FAIL: ICX 2026.2 required" >&2; exit 1; }
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

compile_and_run() {
    local name=$1 compiler=$2 opt=$3
    "$compiler" "$opt" -fwrapv -Wall -Wextra -Werror "$source_file" -o "$work/$name"
    "$work/$name" >"$work/$name.out"
}
for opt in -O0 -O1 -O2 -O3 -Os; do
    compile_and_run "lccc-$opt" "$CCC" "$opt"
    compile_and_run "gcc-$opt" "$GCC" "$opt"
    compile_and_run "clang-$opt" "$CLANG" "$opt"
    cmp "$work/gcc-$opt.out" "$work/lccc-$opt.out"
    cmp "$work/clang-$opt.out" "$work/lccc-$opt.out"
    if [ -n "$ICX" ]; then
        compile_and_run "icx-$opt" "$ICX" "$opt"
        cmp "$work/icx-$opt.out" "$work/lccc-$opt.out"
    fi
done

echo "PASS: wrapping-edge runtime output agrees at O0/O1/O2/O3/Os (lccc/GCC 16.2/Clang 23.1.3${ICX:+/ICX 2026.2})"

"$CCC" -O2 -fwrapv -S "$source_file" -o "$work/lccc.s"
CCC_PEEPHOLE_SKIP=self_test "$CCC" -O2 -fwrapv -S "$source_file" -o "$work/lccc-disabled.s"
"$GCC" -O2 -fwrapv -S "$source_file" -o "$work/gcc.s"
extract() {
    awk -v fn="$2" '$0 ~ "^" fn ":" { inside=1 } inside { print } inside && $0 ~ "^\\.size[[:space:]]+" fn "([,[:space:]]|$)" { exit }' "$1"
}

lccc_nonnegative=$(extract "$work/lccc.s" nonnegative_difference)
lccc_disabled=$(extract "$work/lccc-disabled.s" nonnegative_difference)
gcc_nonnegative=$(extract "$work/gcc.s" nonnegative_difference)
if ! printf '%s\n' "$lccc_disabled" | grep -Eq '[[:space:]]test[lq][[:space:]]'; then
    echo "FAIL: disabling the self-test peepholes did not restore the TEST control" >&2
    printf '%s\n' "$lccc_disabled" >&2
    exit 1
fi
if printf '%s\n' "$lccc_nonnegative" | grep -Eq '[[:space:]]test[lq][[:space:]]'; then
    echo "FAIL: redundant TEST remains in lccc nonnegative_difference" >&2
    printf '%s\n' "$lccc_nonnegative" >&2
    exit 1
fi
printf '%s\n' "$lccc_nonnegative" | grep -Eq '[[:space:]]cmovns[lq]?[[:space:]]' || {
    echo "FAIL: lccc did not use the sign-only condition" >&2
    printf '%s\n' "$lccc_nonnegative" >&2
    exit 1
}
printf '%s\n' "$gcc_nonnegative" | grep -Eq '[[:space:]]cmovns[q]?[[:space:]]' || {
    echo "FAIL: GCC 16.2 oracle shape changed; re-audit the witness" >&2
    printf '%s\n' "$gcc_nonnegative" >&2
    exit 1
}

lccc_positive=$(extract "$work/lccc.s" positive_difference)
printf '%s\n' "$lccc_positive" | grep -Eq '[[:space:]]test[lq][[:space:]]' || {
    echo "FAIL: strict-positive negative control lost its TEST" >&2
    printf '%s\n' "$lccc_positive" >&2
    exit 1
}
printf '%s\n' "$lccc_positive" | grep -Eq '[[:space:]]cmovg[lq][[:space:]]' || {
    echo "FAIL: strict-positive negative control changed condition" >&2
    printf '%s\n' "$lccc_positive" >&2
    exit 1
}
echo "PASS: sign-only SUB/TEST/CMOV codegen matches GCC's condition; strict-positive control is refused"
