#!/usr/bin/env bash
# Execution-differential proof for the i686 select-diamond → cmov peephole.
#
# WHY: `fold_select_diamond_cmov` (cross-pollinated from the x86-64 cmov select
# lowering) rewrites a branch diamond into a conditional move. Assembly
# inspection cannot prove the rewrite preserves both arms on every path, so
# this gate EXECUTES a freestanding select-heavy corpus (no libc / no 32-bit
# sysroot required) and compares lccc's 8-byte checksum against GCC's at four
# optimization levels. It also runs lccc with the fold DISABLED
# (CCC_NO_I686_SELECT_CMOV=1) to prove (a) the pre-fold baseline was already
# correct and (b) the fold is semantics-preserving, and asserts the fold
# actually fires at -O2 (a `cmov` appears) so the test cannot pass vacuously.
#
# Usage: CCC=<lccc> [GCC=gcc-16] bash tests/regression/check_i686_select_cmov.sh
set -uo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
GCC=${GCC:-gcc-16}
command -v "$GCC" >/dev/null 2>&1 || GCC=gcc
SRC="$(dirname "$0")/i686_select_cmov_diff.c"
TMP=${TMPDIR:-/tmp}/lccc-selcmov.$$
trap 'rm -rf "$TMP"' EXIT
mkdir -p "$TMP"
fail=0

CFLAGS="-m32 -nostdlib -ffreestanding -fno-stack-protector -fno-pic"

# run <binary> <outfile> — execute, capture raw stdout bytes.
run() { "$1" >"$2" 2>/dev/null; }

for opt in -O0 -O1 -O2 -Os; do
    # GCC reference (static: the sandbox has no 32-bit dynamic loader).
    if ! "$GCC" $CFLAGS -static "$opt" "$SRC" -o "$TMP/gcc$opt" 2>"$TMP/gcc$opt.err"; then
        echo "SKIP $opt: GCC cannot build freestanding m32 ($(head -1 "$TMP/gcc$opt.err"))"
        continue
    fi
    run "$TMP/gcc$opt" "$TMP/gcc$opt.out"

    # LCCC with the fold enabled.
    if ! "$CCC" $CFLAGS "$opt" "$SRC" -o "$TMP/lccc$opt" 2>"$TMP/lccc$opt.err"; then
        echo "FAIL $opt: lccc compile failed:"; head -5 "$TMP/lccc$opt.err"; fail=1; continue
    fi
    run "$TMP/lccc$opt" "$TMP/lccc$opt.out"
    if cmp -s "$TMP/gcc$opt.out" "$TMP/lccc$opt.out"; then
        echo "PASS $opt: lccc == gcc ($(od -An -tx1 "$TMP/lccc$opt.out" | tr -d ' \n'))"
    else
        echo "FAIL $opt: lccc != gcc"; echo "  gcc:  $(od -An -tx1 "$TMP/gcc$opt.out")"
        echo "  lccc: $(od -An -tx1 "$TMP/lccc$opt.out")"; fail=1
    fi

    # LCCC with BOTH cross-pollinated folds DISABLED must also match: this is
    # the pre-cross-pollination baseline, so a mismatch here would be a
    # pre-existing miscompile, and a match proves the folds preserve semantics.
    if CCC_NO_I686_SELECT_CMOV=1 CCC_NO_I686_LOAD_COPY_RELAY=1 \
        "$CCC" $CFLAGS "$opt" "$SRC" -o "$TMP/nofold$opt" 2>/dev/null; then
        run "$TMP/nofold$opt" "$TMP/nofold$opt.out"
        if cmp -s "$TMP/gcc$opt.out" "$TMP/nofold$opt.out"; then
            echo "PASS $opt: folds-disabled lccc == gcc"
        else
            echo "FAIL $opt: folds-disabled lccc != gcc (baseline miscompile?)"; fail=1
        fi
    fi
done

# The fold must actually fire at -O2: cmov present with the fold, absent (or
# reduced) without it. This guards against a vacuous pass.
"$CCC" $CFLAGS -O2 -S "$SRC" -o "$TMP/on.s" 2>/dev/null
CCC_NO_I686_SELECT_CMOV=1 "$CCC" $CFLAGS -O2 -S "$SRC" -o "$TMP/off.s" 2>/dev/null
on=$(grep -c 'cmov' "$TMP/on.s" 2>/dev/null || true)
off=$(grep -c 'cmov' "$TMP/off.s" 2>/dev/null || true)
if [ "${on:-0}" -gt "${off:-0}" ]; then
    echo "PASS fold-fires: cmov count on=$on off=$off"
else
    echo "FAIL fold-fires: expected more cmov with the fold (on=$on off=$off)"; fail=1
fi

if [ "$fail" -eq 0 ]; then echo "ALL PASS"; else echo "FAILURES PRESENT"; fi
exit "$fail"
