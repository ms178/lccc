#!/usr/bin/env bash
# Lowest-set-bit recognition and the `0 - x` negation canonicalisation.
#
# Two rules are pinned here, both from the SCALAR-PREDICATE-FOLDS backlog:
#
#   1. `1 << Ctz(y)` is `y & -y` for `y != 0`, and `1` for `y == 0` -- the
#      shift masks its count modulo the operand width, so `1 << W` wraps to
#      `1`, not to 0.  The fold therefore carries a `cmov` on the flags the
#      `and` already set; dropping it would be a miscompile the source cannot
#      show ( `__builtin_ctz(0)` is C undefined behaviour, but the IR's `Ctz`
#      is the zero-defined variant and must stay exact ).
#      On x86-64 this replaces
#          not / bts $32 / bsfq / mov $1 / mov / shl        (32-bit operands)
#          test / jnz / mov $64 / jmp / bsfq / ...          (64-bit operands)
#      with `not` (or `add`) / `and` / `cmov` in both cases.
#   2. `0 - x` folds to `Neg`: on x86 that is one `negl` instead of
#      `xorl` + `subl`, and it is what makes `x & (0u - x)` recognisable.
#
# The gate pins the emitted shapes, the ownership of both kill switches, and
# bit-exact agreement between -O0, -O2, -O3 and the disabled-fold builds over
# every boundary value, because an instruction-count win that changes answers
# is worthless.
set -euo pipefail
CCC=${LCCC_BIN:-${CCC:-target/fastbuild/lccc}}
tmp=${TMPDIR:-/tmp}/lccc-lowest-set-bit.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"

cat > "$tmp/shapes.c" <<'C'
unsigned l32(unsigned x) { return 1u << __builtin_ctz(~x); }
unsigned long long l64(unsigned long long x) { return 1ull << __builtin_ctzll(~x); }
unsigned lowbit(unsigned x) { return x & (0u - x); }
/* Negative control: a constant 2 is not the identity, so the count path must
   stay. */
unsigned two(unsigned x) { return 2u << __builtin_ctz(x); }
/* Negative control with a measured margin: the PLAIN count form is cheaper as
   it stands (bsf + staging + shl is 5 instructions at 32 bits and 6 at 64,
   against 8 for the isolated bit plus its zero arm), so it must keep its
   count in every build. A regression here is a pessimisation the shape legs
   above would not catch. */
unsigned plain32(unsigned y) { return 1u << __builtin_ctz(y); }
unsigned long long plain64(unsigned long long y) { return 1ull << __builtin_ctzll(y); }
C

"$CCC" -O2 -S "$tmp/shapes.c" -o "$tmp/shapes.s"
CCC_NO_LOWEST_SET_BIT=1 "$CCC" -O2 -S "$tmp/shapes.c" -o "$tmp/shapes_nolsb.s"
CCC_SIMPLIFY_SKIP=zeroneg "$CCC" -O2 -S "$tmp/shapes.c" -o "$tmp/shapes_nozeroneg.s"

fn_body() { # $1 = asm file, $2 = label; body between the label and ret
    sed -n "/^$2:/,/ret/p" "$1"
}
has() { # $1 = text, $2 = pattern
    grep -q -e "$2" <<<"$1"
}
lacks() { # $1 = text, $2 = pattern, $3 = message
    if grep -q -e "$2" <<<"$1"; then
        echo "$3: unexpected pattern '$2' in:" >&2
        echo "$1" >&2
        return 1
    fi
}

# ── Rule 1: the folded shapes ───────────────────────────────────────────────
body=$(fn_body "$tmp/shapes.s" l32)
# `-~x` is `x + 1`, so the complemented source keeps only the increment (either
# as an `add` or folded into a `lea` by the addressing pass).
if ! has "$body" 'addl \.\$1\|addl \$1\|leal 1('; then
    echo "l32 did not fold the negate into x + 1:" >&2
    echo "$body" >&2
    exit 1
fi
if ! has "$body" 'and[lq] %'; then
    echo "l32 lost the isolation AND:" >&2
    echo "$body" >&2
    exit 1
fi
if ! has "$body" 'cmov'; then
    echo "l32 must keep the zero arm as a cmov (1 << ctz(0) is 1, not 0):" >&2
    echo "$body" >&2
    exit 1
