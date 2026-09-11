#!/usr/bin/env bash
# Structural check for the over-aligned-alloca typed-census class.
#
# overalign_typed_census.c is the trip-wire: h() takes an over-aligned
# parameter, passes &x as a CALL ARGUMENT and reads x. Before the native
# over-aligned admission (S11), both shapes fell back to the mature path:
# the address argument rejected with `Call(arg-unrepresentable)` and the
# load with `ParamRef(over-aligned-alloca)`, splitting the MachInst run
# and flushing it around the call. The native form stages the align-up
# sequence IN the destination register (leaq+addq+andq), so the census
# stays at 100% for the function.
#
# Mutation-verified: against the pre-S11 compiler this file's census
# reports `rejected 1 Call(arg-unrepresentable)` (91.7% at -O1) and this
# check fails; execution stays correct either way (the fallback is
# fail-safe), which is exactly why the structural gate — not an output
# diff — is the regression lock for this class.
set -u
CCC=${CCC:-./target/fastbuild/lccc}
SRC=$(dirname "$0")/overalign_typed_census.c

OUT=$(CCC_ISEL_STATS=1 "$CCC" -O1 -c "$SRC" -o /tmp/oatc.$$ 2>&1)
rm -f /tmp/oatc.$$

if ! grep -q "lowered through MachInst" <<< "$OUT"; then
    echo "FAIL: probe did not flow through MachInst at all"
    exit 1
fi
if grep -q "rejected" <<< "$OUT"; then
    echo "FAIL: typed census rejected an instruction (over-aligned-alloca fallback flush regression?)"
    exit 1
fi
exit 0
