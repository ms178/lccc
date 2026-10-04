#!/usr/bin/env bash
# `Add(symbol, register)` bases fold into ONE memory operand: `sym(%off,%idx)`.
#
# The sliding-window shape `const u8 *m = window + cur; m[i]` is where GCC and
# LCCC differ most (see WORKLOAD_PROVENANCE.md, RA-01): GCC keeps the symbol as
# the memory operand's displacement with BOTH the window offset and the index
# in registers; LCCC used to materialise `window + cur` with a LEA and address
# through two registers.  The fold that removes that LEA is only worth keeping
# if it (a) fires where it pays, (b) strictly shrinks the function, (c) changes
# no program's meaning on any path, and (d) is refused exactly where it must
# be — under PIC, whose SIB has no third register slot for the symbol.
#
# "Where it pays" is a measured contract, not a slogan: the fold is taken only
# when the deadening pass removed the `Add` because EVERY one of its uses is a
# fold of this arm.  A still-live `Add` (RA-01's kernel: the full compare
# reaches `match[0]` through the constant-offset path) keeps the plain
# register-base form — the symbol form there would grow the offset register's
# live range for zero instructions saved.  Row 2 pins that: the RA-01 kernel
# must keep folding to the SAME body as with the kill switch.
#
# Every claim below is measured, not asserted by construction:
#   1. the win-shape build contains the symbol+two-register operand, is
#      strictly smaller than the kill-switch build of the same compiler, and
#      prints what gcc prints;
#   2. the RA-01 kernel's body is byte-identical in size to its kill-switch
#      build (the fold is dormant when it cannot pay) and its stdout matches
#      gcc and the kill switch;
#   3. a LOOP that both loads and stores through such a base keeps its exit
#      code across folded / kill-switch / gcc;
#   4. `-fpie` emits none of the form (the documented PIC refusal).
#
# Usage: bash tests/regression/check_reg_off_sym_fold.sh
#        CCC=/path/to/lccc bash tests/regression/check_reg_off_sym_fold.sh
set -eu

CCC=${CCC:-target/fastbuild/lccc}
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

RA01=tests/benchmark/programs/ra01_global_match.c
FLAGS="-O2 -march=x86-64-v3 -fno-pie"
# Absolute symbol displacements only link into a non-PIE executable; the
# compile-only steps keep `-fno-pie` and every link step adds `-no-pie`
# (gcc's distro default is PIE, which would reject R_X86_64_32S against the
# probe's own symbols).
LINK_FLAGS="$FLAGS -no-pie"

# A symbol displacement followed by two register operands (`window(%r8, %rsi)`),
# optionally with a composed `+K` displacement.  The plain symbol form has no
# register before the comma; the register-base form has no symbol before the
# parenthesis — this pattern matches only the register-offset symbol fold.
SHAPE='[A-Za-z_.$][A-Za-z0-9_.$]*(\+[0-9]+)?\(%r[a-z0-9]+, %r[a-z0-9]+\)'

count_shape() { grep -Ec "$SHAPE" "$1" || true; }

# Instruction lines between the function label and its `.size` directive.
body_insns() {
    awk -v fn="$2" '
        $0 ~ "^" fn ":" { inb = 1; next }
        inb && /^[[:space:]]*\.size[[:space:]]/ { exit }
        inb && /^[[:space:]]+[a-z][a-z0-9.]*[[:space:]]/ { n++ }
        END { print n + 0 }
    ' "$1"
}

fail=0
note() { printf '  %-6s %s\n' "$1" "$2"; }

# ---- 1: the shape the fold exists for ------------------------------------
cat >"$TMP/win_shape.c" <<'EOF'
/* `const u8 *m = window + cur; m[i]`: the whole point of the fold.  Both
   uses of the derived pointer are indexed accesses, so the base `Add` is
   consumed entirely at the accesses and must never materialise. */
unsigned char win_window[8192];

