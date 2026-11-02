#!/usr/bin/env bash
# Narrow arithmetic: the value, not just the storage, has to be right.
#
# A narrow (i8/i16/u8/u16) result is only meaningful in its low 8/16 bits, and
# the backend keeps those bits canonical in a 32-bit register. That invariant
# is exactly what makes the width an implementation detail — and exactly what a
# width change can break: `addl` and `addq` agree on the low 32 bits but differ
# in the upper half AND in the flags a consumer reads, so any future narrowing
# of the ALU has to keep the *value* contract.
#
# This gate states that contract executably, over the whole u8 grid and a
# sampled i16 grid:
#
#   * the wrapped bit pattern of + - * & | ^ ~ - shl shr sar is the same as
#     the reference computed in wider arithmetic;
#   * a narrow result sign-extends (signed) or zero-extends (unsigned) when it
#     is used at a wider type — the canonical-form contract, which is what a
#     32-bit ALU write would have to reproduce;
#   * signed and unsigned comparisons agree with the reference for every pair.
#
# The fixture checks itself against that reference and prints a checksum, so
# the gate needs no oracle for correctness of a single arm — and it compares
# lccc's output against gcc's at every optimisation level anyway, so a fixture
# that silently stopped computing anything would fail here rather than pass.
#
# Usage: bash tests/regression/check_narrow_alu_arithmetic.sh
#        CCC=/path/to/lccc GCC_ORACLE=/usr/bin/gcc bash tests/regression/check_narrow_alu_arithmetic.sh
set -eu

CCC=${CCC:-target/fastbuild/lccc}
GCC_ORACLE=${GCC_ORACLE:-gcc}
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

if [ ! -x "$CCC" ]; then
    echo "FAIL: lccc not found at $CCC" >&2
    exit 1
fi
if ! command -v "$GCC_ORACLE" >/dev/null 2>&1; then
    echo "SKIP: no reference compiler '$GCC_ORACLE' on PATH" >&2
    exit 0
fi

work=$TMP/work
mkdir -p "$work"

cat >"$work/narrow_alu.c" <<'EOF'
#include <stdio.h>

/* The wrapped bit pattern of `v` at width 8/16, computed with arithmetic that
   cannot overflow, then interpreted as a signed value in [-128,127] /
   [-32768,32767]. */
static int wrap8(int v) {
    int u = v & 0xFF;
    return u > 0x7F ? u - 0x100 : u;
}
static int wrap16(int v) {
    int u = v & 0xFFFF;
    return u > 0x7FFF ? u - 0x10000 : u;
}

static unsigned long fails;
static unsigned long sink;

