#!/usr/bin/env bash
# Structural companion to inline_loop_guard_hot_varint.c: execution alone
# cannot prove the atomic two-site -O2 clone or the -Os size-optimization gate.
set -euo pipefail

here=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
CCC=${CCC:-$here/../../target/fastbuild/lccc}
[[ -x $CCC ]] || { echo "loop-guard test: lccc not found at $CCC" >&2; exit 1; }

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# Count transfers to the decoder *within a single caller*, not the entire
# assembly. Jumps also represent out-of-line sibling calls.
transfers_to_decoder() { # transfers_to_decoder <asm> <caller>
    awk -v fn="$2" '
        $0 == fn ":" { inside = 1; next }
        inside && $1 == ".size" && $2 == fn "," { exit }
        inside { print }
    ' "$1" | grep -Ec '^[[:space:]]*(callq?|jmp|bl|b|jal|j|tail)[[:space:]]+sqlite_get_varint([[:space:]]|$)' || true
}

# The inliner is shared by the four supported backend drivers. Cross targets
# compile without host libc headers; no emulator or cross sysroot is needed
# to prove the -O2 clone / -Os refusal and IR invariants. The native and
# static-wrapper binaries additionally exercise the runtime recurrence.
for backend in x86_64 i686 arm riscv; do
    flags=()
    case $backend in
        x86_64) compiler=$CCC ;;
        i686) compiler=${CCC_I686:-$here/../../target/fastbuild/lccc-i686} ;;
        arm) compiler=${CCC_ARM:-$here/../../target/fastbuild/lccc-arm} ;;
        riscv) compiler=${CCC_RISCV:-$here/../../target/fastbuild/lccc-riscv} ;;
    esac
    [[ -x $compiler ]] || { echo "loop-guard test: missing $backend compiler at $compiler" >&2; exit 1; }
    if [[ $backend != x86_64 ]]; then
        flags=(-DLOOP_GUARD_FREESTANDING)
    fi
    for kind in exported static-wrapper; do
        kind_flags=("${flags[@]}")
        if [[ $kind == static-wrapper ]]; then
            kind_flags+=(-DLOOP_GUARD_STATIC_WRAPPERS)
        fi
        hot="$work/$backend.$kind.hot.s"
        small="$work/$backend.$kind.small.s"
        # If either IR verifier rejects a clone, compilation must fail.
        CCC_VERIFY_IR=abort CCC_VALIDATE_SSA=1 "$compiler" -O2 "${kind_flags[@]}" \
            -S "$here/inline_loop_guard_hot_varint.c" -o "$hot"
        "$compiler" -Os "${kind_flags[@]}" -S "$here/inline_loop_guard_hot_varint.c" -o "$small"

        for caller in hot_forward hot_backward; do
            hot_calls=$(transfers_to_decoder "$hot" "$caller")
            small_calls=$(transfers_to_decoder "$small" "$caller")
            if [[ $hot_calls != 0 || $small_calls != 1 ]]; then
                echo "loop-guard test ($backend/$kind): $caller has $hot_calls decoder transfers at -O2 (want 0), $small_calls at -Os (want 1)" >&2
                exit 1
            fi
        done
        # No spurious static stub should remain once both sites are cloned.
        if grep -Eq '^sqlite_get_varint:' "$hot"; then
            echo "loop-guard test ($backend/$kind): dead decoder still emitted at -O2" >&2
            exit 1
        fi
        if ! grep -Eq '^sqlite_get_varint:' "$small"; then
            echo "loop-guard test ($backend/$kind): missing outlined decoder at -Os" >&2
            exit 1
        fi
        # The corpus compares the default native form with GCC. Run the
        # static-wrapper variant too: its own harness checks all nine byte
        # lengths and both traversals against the independent recurrence.
        if [[ $backend == x86_64 && $kind == static-wrapper ]]; then
            "$compiler" -O2 "${kind_flags[@]}" "$here/inline_loop_guard_hot_varint.c" -o "$work/static.bin"
            "$work/static.bin" > /dev/null
        fi
    done
done
echo 'loop-guard test: x86-64/i686/ARM/RISC-V clone both hot shapes at -O2, none at -Os; dead bodies removed'
