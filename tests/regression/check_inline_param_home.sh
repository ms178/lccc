#!/usr/bin/env bash
# The inliner's argument bridge has to put each store where the callee will
# actually read the home -- and that is not the same position for every body.
#
# A memory-passed parameter (`__int128`, `__float128`, a vector, a struct) is
# read straight out of its home in the entry block, in the middle of the
# prologue: `baz` below loads its `__int128` home at instruction 2 while the
# callee's LAST alloca is instruction 3.  A bridge inserted after "the allocas"
# (the first run of them, or the last one) lands after that read, so the switch
# reads a slot nothing has written yet: `baz(0)` returned 0 instead of 1 and the
# test aborted.  The store now goes immediately after the alloca that defines
# ITS home, and a site that cannot satisfy "after the definition, before the
# first read" declines the inline instead of emitting a use before its
# definition.
#
# The same bodies pin the second half: a synthesized home must own the whole
# value.  An `__int128`, `__float128` or vector parameter is 16 bytes, and a
# scalar-sized slot truncates it (or, with a wider store, overwrites its
# neighbours), which is what made the vector and float128 rows diverge.
#
# Rows, per program and per level: lccc must agree with gcc, AND with lccc's
# own `CCC_DISABLE_PASSES=inline` build -- the second comparison is what makes a
# row evidence about the inliner rather than about the rest of the pipeline.  A
# coverage row asserts the inliner really does fire on the i128 kernel, so the
# agreement is not vacuous.
#
# Usage: bash tests/regression/check_inline_param_home.sh
#        CCC=/path/to/lccc bash tests/regression/check_inline_param_home.sh
set -eu

CCC=${CCC:-target/fastbuild/lccc}
GGC=${GCC:-gcc}
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

fail=0
note() { printf '  %-6s %s\n' "$1" "$2"; }

# `__int128` switch, `__float128` conversions, 8-byte vector ABI: one program
# per memory-passed parameter class.
PROGRAMS="switch_i128_high_half float128_usual_conversions vec8_abi_sse1_only"
LEVELS="-O1 -O2 -O3 -Os"

for program in $PROGRAMS; do
    source="tests/regression/$program.c"
    "$GGC" -O2 "$source" -o "$TMP/gcc.bin"
    rc_ref=0; out_ref=$("$TMP/gcc.bin") || rc_ref=$?
    for level in $LEVELS; do
        "$CCC" "$level" "$source" -o "$TMP/lccc.bin"
        rc_new=0; out_new=$("$TMP/lccc.bin") || rc_new=$?
        env CCC_DISABLE_PASSES=inline "$CCC" "$level" "$source" -o "$TMP/noinline.bin"
        rc_off=0; out_off=$("$TMP/noinline.bin") || rc_off=$?
        if [ "$out_new" = "$out_ref" ] && [ "$rc_new" = "$rc_ref" ] \
            && [ "$out_new" = "$out_off" ] && [ "$rc_new" = "$rc_off" ]; then
            note PASS "$program $level: lccc = inline-disabled = gcc"
        else
            note FAIL "$program $level: lccc(rc=$rc_new) = '$out_new', inline-disabled(rc=$rc_off) = '$out_off', gcc(rc=$rc_ref) = '$out_ref'"
            fail=1
        fi
    done
done

# Coverage: without this row every PASS above could be about a pipeline that
# never inlines the parameter at all.
inlined=$(CCC_INLINE_DEBUG=1 "$CCC" -O2 -S tests/regression/switch_i128_high_half.c \
    -o /dev/null 2>&1 | grep -c "Inlined 'baz' into 'main'" || true)
if [ "$inlined" -ge 1 ]; then
    note PASS "the i128 parameter is inlined (${inlined} sites), so the rows above cover the bridge"
else
    note FAIL "the inliner did not inline 'baz' into 'main'; the rows above are vacuous"
    fail=1
fi

echo
if [ "$fail" -eq 0 ]; then
    echo "INLINE PARAMETER HOME PASS"
else
    echo "INLINE PARAMETER HOME FAIL"
fi
exit "$fail"
