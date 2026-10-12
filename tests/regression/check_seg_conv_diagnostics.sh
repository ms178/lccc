#!/usr/bin/env bash
# Implicit conversions between named and generic pointers, through identifier
# arrays, struct members of a __seg_gs object, subscripts of a __seg_gs array,
# arguments, assignments, returns and initializers.
#
# Every fixture in tests/regression/seg_conv/ is compiled with $CCC and with the
# GCC 14 oracle (see lib/gcc_oracle.sh). The file name is the expected verdict:
#   neg_*.c  must be REJECTED, with an address-space diagnostic;
#   pos_*.c  must be ACCEPTED.
#
# A verdict is only trusted from an ordinary compiler exit:
#   exit 0  = accepted;
#   exit 1  = rejected (the diagnostic-carrying error exit of both compilers);
#   anything else (a signal such as 139, or any other status) = a FAILURE, never
#   a rejection. A crash that happens to print an address-space message must not
#   count as the right rejection.
# A reject must also say why: its stderr needs an address-space diagnostic, so a
# fixture rejected for an unrelated reason fails the gate even when the verdict
# happens to match.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
dir=tests/regression/seg_conv
tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-seg-conv.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
# shellcheck source=tests/regression/lib/gcc_oracle.sh
. tests/regression/lib/gcc_oracle.sh
require_gcc_oracle "check_seg_conv_diagnostics" || exit 1
gcc_oracle_positive_control "$CCC" "$tmp" || exit 1
gcc_oracle_positive_control "$GCC_ORACLE_BIN" "$tmp" || exit 1
echo "Conversion oracle: $(basename "$GCC_ORACLE_BIN") (two compilers, positive controls)"

# compile_verdict <compiler> <flags> <fixture> <log>
# Prints accept|reject|crash:<rc> and leaves the diagnostics in <log>.
compile_verdict() {
    local rc
    set +e
    # shellcheck disable=SC2086  # <flags> is a deliberate word list
    "$1" $2 -c -o "$tmp/verdict.o" "$3" >"$4" 2>&1
    rc=$?
    set -e
    case "$rc" in
        0) echo accept ;;
        1) echo reject ;;
        *) echo "crash:$rc" ;;
    esac
}

fail=0
shopt -s nullglob
files=("$dir"/neg_*.c "$dir"/pos_*.c)
if [ "${#files[@]}" -eq 0 ]; then
    echo "FAIL: no fixtures in $dir" >&2
    exit 1
fi
for f in "${files[@]}"; do
    base=$(basename "$f")
    case "$base" in
        neg_*) want=reject ;;
        pos_*) want=accept ;;
    esac
    got=$(compile_verdict "$CCC" "" "$f" "$tmp/lccc.err")
    if [ "$got" != "$want" ]; then
        echo "FAIL: lccc $got $base (expected $want)" >&2
        grep -m2 . "$tmp/lccc.err" >&2 || true
        fail=1
        continue
    fi
    # Assignment, initializer and return conversions say "incompatible address
    # space"; an argument keeps the argument diagnostic (same rule as
    # check_seg_named_decl_qualifier.sh).
    if [ "$want" = reject ] && ! grep -q -e 'address space' -e 'incompatible pointer type for argument' "$tmp/lccc.err"; then
        echo "FAIL: lccc rejects $base without an address-space diagnostic" >&2
        grep -m2 . "$tmp/lccc.err" >&2 || true
        fail=1
        continue
    fi
    gg=$(compile_verdict "$GCC_ORACLE_BIN" "-w" "$f" "$tmp/gcc.err")
    if [ "$gg" != "$want" ]; then
        echo "FAIL: fixture $base expects $want but the GCC 14 oracle says $gg; fix the fixture" >&2
        grep -m2 . "$tmp/gcc.err" >&2 || true
        fail=1
        continue
    fi
    if [ "$want" = reject ] && ! grep -q -e 'address space' -e 'incompatible pointer type' "$tmp/gcc.err"; then
        echo "FAIL: the GCC 14 oracle rejects $base for a reason other than the address space" >&2
        grep -m2 . "$tmp/gcc.err" >&2 || true
        fail=1
    fi
done
if [ "$fail" -ne 0 ]; then
    echo "check_seg_conv_diagnostics: FAILED"
    exit 1
fi
echo "check_seg_conv_diagnostics: PASSED (${#files[@]} files; two compilers, positive controls)"
