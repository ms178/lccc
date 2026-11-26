#!/usr/bin/env bash
# Surgical SQLite varint correctness and SSA rewrite/codegen regression.
set -euo pipefail
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
LCCC=${LCCC:-$ROOT/target/fastbuild/lccc}
GCC=${GCC:-gcc-16}
CLANG=${CLANG:-clang-23}
fixture="$ROOT/tests/workloads/sqlite-3.54.1/ptr_delta_phi.c"
negative_fixture="$ROOT/tests/workloads/sqlite-3.54.1/ptr_delta_phi_negative_bench.c"
negative_add_fixture="$ROOT/tests/workloads/sqlite-3.54.1/ptr_delta_phi_negative_add_bench.c"
# Do not allow the optimizer's own IR/SSA checker or a C diagnostic to be
# bypassed just because the binary happened to print the expected checksum.
export CCC_VERIFY_IR=abort CCC_VALIDATE_SSA=1
flags=(-std=c11 -O2 -march=x86-64-v3 -Wall -Wextra -Werror)
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT
for key in gcc lccc clang lccc-off; do
    case "$key" in
        gcc) cc=$GCC ;;
        clang) cc=$CLANG ;;
        *) cc=$LCCC ;;
    esac
    if [[ $key == lccc-off ]]; then
        CCC_DISABLE_PASSES=ptr_delta_phi timeout --kill-after=5s 120 \
            "$cc" "${flags[@]}" -S "$fixture" -o "$work/$key.s"
        CCC_DISABLE_PASSES=ptr_delta_phi timeout --kill-after=5s 120 \
            "$cc" "${flags[@]}" "$fixture" -o "$work/$key"
    else
        timeout --kill-after=5s 120 "$cc" "${flags[@]}" -S "$fixture" -o "$work/$key.s"
        timeout --kill-after=5s 120 "$cc" "${flags[@]}" "$fixture" -o "$work/$key"
    fi
    timeout --kill-after=5s 30 "$work/$key" > "$work/$key.out" 2> "$work/$key.err"
    [[ ! -s "$work/$key.err" ]] || { echo "unexpected varint stderr: $key" >&2; exit 1; }
    grep -q '^ptr_delta_phi: 90000 varints verified;' "$work/$key.out"
    cmp "$work/gcc.out" "$work/$key.out" || {
        echo "different answers for $key and GCC" >&2; exit 1;
    }
    # Native host regressions for BOTH backward forms: Sub(base, 7) and
    # GEP/Add(base, -7). Keep the timing loop short here; perf_ab.py separately
    # measures full-length pairs. GCC/Clang parity is mandatory in each mode.
    for shape in negative negative-add; do
        source=$negative_fixture
        [[ $shape == negative-add ]] && source=$negative_add_fixture
        if [[ $key == lccc-off ]]; then
            CCC_DISABLE_PASSES=ptr_delta_phi timeout --kill-after=5s 120 \
                "$cc" "${flags[@]}" -DBENCH_PASSES=1 "$source" -o "$work/$key-$shape"
        else
            timeout --kill-after=5s 120 \
                "$cc" "${flags[@]}" -DBENCH_PASSES=1 "$source" -o "$work/$key-$shape"
        fi
        timeout --kill-after=5s 30 "$work/$key-$shape" \
            > "$work/$key-$shape.out" 2> "$work/$key-$shape.err"
        [[ ! -s "$work/$key-$shape.err" ]] || {
            echo "unexpected $shape stderr: $key" >&2; exit 1;
        }
        cmp "$work/gcc-$shape.out" "$work/$key-$shape.out" || {
            echo "different $shape pointer-delta answers for $key and GCC" >&2; exit 1;
        }
    done
done
# The pass is never credited merely because an output is equal: show its
# mechanism in the isolated function, and prove the kill switch restores the
# former pointer-subtraction path. Keep the names ABI-specific to this x86 gate.
awk '/^sqlite_varint_delta:/{found=1} found {print} /^\.size sqlite_varint_delta,/{exit}' \
    "$work/lccc.s" > "$work/on.func"
awk '/^sqlite_varint_delta:/{found=1} found {print} /^\.size sqlite_varint_delta,/{exit}' \
    "$work/lccc-off.s" > "$work/off.func"
grep -q '^sqlite_varint_delta:' "$work/on.func"
if grep -Eq 'subq[[:space:]]+%rdi,' "$work/on.func"; then
    echo 'pointer difference survived the SSA transform' >&2; exit 1
fi
if ! grep -Eq 'subq[[:space:]]+%rdi,' "$work/off.func"; then
    echo 'disabled pass did not restore the baseline pointer subtraction' >&2; exit 1
fi
printf 'SQLite varint (90000) and both backward forms (4096 each) x GCC/LCCC/Clang/LCCC-off: PASS; forward-only pointer-delta fold: PASS\n'
