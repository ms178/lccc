#!/usr/bin/env bash
# Focused runtime pins for five independent x86 execute failures: aggregate
# snapshot overwrites and global-alias windows, the target-dependent small
# __builtin_shuffle result convention, exact generic 32-bit multiplication
# overflow, and noinline inherited from a prototype.
set -euo pipefail
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
CCC=$(realpath "${CCC:-$root/target/fastbuild/lccc}")
CCC_I686=$(realpath "${CCC_I686:-$root/target/fastbuild/lccc-i686}")
[[ -x $CCC ]] || { echo "missing x86-64 LCCC: $CCC" >&2; exit 1; }
[[ -x $CCC_I686 ]] || { echo "missing i686 LCCC: $CCC_I686" >&2; exit 1; }
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

tests=(
    aggregate_copy_snapshot_overwrite
    aggregate_global_alias_window
    builtin_shuffle_convention
    overflow_mul_narrow_result
    noinline_prototype_attribute
    noinline_comma_attribute
)
for arch in x86-64 i686; do
    cc=$CCC
    [[ $arch == i686 ]] && cc=$CCC_I686
    for test in "${tests[@]}"; do
        for opt in -O0 -O1 -O2 -O3 -Os; do
            exe=$work/$arch-$test$opt
            "$cc" "$opt" "$root/tests/regression/$test.c" -o "$exe"
            timeout 10 "$exe"
        done
        echo "PASS $arch $test (-O0/-O1/-O2/-O3/-Os)"
    done
done

# Runtime equality cannot establish the noinline property. Pin the call in
# each optimized caller, separately from the parser's declarator matrix.
for cc in "$CCC" "$CCC_I686"; do
    "$cc" -O2 -S "$root/tests/regression/noinline_comma_attribute.c" -o "$work/noinline.s"
    awk '/^caller:/ { inside=1 } inside { print } /^\.size[[:space:]]+caller,/ { inside=0 }' \
        "$work/noinline.s" > "$work/caller.s"
    grep -Eq '(call|jmp)(q|l)?[[:space:]]+kept(@PLT)?([[:space:]]|$)' "$work/caller.s" || {
        echo "FAIL: comma-declared noinline function was inlined by $cc" >&2
        cat "$work/caller.s" >&2
        exit 1
    }
done
