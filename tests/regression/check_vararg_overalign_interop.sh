#!/usr/bin/env bash
# Four-direction interop oracle for the GCC-4.6 over-aligned MEMORY-class
# vararg ABI (pr92904): lccc and system GCC are caller AND callee, so every
# combination must agree — lccc-caller/GCC-callee, GCC-caller/lccc-callee,
# and both monolithic builds. Alignment 32 and 64, anchor-parity sweeps of
# 0..20 leading stack slots, integer-only overaligned structs, a VLA
# (dynamic %rsp delta) before a realigned call, and the named-param side.
# Each linked binary runs 25x (the pre-fix bug was ASLR-dependent and
# aborted ~50% of runs).
set -euo pipefail

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
lccc=${CCC_X64:-$repo/target/fastbuild/lccc}
cc=${CC:-gcc}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
fail=0

compile_pair() { # $1 = .c file, $2 = tag ("l" or "g") -> $1_$2.o
    local src=$1 tag=$2 out=$3
    if [ "$tag" = l ]; then
        "$lccc" -O2 -c "$src" -o "$out"
    else
        "$cc" -O2 -c "$src" -o "$out"
    fi
}

run_matrix() { # $1 callee.c $2 caller.c $3 label
    local callee=$1 caller=$2 label=$3 ct gt rt
    compile_pair "$callee" l "$tmp/c_l.o"
    compile_pair "$callee" g "$tmp/c_g.o"
    compile_pair "$caller" l "$tmp/m_l.o"
    compile_pair "$caller" g "$tmp/m_g.o"
    for pair in "c_l m_l lccc/lccc" "c_g m_g gcc/gcc" "c_l m_g lccc-caller/gcc-callee" "c_g m_l gcc-caller/lccc-callee"; do
        set -- $pair
        "$cc" -O2 "$tmp/$1.o" "$tmp/$2.o" -o "$tmp/bin"
        local f=0 i
        for i in $(seq 1 25); do
            "$tmp/bin" >/dev/null 2>&1 || f=$((f+1))
        done
        if [ "$f" -ne 0 ]; then
            echo "FAIL: $label [$3]: $f/25 runs failed" >&2
            fail=1
        fi
    done
    echo "PASS: $label (4 combinations x 25 runs)"
}

run_matrix "$(dirname "$0")/interop/vararg_overalign_interop_callee.c" \
           "$(dirname "$0")/interop/vararg_overalign_interop_caller.c" "x86-64 interop"

[ "$fail" -eq 0 ] || exit 1