unsigned
win_probe(unsigned cur, unsigned i, unsigned j)
{
    const unsigned char *m = win_window + cur;
    return m[i] + m[j];
}
EOF
"$CCC" $FLAGS -S "$TMP/win_shape.c" -o "$TMP/win.s"
env CCC_NO_REG_OFF_SYM=1 "$CCC" $FLAGS -S "$TMP/win_shape.c" -o "$TMP/win_nofold.s"
cat >"$TMP/win_main.c" <<'EOF'
#include <stdio.h>
unsigned win_probe(unsigned cur, unsigned i, unsigned j);
extern unsigned char win_window[8192];
int main(void)
{
    unsigned acc = 0;
    for (unsigned i = 0; i < 8192; i++)
        win_window[i] = (unsigned char)(i * 13 + 5);
    for (unsigned cur = 0; cur < 1024; cur++)
        for (unsigned i = 0; i + 1 < 256; i += 7)
            acc += win_probe(cur, i, i + 129);
    printf("%u\n", acc);
    return 0;
}
EOF
"$CCC" $LINK_FLAGS "$TMP/win_shape.c" "$TMP/win_main.c" -o "$TMP/win.bin"
env CCC_NO_REG_OFF_SYM=1 "$CCC" $LINK_FLAGS "$TMP/win_shape.c" "$TMP/win_main.c" -o "$TMP/win_nofold.bin"
gcc $LINK_FLAGS "$TMP/win_shape.c" "$TMP/win_main.c" -o "$TMP/win_gcc.bin"

win_shape=$(count_shape "$TMP/win.s")
win_nofold_shape=$(count_shape "$TMP/win_nofold.s")
win_insns=$(body_insns "$TMP/win.s" win_probe)
win_nofold_insns=$(body_insns "$TMP/win_nofold.s" win_probe)

if [ "$win_shape" -ge 2 ]; then
    note PASS "win shape emits the symbol+two-register form (${win_shape}x)"
else
    note FAIL "win shape emitted ${win_shape} of the form (want >= 2)"
    fail=1
fi
if [ "$win_nofold_shape" -eq 0 ]; then
    note PASS "CCC_NO_REG_OFF_SYM=1 emits none of the form (negative control)"
else
    note FAIL "kill switch still emitted ${win_nofold_shape} of the form"
    fail=1
fi
if [ "$win_insns" -lt "$win_nofold_insns" ]; then
    note PASS "win-shape body strictly smaller: ${win_insns} < ${win_nofold_insns} instructions"
else
    note FAIL "win-shape body not smaller: ${win_insns} vs ${win_nofold_insns} instructions"
    fail=1
fi

out_fold=$("$TMP/win.bin")
out_nofold=$("$TMP/win_nofold.bin")
out_gcc=$("$TMP/win_gcc.bin")
if [ "$out_fold" = "$out_nofold" ] && [ "$out_fold" = "$out_gcc" ]; then
    note PASS "win-shape stdout identical: folded = kill switch = gcc (${out_fold})"
else
    note FAIL "win-shape stdout differs: folded='$out_fold' nofold='$out_nofold' gcc='$out_gcc'"
    fail=1
fi

# ---- 2: RA-01's kernel — dormant where it cannot pay ---------------------
"$CCC" $FLAGS -S "$RA01" -o "$TMP/ra01.s"
env CCC_NO_REG_OFF_SYM=1 "$CCC" $FLAGS -S "$RA01" -o "$TMP/ra01_nofold.s"
"$CCC" $LINK_FLAGS "$RA01" -o "$TMP/ra01.bin"
env CCC_NO_REG_OFF_SYM=1 "$CCC" $LINK_FLAGS "$RA01" -o "$TMP/ra01_nofold.bin"
gcc $LINK_FLAGS "$RA01" -o "$TMP/ra01_gcc.bin"
ra01_insns=$(body_insns "$TMP/ra01.s" global_match_probe)
ra01_nofold_insns=$(body_insns "$TMP/ra01_nofold.s" global_match_probe)
if [ "$ra01_insns" = "$ra01_nofold_insns" ]; then
    note PASS "RA-01 kernel unchanged where the fold cannot pay (${ra01_insns} instructions)"