fi
lacks "$body" 'bsf' "l32 still counts trailing zeroes"
lacks "$body" 'bts' "l32 still pays the stop-bit fixup"
lacks "$body" '[[:space:]]shl[lq]\? ' "l32 still shifts"
lacks "$body" 'test[lq]\? %' "l32 re-tests flags the and already set"

# 64-bit: the whole point is that the test/jnz/jmp diamond for Ctz(0) is gone.
body=$(fn_body "$tmp/shapes.s" l64)
if ! has "$body" 'cmov'; then
    echo "l64 must keep the zero arm as a cmov:" >&2
    echo "$body" >&2
    exit 1
fi
lacks "$body" 'bsf' "l64 still counts trailing zeroes"
lacks "$body" 'jnz\|jmp' "l64 still carries the Ctz(0) diamond"
lacks "$body" '[[:space:]]shl[lq]\? ' "l64 still shifts"

# ── Rule 2: `0 - x` is `Neg` ────────────────────────────────────────────────
body=$(fn_body "$tmp/shapes.s" lowbit)
if ! has "$body" 'neg[lq]\? %'; then
    echo "lowbit lost the negate (0 - x must canonicalise to Neg):" >&2
    echo "$body" >&2
    exit 1
fi
lacks "$body" 'sub[lq]\? %' "lowbit still subtracts instead of negating"
body=$(fn_body "$tmp/shapes_nozeroneg.s" lowbit)
if ! has "$body" 'sub[lq]\? %'; then
    echo "control lowbit does not restore the subtraction:" >&2
    echo "$body" >&2
    exit 1
fi

# ── Kill-switch ownership ───────────────────────────────────────────────────
# With the lowest-set-bit rule off, the count path must come back; that is what
# proves the shapes above belong to this rule and not to an unrelated one.
body=$(fn_body "$tmp/shapes_nolsb.s" l32)
if ! has "$body" 'bsf\|shl'; then
    echo "CCC_NO_LOWEST_SET_BIT=1 does not restore the count path:" >&2
    echo "$body" >&2
    exit 1
fi
body=$(fn_body "$tmp/shapes_nolsb.s" l64)
if ! has "$body" 'bsf'; then
    echo "CCC_NO_LOWEST_SET_BIT=1 does not restore the 64-bit count:" >&2
    echo "$body" >&2
    exit 1
fi

# Negative control: the plain (non-complemented) count form must not be
# rewritten into the isolated-bit shape -- it is strictly cheaper as it stands.
for f in "$tmp/shapes.s" "$tmp/shapes_nolsb.s"; do
    for plain in plain32 plain64; do
        body=$(fn_body "$f" "$plain")
        if ! has "$body" 'bsf'; then
            echo "$plain lost its count path in $f (pessimisation):" >&2
            echo "$body" >&2
            exit 1
        fi
    done
done
n=$(fn_body "$tmp/shapes.s" plain32 | grep -c -e '^[[:space:]]\+[a-z]' || true)
[ "$n" -le 5 ] || { echo "plain32 grew to $n instructions" >&2; fn_body "$tmp/shapes.s" plain32 >&2; exit 1; }
n=$(fn_body "$tmp/shapes.s" plain64 | grep -c -e '^[[:space:]]\+[a-z]' || true)
[ "$n" -le 6 ] || { echo "plain64 grew to $n instructions" >&2; fn_body "$tmp/shapes.s" plain64 >&2; exit 1; }

# Negative control: a shift by a count that is not a bare Ctz must keep its
# shift in every build.
for f in "$tmp/shapes.s" "$tmp/shapes_nolsb.s"; do
    body=$(fn_body "$f" two)
    if ! has "$body" 'shl\|lea'; then
        echo "control 'two' lost its shift in $f:" >&2
        echo "$body" >&2
        exit 1
    fi
done

# ── Execution: every build must agree bit for bit ───────────────────────────
cat > "$tmp/exec.c" <<'C'
#include <stdio.h>

static unsigned l32(unsigned x) { return 1u << __builtin_ctz(~x); }
static unsigned long long l64(unsigned long long x) { return 1ull << __builtin_ctzll(~x); }
static unsigned lowbit(unsigned x) { return x & (0u - x); }

