#!/usr/bin/env bash
# Descending-loop spelling probe: which C spellings of a descending counter
# produce a `BasicIV`, what recurrence the frontend actually emits for it, and
# whether IVSR then collects a derived expression to reduce.
#
# WHY THIS EXISTS.  A shipped claim in
# engineering/FOLLOWUP-2026-10-07-ivsr-review-hardening.md §4.1 said "no phi is
# ever recognised, so no descending loop in any C program gets an IV recurrence".
# It was generalised from ONE spelling (`i-- > 0`).  An external audit (finding
# F1) called that out without being able to compile anything; this script is the
# measurement that settles it, and it is re-runnable so the claim stays settled.
#
# RESULT (2026-10-07, base 9d85b134, fastbuild): 10 of 16 spellings DO form a
# BasicIV.  For a narrow unsigned counter the frontend zero-extends the decrement
# addend, so the recorded step is +2^32-1 rather than -1, and `i > 0`
# canonicalises to `Ne`; both make `unsigned_iv_bound` refuse the loop, which is
# why A1's descending arm stays unreachable from C.  No spelling produced a
# derived expression at all, so no descending recurrence fired -- for a reason
# that has nothing to do with phi recognition.  See §2 of
# engineering/FOLLOWUP-2026-10-07-ivsr-audit-response.md.
#
# A row that says COMPILE-FAIL is a bug in THIS script, not a result: the probe
# must never report an empty trace as if it were a measurement.
#
# Usage: LCCC=target/fastbuild/lccc bash descending-spelling-probe.sh
set -uo pipefail

LCCC=${LCCC:-target/fastbuild/lccc}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# name|counter type|loop+body (T is the counter/array element type)
cases=(
  's01_u32_postdec|unsigned|for (T i = n; i-- > 0;) s += p[i];'
  's02_u32_minuseq|unsigned|for (T i = n; i > 0; i -= 1) s += p[i - 1];'
  's03_u32_pluseq_neg|unsigned|for (T i = n; i > 0;) { i += -1; s += p[i]; }'
  's04_u32_plus_neg|unsigned|for (T i = n; i > 0;) { i = i + -1; s += p[i]; }'
  's05_u32_plus_umax|unsigned|for (T i = n; i > 0;) { i = i + (T)-1; s += p[i]; }'
  's06_u32_plus_lit|unsigned|for (T i = n; i > 0;) { i = i + 0xFFFFFFFFu; s += p[i]; }'
  's07_u32_zero_min1|unsigned|for (T i = n; i > 0;) { i = i + (0u - 1u); s += p[i]; }'
  's08_u32_neg_cast|unsigned|for (T i = n; i > 0;) { i += -(T)1; s += p[i]; }'
  's09_u32_lit_init|unsigned|for (T i = 64; i > 0;) { i += -1; s += p[i]; }'
  's10_i32_postdec|int|for (T i = (T)n; i-- > 0;) s += p[i];'
  's11_i32_pluseq_neg|int|for (T i = (T)n; i > 0;) { i += -1; s += p[i]; }'
  's12_u64_pluseq|unsigned long long|for (T i = n; i > 0;) { i += (T)-1; s += p[i]; }'
  's13_u16_pluseq|unsigned short|for (T i = (T)n; i > 0;) { i += (T)-1; s += p[i]; }'
  's14_u8_pluseq|unsigned char|for (T i = (T)n; i > 0;) { i += (T)-1; s += p[i]; }'
  's15_u64_walk_back|unsigned long long|for (T i = n; i > 0;) { i += (T)-1; s += p[i - 1]; }'
  's16_u8_stride1|unsigned char|for (T i = (T)n; i > 0;) { i += (T)-1; s += p[i]; }'
)

printf '%-22s %-9s %-62s %s\n' SPELLING BASIC_IV 'RECURRENCE / HEADER TEST' DERIVED
for c in "${cases[@]}"; do
  IFS='|' read -r name ty body <<<"$c"
  src="$work/$name.c"
  # The accumulator is deliberately wider than the counter: this probes IV
  # recognition, and a narrow accumulator would introduce its own wraparound.
  {
    printf 'typedef %s T;\n' "$ty"
    printf 'unsigned long long f(const T *p, unsigned n) {\n'
    printf '    unsigned long long s = 0;\n    %s\n    return s;\n}\n' "$body"
  } >"$src"

  if ! CCC_IVSR_DEBUG=1 "$LCCC" -O2 -S -o /dev/null "$src" >"$work/$name.ok" 2>"$work/$name.err"; then
    printf '%-22s %-9s %-62s %s\n' "$name" COMPILE-FAIL "$(head -c 60 "$work/$name.err" | tr '\n' ' ')" -
    continue
  fi
  trace=$(grep -E '^\[IVSR\] header' "$work/$name.err" | head -2 | tr '\n' ' ')
  iv=no
  [[ $trace == *'ivs='* ]] && iv=yes
  derived=$(grep -oE 'no derived exprs|derived=\[[^]]*\]' <<<"$trace" | head -1)
  [[ -z $derived ]] && derived='-'

    # Compact form: the CONSTANT-STEP recurrence (the only shape
  # `find_basic_ivs` accepts -- value-valued adds are the accumulator, not the
  # counter) and the header test, which are the two facts every proof gate reads.
  CCC_DUMP_IR=1 "$LCCC" -O2 -S -o /dev/null "$src" >/dev/null 2>"$work/$name.ir"
  rec=$(sed -nE 's/.*op: (Add|Sub), lhs: Value\(Value\([0-9]+\)\), rhs: (Const\([A-Za-z0-9()-]+\)), ty: ([A-Za-z0-9]+).*/\1 \2 \3/p;
                 s/.*op: (Ne|Eq|Sgt|Ugt|Ult|Sle|Ule|Sge|Uge), lhs: Value\(Value\([0-9]+\)\), rhs: Const\([A-Za-z0-9()-]+\), ty: ([A-Za-z0-9]+).*/cmp \1 \2/p' \
    "$work/$name.ir" | sort -u | tr '\n' ';')
  [[ -z $rec ]] && rec='(no matching recurrence)'
  printf '%-22s %-9s %-62s %s\n' "$name" "$iv" "$rec" "$derived"
done
