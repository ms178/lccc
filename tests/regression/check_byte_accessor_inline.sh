#!/usr/bin/env bash
# Runtime matrix + actual workload shape; does not relax an inline budget.
set -euo pipefail
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
CCC=${CCC:-$root/target/fastbuild/lccc}
GCC=${GCC:-gcc-16}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
for mode in -m64 -m32; do
    for opt in -O0 -O1 -O2 -O3 -Os; do
        "$GCC" "$mode" "$opt" -Wall -Wextra -Werror "$root/tests/regression/byte_accessor_inline.c" -o "$tmp/ref"
        "$CCC" "$mode" "$opt" "$root/tests/regression/byte_accessor_inline.c" -o "$tmp/mine"
        "$tmp/ref"
        "$tmp/mine"
    done
done
for opt in -O2 -O3; do
    "$CCC" "$opt" -march=x86-64-v3 -S "$root/tests/benchmark/programs/byte_load_reassembly.c" -o "$tmp/byte.s"
    if grep -Eq '^[[:space:]]*call[q]?[[:space:]]+body_le64' "$tmp/byte.s"; then
        echo 'FAIL: compact byte accessor still outlined in the measured workload' >&2
        exit 1
    fi
done
# The x86-64 early placement must not erase the established i686 placement.
for opt in -O2 -O3; do
    "$CCC" -m32 "$opt" -march=i686 -S "$root/tests/benchmark/programs/byte_load_reassembly.c" -o "$tmp/byte32.s"
    if grep -Eq '^[[:space:]]*call[q]?[[:space:]]+body_le64' "$tmp/byte32.s"; then
        echo 'FAIL: i686 compact byte accessor scheduling was lost' >&2
        exit 1
    fi
done
# Both diagnostic switches must suppress the extra scheduling opportunity.
# These are controls for ONE variable: whether byte scheduling is what makes
# body_le64 compact enough to inline. The inliner's capped-caller cleanup
# shrinks main() for unrelated reasons and can then admit the unscheduled
# accessor on its own budget, so the controls pin that path off; the positive
# O2/O3 assertions above run with every inliner feature enabled.
control=(env CCC_NO_INLINE_CALLER_CLEANUP=1)
for mode in -m64 -m32; do
    for disabled in byte_load postinline; do
        "${control[@]}" CCC_DISABLE_PASSES="$disabled" "$CCC" "$mode" -O2 -S "$root/tests/benchmark/programs/byte_load_reassembly.c" -o "$tmp/disabled.s"
        if ! grep -Eq '^[[:space:]]*call[q]?[[:space:]]+body_le64' "$tmp/disabled.s"; then
            echo "FAIL: byte scheduling ignored disabled pass $disabled on $mode" >&2
            exit 1
        fi
    done
done
# The new early-only switch is separate from the preserved i686 placement.
"${control[@]}" CCC_DISABLE_PASSES=byte_load_early "$CCC" -m64 -O2 -S "$root/tests/benchmark/programs/byte_load_reassembly.c" -o "$tmp/early64.s"
if ! grep -Eq '^[[:space:]]*call[q]?[[:space:]]+body_le64' "$tmp/early64.s"; then
    echo 'FAIL: x86-64 early byte scheduling ignored its own switch' >&2
    exit 1
fi
"${control[@]}" CCC_DISABLE_PASSES=byte_load_early "$CCC" -m32 -O2 -S "$root/tests/benchmark/programs/byte_load_reassembly.c" -o "$tmp/early32.s"
if grep -Eq '^[[:space:]]*call[q]?[[:space:]]+body_le64' "$tmp/early32.s"; then
    echo 'FAIL: early-only switch suppressed the separate i686 placement' >&2
    exit 1
fi
"$CCC" -O2 -S "$root/tests/regression/byte_accessor_inline.c" -o "$tmp/noinline.s"
if ! grep -Eq '^[[:space:]]*call[q]?[[:space:]]+read_noinline' "$tmp/noinline.s"; then
    echo 'FAIL: byte scheduling ignored noinline' >&2
    exit 1
fi
echo 'PASS: byte accessor bounds/order/store matrix: 10 LCCC + 10 GCC executions; O2/O3 inline, noinline and disable controls'
