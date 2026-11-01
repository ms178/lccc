#!/usr/bin/env bash
# A linked PIE (ET_DYN) puts the image wherever the loader likes; code that
# assumes the image lives below 4 GiB runs correctly for years in ET_EXEC and
# dies the moment it is linked `-pie`.
#
# The shape that bit: the inliner bridged a call argument into the callee's
# "parameter alloca" chosen POSITIONALLY (the callee's first N allocas after
# cloning) instead of by data flow.  Once `mem2reg` folds `store param -> home;
# load home` into a direct `ParamRef` use and a later pass drops the now-dead
# home, the first N allocas are the callee's LOCALS -- so gzip's
# `load32` (`memcpy(&v, p, sizeof v)`) had its POINTER argument stored into
# `unsigned int v` with `ty: U32`, reloaded as `U32`, cast back to a pointer and
# therefore zero-extended from 32 bits on every use:
#
#   mov %r15d,%r15d        # zero-extend the address (Cast U32 -> Ptr)
#   mov (%r15),%r15d       # ... then dereference it: SIGSEGV at 0xaac8a202
#
# Rows below pin the two halves of the contract, both measured:
#   1. a `-pie`-linked build of the kernel runs and its own reference
#      implementation agrees with it, on a byte-identical `.o` to the `-fpie`
#      (ET_EXEC) build -- codegen must not assume a fixed image;
#   2. gcc, `-fpie` and `-fpie -pie` builds agree on stdout and exit status;
#   3. a struct-by-value corpus program (whose argument bridge is a `Memcpy` of
#      the whole object, not a `Store`) still runs under `-pie` -- the
#      data-flow home must not be confused with `memcpy(&local, p, n)`;
#   4. the IR verifier (`CCC_VERIFY_IR=1`), which found the dominance half of
#      the same malformed bridge, reports nothing for the kernel.
#
# Usage: bash tests/regression/check_pie_link_execution.sh
#        CCC=/path/to/lccc bash tests/regression/check_pie_link_execution.sh
set -eu

CCC=${CCC:-target/fastbuild/lccc}
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

GZIP=tests/benchmark/programs/gzip_longest_match.c
STRUCT=tests/benchmark/programs/struct_copy.c

fail=0
note() { printf '  %-6s %s\n' "$1" "$2"; }

run_output() { # binary -> stdout, exit status in RC
    RC=0
    OUT=$("$1" 2>/dev/null) || RC=$?
}

# ---- 1 + 2: the kernel, linked both ways ---------------------------------
# `-fpie` alone links ET_EXEC here (the driver only makes the link ET_DYN when
# told), which is exactly the pair that used to differ.
"$CCC" -O1 -march=x86-64-v3 -fpie -DPASSES=1 "$GZIP" -o "$TMP/gzip_pie.bin"
"$CCC" -O1 -march=x86-64-v3 -fpie -pie -DPASSES=1 "$GZIP" -o "$TMP/gzip_dyn.bin"
gcc -O1 -march=x86-64-v3 -fpie -pie -DPASSES=1 "$GZIP" -o "$TMP/gzip_gcc.bin"

pie_type=$(readelf -h "$TMP/gzip_dyn.bin" | awk '$1 == "Type:" { print $2 }')
exec_type=$(readelf -h "$TMP/gzip_pie.bin" | awk '$1 == "Type:" { print $2 }')
if [ "$pie_type" = "DYN" ] && [ "$exec_type" = "EXEC" ]; then
    note PASS "link modes are the pair under test: -fpie ${exec_type}, -fpie -pie ${pie_type}"
else
    note FAIL "unexpected ELF types: -fpie='$exec_type' -fpie -pie='$pie_type'"
    fail=1
fi

run_output "$TMP/gzip_dyn.bin"
rc_dyn=$RC out_dyn=$OUT
if [ "$rc_dyn" -eq 0 ] && [ "${out_dyn##* }" = "OK" ]; then
    note PASS "ET_DYN kernel self-check passes: ${out_dyn#* }"
else
    note FAIL "ET_DYN kernel failed (rc=$rc_dyn): '$out_dyn'"
    fail=1
fi

run_output "$TMP/gzip_pie.bin"
rc_exec=$RC out_exec=$OUT
run_output "$TMP/gzip_gcc.bin"
rc_gcc=$RC out_gcc=$OUT
if [ "$out_dyn" = "$out_exec" ] && [ "$out_dyn" = "$out_gcc" ] \
    && [ "$rc_dyn" = "$rc_exec" ] && [ "$rc_dyn" = "$rc_gcc" ]; then
    note PASS "ET_DYN = ET_EXEC = gcc on stdout and status"
else
    note FAIL "outputs differ: dyn='$out_dyn' exec='$out_exec' gcc='$out_gcc'"
    fail=1
fi

# ---- 3: struct-by-value argument bridges ---------------------------------
for level in -O1 -O2; do
    "$CCC" "$level" -march=x86-64-v3 -fpie -pie -DN=2000 "$STRUCT" -o "$TMP/struct.bin"
    gcc "$level" -march=x86-64-v3 -fpie -pie -DN=2000 "$STRUCT" -o "$TMP/struct_gcc.bin"
    run_output "$TMP/struct.bin"
    rc_new=$RC out_new=$OUT
    run_output "$TMP/struct_gcc.bin"
    rc_ref=$RC out_ref=$OUT
    if [ "$out_new" = "$out_ref" ] && [ "$rc_new" = "$rc_ref" ]; then
        note PASS "struct-by-value under ${level} -pie matches gcc (${out_new})"
    else
        note FAIL "struct-by-value ${level} differs: '${out_new}' (rc=$rc_new) vs '${out_ref}' (rc=$rc_ref)"
        fail=1
    fi
done

# ---- 4: the verifier stays silent on the kernel --------------------------
CCC_VERIFY_IR=1 "$CCC" -O1 -march=x86-64-v3 -fpie -S "$GZIP" -o /dev/null \
    >"$TMP/verify.log" 2>&1 || true
violations=$(grep -c '^\[ir-verify\]' "$TMP/verify.log" || true)
if [ "$violations" -eq 0 ]; then
    note PASS "IR verifier reports no violation for the kernel"
else
    note FAIL "IR verifier reported ${violations} violation(s):"
    sed -n '1,5p' "$TMP/verify.log" | sed 's/^/         /'
    fail=1
fi

echo
if [ "$fail" -eq 0 ]; then
    echo "PIE LINK EXECUTION PASS"
else
    echo "PIE LINK EXECUTION FAIL"
fi
exit "$fail"