/* Rows whose source expression is C undefined behaviour (`~x == 0` makes the
   shift count equal the width) are skipped: -O0 evaluates them through the
   masked shift and -O2 through the constant folder, and the two legitimately
   differ on UB. The compiler's own DEFINED answer for that case is pinned by
   the IR unit tests (`zero_operand_yields_one_not_zero`), not by this
   -O0/-O2 comparison, which must only speak about defined programs. */
static unsigned long long skipped;
static void one(unsigned long long x) {
    if ((unsigned)~x == 0u || ~x == 0ull) {
        skipped++;
        return;
    }
    printf("%08x %016llx %08x\n", l32((unsigned)x), l64(x), lowbit((unsigned)x));
}

int main(void) {
    /* Zero, single bits, all ones and the neighbours of every boundary that
       can distinguish the zero arm from `y & -y`. */
    one(0);
    one(~0ull);
    one(~0ull - 1);
    for (unsigned bit = 0; bit < 64; bit++) {
        one(1ull << bit);
        one(~(1ull << bit));
    }
    for (unsigned i = 0; i <= 2048; i++) {
        one(i);
        one(~(unsigned long long)i);
        one(0xffffffffull - i);
    }
    one(0x7fffffffull);
    one(0x80000000ull);
    one(0x7fffffffffffffffull);
    one(0x8000000000000000ull);
    one(0xdeadbeefcafef00dull);
    one(0x0123456789abcdefull);
    printf("skipped %llu undefined rows\n", skipped);
    return 0;
}
C

ref=""
for opt in -O0 -O2 -O3; do
    "$CCC" $opt "$tmp/exec.c" -o "$tmp/e$opt"
    "$tmp/e$opt" > "$tmp/out$opt"
    if [ -z "$ref" ]; then
        ref="$tmp/out$opt"
    elif ! cmp -s "$ref" "$tmp/out$opt"; then
        echo "-O0 and $opt disagree on the folded answers:" >&2
        diff "$ref" "$tmp/out$opt" | sed -n '1,10p' >&2
        exit 1
    fi
done

# The kill switch may change the code, never the answers.
for knob in CCC_NO_LOWEST_SET_BIT=1 CCC_SIMPLIFY_SKIP=zeroneg; do
    env $knob "$CCC" -O2 "$tmp/exec.c" -o "$tmp/ek"
    "$tmp/ek" > "$tmp/outk"
    if ! cmp -s "$ref" "$tmp/outk"; then
        echo "$knob changes the answers:" >&2
        diff "$ref" "$tmp/outk" | sed -n '1,10p' >&2
        exit 1
    fi
done

# `1 << ctz(~x) == lowbit(~x)` must hold on the defined domain; only the
# y == 0 row is C undefined behaviour, so it is excluded here even though the
# compiler must still answer it exactly (the -O0/-O2/-O3 comparison covers it).
if command -v "${GCC_ORACLE:-gcc-16}" >/dev/null 2>&1; then
    cat > "$tmp/oracle.c" <<'C'
#include <stdio.h>
static unsigned l32(unsigned x) { return 1u << __builtin_ctz(~x); }
static unsigned lowbit(unsigned x) { return x & (0u - x); }
int main(void) {
    for (unsigned x = 0; x < 4096; x++) {
        if (~x == 0) continue;
        if (l32(x) != lowbit(~x)) { printf("BAD %u\n", x); return 1; }
    }
    for (unsigned bit = 0; bit < 32; bit++) {
        unsigned x = ~(1u << bit);
        if (l32(x) != lowbit(~x)) { printf("BAD bit %u\n", bit); return 1; }
    }
    puts("OK");
    return 0;
}
C
    "${GCC_ORACLE:-gcc-16}" -O2 "$tmp/oracle.c" -o "$tmp/oracle"
    out=$("$tmp/oracle")
    [ "$out" = "OK" ] || { echo "oracle identity check: $out" >&2; exit 1; }
    "$CCC" -O2 "$tmp/oracle.c" -o "$tmp/oracle_lccc"
    out=$("$tmp/oracle_lccc")
    [ "$out" = "OK" ] || { echo "lccc identity check: $out" >&2; exit 1; }
fi

# ── Instruction counts, printed so a drift is visible in the CI log ─────────
for name in l32 l64 lowbit plain32 plain64; do
    n=$(fn_body "$tmp/shapes.s" "$name" | grep -c -e '^[[:space:]]\+[a-z]' || true)
    echo "  $name: $n instructions"
done

echo "lowest-set-bit gate OK"