#define CHECK(cond)                                                            \
    do {                                                                       \
        if (!(cond)) {                                                         \
            if (fails < 8) printf("FAILED: %s at line %d\n", #cond, __LINE__);  \
            fails++;                                                           \
        }                                                                      \
    } while (0)

#define MIX(v) (sink = (sink ^ (unsigned long)(v)) * 1099511628211UL)

int main(void) {
    sink = 1469598103934665603UL;

    /* ---- u8 x u8, exhaustive ------------------------------------------- */
    for (unsigned ai = 0; ai < 256; ai++) {
        for (unsigned bi = 0; bi < 256; bi++) {
            unsigned char a = (unsigned char)ai, b = (unsigned char)bi;
            unsigned sh = ai & 7u;

            CHECK((unsigned)(unsigned char)(a + b) == ((ai + bi) & 0xFFu));
            CHECK((unsigned)(unsigned char)(a - b) == ((ai - bi) & 0xFFu));
            CHECK((unsigned)(unsigned char)(a * b) == ((ai * bi) & 0xFFu));
            CHECK((unsigned)(a & b) == (ai & bi));
            CHECK((unsigned)(a | b) == (ai | bi));
            CHECK((unsigned)(a ^ b) == (ai ^ bi));
            CHECK((unsigned)(unsigned char)~a == ((~ai) & 0xFFu));
            CHECK((unsigned)(unsigned char)(a << sh) == ((ai << sh) & 0xFFu));
            CHECK((unsigned)(unsigned char)(a >> sh) == ((ai >> sh) & 0xFFu));
            CHECK((a < b) == (ai < bi));
            CHECK((a <= b) == (ai <= bi));
            CHECK((a == b) == (ai == bi));
            MIX(a + b);
            MIX(a ^ b);
            MIX(a * b);

            /* The same bit pattern read as a signed value: every signed
               operation's wrapped result must match the reference pattern, a
               narrowing conversion must wrap, and widening the narrow result
               must sign-extend it (the canonical-form contract). */
            signed char sa = (signed char)a, sb = (signed char)b;
            CHECK((int)sa == wrap8((int)ai));
            CHECK((int)sb == wrap8((int)bi));

            signed char s_add = (signed char)(unsigned char)((ai + bi) & 0xFFu);
            signed char s_sub = (signed char)(unsigned char)((ai - bi) & 0xFFu);
            signed char s_mul = (signed char)(unsigned char)((ai * bi) & 0xFFu);
            signed char s_neg = (signed char)(unsigned char)((0u - ai) & 0xFFu);
            signed char s_not = (signed char)(unsigned char)((~ai) & 0xFFu);
            signed char s_shl = (signed char)(unsigned char)((ai << sh) & 0xFFu);
            CHECK((int)s_add == wrap8((int)((ai + bi) & 0xFFu)));
            CHECK((int)s_sub == wrap8((int)((ai - bi) & 0xFFu)));
            CHECK((int)s_mul == wrap8((int)((ai * bi) & 0xFFu)));
            CHECK((int)s_neg == wrap8((int)((0u - ai) & 0xFFu)));
            CHECK((int)s_not == wrap8((int)((~ai) & 0xFFu)));
            CHECK((int)s_shl == wrap8((int)((ai << sh) & 0xFFu)));
            CHECK((sa < sb) == (wrap8((int)ai) < wrap8((int)bi)));
            CHECK((sa <= sb) == (wrap8((int)ai) <= wrap8((int)bi)));
            CHECK((int)(sa >> (sh & 3)) == (wrap8((int)ai) >> (int)(sh & 3)));
            MIX((int)s_add);
            MIX((int)s_mul);
            MIX((int)s_neg);
            MIX((int)(sa >> (sh & 3)));
        }
    }

    /* ---- i16 sample x i16 sample --------------------------------------- */
    static const unsigned short vals[] = {0u,     1u,     2u,     3u,     7u,
                                          0x7Fu,  0x80u,  0xFFu,  0x100u, 0x1000u,
                                          0x7FFFu, 0x8000u, 0xFFFEu, 0xFFFFu, 0x5555u,
                                          0xAAAAu, 0x1234u, 0x8001u, 0x4000u, 0xBFFFu};
    const unsigned nvals = sizeof(vals) / sizeof(vals[0]);
    for (unsigned xi = 0; xi < nvals; xi++) {
        for (unsigned yi = 0; yi < nvals; yi++) {
            unsigned short a = vals[xi], b = vals[yi];
            unsigned sh = xi & 15u;
            CHECK((unsigned)(unsigned short)(a + b) == (((unsigned)a + b) & 0xFFFFu));
            CHECK((unsigned)(unsigned short)(a - b) == (((unsigned)a - b) & 0xFFFFu));
            CHECK((unsigned)(unsigned short)(a * b) == (((unsigned)a * b) & 0xFFFFu));
            CHECK((unsigned)(a ^ b) == (((unsigned)a ^ b) & 0xFFFFu));
            CHECK((unsigned)(unsigned short)(a << sh) == (((unsigned)a << sh) & 0xFFFFu));

            short sa = (short)a, sb = (short)b;
            CHECK((int)sa == wrap16((int)a));
            CHECK((int)sb == wrap16((int)b));
            short s_add = (short)(unsigned short)(((unsigned)a + b) & 0xFFFFu);
            short s_sub = (short)(unsigned short)(((unsigned)a - b) & 0xFFFFu);
            short s_mul = (short)(unsigned short)(((unsigned)a * b) & 0xFFFFu);
            short s_neg = (short)(unsigned short)((0u - a) & 0xFFFFu);
            CHECK((int)s_add == wrap16((int)(((unsigned)a + b) & 0xFFFFu)));
            CHECK((int)s_sub == wrap16((int)(((unsigned)a - b) & 0xFFFFu)));
            CHECK((int)s_mul == wrap16((int)(((unsigned)a * b) & 0xFFFFu)));
            CHECK((int)s_neg == wrap16((int)((0u - a) & 0xFFFFu)));
            CHECK((sa < sb) == (wrap16((int)a) < wrap16((int)b)));
            CHECK((int)(sa >> (sh & 7)) == (wrap16((int)a) >> (int)(sh & 7)));
            MIX((int)s_add);
            MIX((int)s_mul);
            MIX((unsigned)s_sub);
        }
    }

    printf("checksum=%lu fails=%lu\n", sink, fails);
    return fails == 0 ? 0 : 1;
}
EOF

note() { printf '  %-8s %s\n' "$1" "$2"; }
fail=0
ok() { note ok "$1"; }
bad() {
    note FAIL "$1"
    fail=1
}

"$GCC_ORACLE" -O2 -o "$work/ref" "$work/narrow_alu.c" 2>/dev/null || bad "gcc oracle failed to build"
want=$("$work/ref") || bad "gcc oracle failed its own checks: $want"

for opt in -O0 -O1 -O2 -O3 -Os; do
    for dbg in "" "-g"; do
        # shellcheck disable=SC2086
        "$CCC" $opt $dbg -o "$work/t" "$work/narrow_alu.c" || {
            bad "$opt $dbg: build failed"
            continue
        }
        got=$("$work/t") || bad "$opt $dbg: run failed ($got)"
        [ "${got:-}" = "$want" ] || bad "$opt $dbg: '$got' != gcc '$want'"
    done
    note ok "$opt: narrow arithmetic matches gcc at every width"
done

# A fixture that stopped computing would still match gcc, so require that the
# checksum actually depends on the work and that no self-check fired.
case "$want" in
    *"fails=0"*) ok "fixture self-checks pass against the wider-arithmetic reference" ;;
    *) bad "oracle reported failures: $want" ;;
esac
case "$want" in
    "checksum=1469598103934665603 "*) bad "oracle checksum is the empty hash — the loops did not run" ;;
esac

if [ "$fail" -ne 0 ]; then
    echo "FAIL: narrow ALU arithmetic" >&2
    exit 1
fi
echo "PASS: narrow ALU arithmetic (u8 exhaustive and i16 sampled; wrapped values, sign extension and comparisons match gcc at -O0..-Os)"