else
    note FAIL "RA-01 kernel changed without gain: ${ra01_insns} vs ${ra01_nofold_insns}"
    fail=1
fi
ra01_out=$("$TMP/ra01.bin"); ra01_nofold_out=$("$TMP/ra01_nofold.bin"); ra01_gcc_out=$("$TMP/ra01_gcc.bin")
if [ "$ra01_out" = "$ra01_nofold_out" ] && [ "$ra01_out" = "$ra01_gcc_out" ]; then
    note PASS "RA-01 kernel stdout identical: folded = kill switch = gcc (${ra01_out})"
else
    note FAIL "RA-01 stdout differs: folded='$ra01_out' nofold='$ra01_nofold_out' gcc='$ra01_gcc_out'"
    fail=1
fi

# ---- 3: a loop that stores AND loads through such a base ------------------
cat >"$TMP/loop_shape.c" <<'EOF'
/* The store half of the window shape plus its load: a wrong base/index
   assignment in the SIB (or a dead base kept by the store path) shows up as
   a wrong result, not as an assembler error. */
unsigned char loop_window[8192];

unsigned
loop_probe(unsigned cur, unsigned n)
{
    unsigned char *m = loop_window + cur;
    unsigned acc = 0;
    for (unsigned i = 0; i + 1 < n; i++) {
        m[i] = (unsigned char)(m[i + 1] + i);
        acc += m[i] + m[i + 1];
    }
    return acc;
}

int
main(void)
{
    unsigned acc = 0;
    for (unsigned i = 0; i < 8192; i++)
        loop_window[i] = (unsigned char)(i * 7 + 1);
    for (unsigned cur = 0; cur < 512; cur += 3)
        acc += loop_probe(cur, 200);
    for (unsigned i = 0; i < 8192; i++)
        acc += loop_window[i];
    return (int)(acc & 0x7fffffff);
}
EOF
"$CCC" $LINK_FLAGS "$TMP/loop_shape.c" -o "$TMP/loop.bin"
env CCC_NO_REG_OFF_SYM=1 "$CCC" $LINK_FLAGS "$TMP/loop_shape.c" -o "$TMP/loop_nofold.bin"
gcc $LINK_FLAGS "$TMP/loop_shape.c" -o "$TMP/loop_gcc.bin"
rc_fold=0; "$TMP/loop.bin" || rc_fold=$?
rc_nofold=0; "$TMP/loop_nofold.bin" || rc_nofold=$?
rc_gcc=0; "$TMP/loop_gcc.bin" || rc_gcc=$?
if [ "$rc_fold" = "$rc_nofold" ] && [ "$rc_fold" = "$rc_gcc" ]; then
    note PASS "loop shape exit code identical: folded = kill switch = gcc (${rc_fold})"
else
    note FAIL "loop shape exit differs: folded=$rc_fold nofold=$rc_nofold gcc=$rc_gcc"
    fail=1
fi

# ---- 4: PIC's documented refusal -----------------------------------------
"$CCC" -O2 -march=x86-64-v3 -fpie -S "$RA01" -o "$TMP/pie.s"
pie_shape=$(count_shape "$TMP/pie.s")
if [ "$pie_shape" -eq 0 ]; then
    note PASS "-fpie emits none of the form (documented PIC refusal)"
else
    note FAIL "-fpie emitted ${pie_shape} symbol+two-register operands"
    fail=1
fi

echo
if [ "$fail" -eq 0 ]; then
    echo "REGISTER-OFFSET SYMBOL FOLD PASS"
else
    echo "REGISTER-OFFSET SYMBOL FOLD FAIL"
fi
exit "$fail"
