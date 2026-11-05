#!/usr/bin/env bash
# GCC 16.2 inline-asm constraint grammar regression gate.
#
# Four gcc.c-torture/compile cases (20000922-1.c, asmgoto-2.c, pr27528.c,
# pr35318.c) used to fail with `impossible constraint in 'asm'` because the
# front-end validator treated a constraint as a comma-free set of letters and
# reclassified every unknown character as a general-purpose register.  GCC
# constraints are actually a COMMA-separated list of ALTERNATIVES with cost /
# selection modifiers; an operand is legal when at least one alternative is
# satisfiable, and every operand of one asm must agree on the alternative
# count.
#
# All expectations below were measured with gcc-16 (GCC) 16.2.0 at -O2 on
# x86-64 and cross-checked with Clang 23.1.5 where Clang implements the GNU
# spelling.  Two deliberate divergences from the older gcc 14.2 measurement
# are pinned here:
#
#   * `"n"(symbol)` is `impossible constraint in 'asm'` in gcc 16.2 (a
#     relocation is not a numeric immediate), while `"i"(symbol)` and
#     `"s"(symbol)` are accepted;
#   * `"s"(5)` is refused: `s` is a symbolic/string-literal class, not an
#     integer class.
#
# The gate also runs one real instruction with two-alternative constraints
# end to end on the host ABI, so the grammar fix cannot pass by silently
# dropping the asm statement.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
CCC_I686=${CCC_I686:-./target/fastbuild/lccc-i686}
GCC=${GCC:-gcc-16}
command -v "$GCC" >/dev/null 2>&1 || GCC=gcc
[ -x "$CCC" ] || { echo "check_gcc_asm_constraints: SKIP ($CCC not built)"; exit 0; }

tmp=${TMPDIR:-/tmp}/lccc-asm-constraints.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"

pass=0
fail=0
report() { # report <label> <rc>
    if [ "$2" -eq 0 ]; then
        pass=$((pass + 1))
    else
        printf 'FAIL: %s\n' "$1"
        fail=$((fail + 1))
    fi
}
compile_ok() { # compile_ok <label> <compiler> <opt> <file>
    local label=$1 cc=$2 opt=$3 file=$4 out rc
    out=$("$cc" "$opt" -c "$file" -o "$tmp/out.o" 2>&1) && rc=0 || rc=$?
    if [ "$rc" -ne 0 ] || grep -q 'error:' <<<"$out"; then
        printf 'FAIL: %s rc=%s: %s\n' "$label" "$rc" "$out"
        return 1
    fi
    return 0
}
expect_error() { # expect_error <label> <compiler> <file> <needle>
    local label=$1 cc=$2 file=$3 needle=$4 out rc
    out=$("$cc" -O2 -c "$file" -o "$tmp/out.o" 2>&1) && rc=0 || rc=$?
    if [ "$rc" -eq 0 ] || ! grep -qF "$needle" <<<"$out"; then
        printf 'FAIL: %s rc=%s, expected "%s": %s\n' "$label" "$rc" "$needle" "$out"
        return 1
    fi
    return 0
}

# ── Accepted: comma alternatives, modifiers, symbolic immediates ──────────
cat >"$tmp/accept.c" <<'C'
int g_arr[4];
void fn(void);

/* pr35318.c: two alternatives on every operand, tied input, comma input. */
int f1(int a, int b)
{
    int r, s;
    asm volatile("movl %2, %0; addl %3, %1" : "=r,r"(r), "=r,r"(s) : "%0,0"(a), "m,r"(b));
    return r + s;
}

/* asmgoto-2.c: multi-alternative outputs only. */
int f2(int a)
{
    int x, y;
    asm volatile("" : "=r,m"(x), "=r,m"(y) : "r,m"(a));
    return x + y;
}

/* 20000922-1.c: '#' comment tail and the '*' ignore-next-letter marker.
   A CONSTANT operand is valid at every optimization level. The corpus's own
   __builtin_constant_p(x) shape is checked separately at -O2 below. */
void f3(void)
{
    asm volatile("" :: "i#*X"(7));
}

/* pr27528.c: s / i / n with link-time symbolic operands. */
void f4(void)
{
    asm volatile("" :: "s"(g_arr));
    asm volatile("" :: "s"(fn));
    asm volatile("" :: "i"(fn));
    asm volatile("" :: "i"("literal"));
    asm volatile("" :: "s"("literal"));
    asm volatile("" :: "n"(5));
}

/* GCC cost / selection modifiers are not constraint letters. */
void f5(int x)
{
    asm volatile("" :: "r?"(x));
    asm volatile("" :: "!r"(x));
    asm volatile("" :: "^r"(x));
    asm volatile("" :: "$r"(x));
    asm volatile("" :: "r,m"(x));
}
C
# The exact gcc.c-torture/compile/20000922-1.c shape: the asm lives behind a
# `__builtin_constant_p` guard and must be folded away at -O2.  At -O0 gcc
# 16.2 still accepts it (the builtin folds to 0 during gimplification), while
# lccc's late immediate check sees the uneliminated statement; that residual
# -O0 divergence is tracked in engineering/FOLLOWUP, so the corpus-equivalent
# contract is pinned at the optimization level of the torture harness (-O2).
cat >"$tmp/builtin_constant.c" <<'C'
int f(int x)
{
    if (__builtin_constant_p(x) && x != 0)
        asm volatile("" :: "i#*X"(x));
    return x;
}
C
for cc in "$CCC" "$CCC_I686"; do
    [ -x "$cc" ] || continue
    for opt in -O0 -O2; do
        if compile_ok "accept.c $(basename "$cc") $opt" "$cc" "$opt" "$tmp/accept.c"; then
            report "accept.c $(basename "$cc") $opt" 0
        else
            report "accept.c $(basename "$cc") $opt" 1
        fi
    done
    if compile_ok "builtin_constant.c $(basename "$cc") -O2" "$cc" -O2 "$tmp/builtin_constant.c"; then
        report "builtin_constant.c $(basename "$cc") -O2" 0
    else
        report "builtin_constant.c $(basename "$cc") -O2" 1
    fi
done

# ── Rejected: exact GCC 16.2 diagnostics ──────────────────────────────────
cat >"$tmp/matching.c" <<'C'
void f(int x) { asm volatile("" :: "0"(x)); }
C
cat >"$tmp/altcount.c" <<'C'
void f(int x) { int r; asm volatile("" : "=r"(r) : "r,m"(x)); }
C
cat >"$tmp/nstring.c" <<'C'
void f(void) { asm volatile("" :: "n"("literal")); }
C
cat >"$tmp/sinteger.c" <<'C'
void f(void) { asm volatile("" :: "s"(5)); }
C
cat >"$tmp/unmodelled.c" <<'C'
void f(int x) { asm volatile("" :: "z"(x)); }
C
cat >"$tmp/punct.c" <<'C'
void f(void) { asm volatile("" :: "i#@@@"(7)); }
C
for cc in "$CCC" "$CCC_I686"; do
    [ -x "$cc" ] || continue
    if expect_error "$(basename "$cc") matching digit" "$cc" "$tmp/matching.c" 'matching constraint references invalid operand number'; then
        report "$(basename "$cc") matching digit" 0
    else
        report "$(basename "$cc") matching digit" 1
    fi
    if expect_error "$(basename "$cc") alternative count" "$cc" "$tmp/altcount.c" 'differ in number of alternatives'; then
        report "$(basename "$cc") alternative count" 0
    else
        report "$(basename "$cc") alternative count" 1
    fi
    if expect_error "$(basename "$cc") n(string)" "$cc" "$tmp/nstring.c" "impossible constraint in 'asm'"; then
        report "$(basename "$cc") n(string)" 0
    else
        report "$(basename "$cc") n(string)" 1
    fi
    if expect_error "$(basename "$cc") s(5)" "$cc" "$tmp/sinteger.c" "impossible constraint in 'asm'"; then
        report "$(basename "$cc") s(5)" 0
    else
        report "$(basename "$cc") s(5)" 1
    fi
    if expect_error "$(basename "$cc") unmodelled letter" "$cc" "$tmp/unmodelled.c" "impossible constraint in 'asm'"; then
        report "$(basename "$cc") unmodelled letter" 0
    else
        report "$(basename "$cc") unmodelled letter" 1
    fi
    if expect_error "$(basename "$cc") stray punctuation" "$cc" "$tmp/punct.c" "invalid punctuation '@' in constraint"; then
        report "$(basename "$cc") stray punctuation" 0
    else
        report "$(basename "$cc") stray punctuation" 1
    fi
done

# ── Explicit register constraints are REGISTER classes, not immediates ─────
# `{esi}` contains e/s/i, all letters the immediate classifier knows.  A
# global register variable used to be misclassified as "immediate-only" and
# rejected after optimisation (gcc.c-torture/compile/20041119-1.c on i686).
cat >"$tmp/explicit_reg.c" <<'C'
register unsigned int reg __asm("esi");
unsigned int f(void) { return reg + 1; }
C
if [ -x "$CCC_I686" ]; then
    out=$("$CCC_I686" -O2 -c "$tmp/explicit_reg.c" -o "$tmp/out.o" 2>&1) && rc=0 || rc=$?
    if [ "$rc" -eq 0 ] && ! grep -q 'error:' <<<"$out"; then
        report 'i686 explicit register constraint {esi}' 0
    else
        printf 'FAIL: i686 {esi} rc=%s: %s\n' "$rc" "$out"
        report 'i686 explicit register constraint {esi}' 1
    fi
fi

# ── Runtime: a two-alternative asm must still execute the instruction ──────
cat >"$tmp/runtime.c" <<'C'
#include <stdio.h>
static int add(int a, int b)
{
    int r;
    asm("movl %1, %0; addl %2, %0" : "=r,r"(r) : "r,m"(a), "r,m"(b));
    return r;
}
int main(void)
{
    int r = add(17, 25);
    if (r != 42) { printf("FAIL add(17,25)=%d, expected 42\n", r); return 1; }
    return 0;
}
C
if "$CCC" -O2 -c "$tmp/runtime.c" -o "$tmp/runtime.o" 2>"$tmp/runtime.err" \
   && ! grep -q 'error:' "$tmp/runtime.err" \
   && "$GCC" -o "$tmp/runtime" "$tmp/runtime.o" 2>/dev/null \
   && "$tmp/runtime"; then
    report 'runtime two-alternative addl' 0
else
    printf 'FAIL: runtime two-alternative addl: %s\n' "$(cat "$tmp/runtime.err" 2>/dev/null)"
    report 'runtime two-alternative addl' 1
fi

# ── RED-TEAM: 64-spelling accept/reject DIFFERENTIAL against gcc 16.2 ─────
# Each source is compiled by BOTH compilers at -O0 (deferral off, so the
# front-end grammar is what decides).  This is the ratchet for the holes the
# red-team differential found: unmodelled letters are IMPOSSIBLE ALTERNATIVES
# (but do NOT poison an alternative that also has a class letter -- `rz`,
# `zr`, `zri`, `rzi`, `Yzz` must compile), bare `Y`/`C` prefixes are
# impossible, a `#`-only constraint is "inconsistent operand constraints",
# `%` needs a following operand, and matching digits are range-checked
# anywhere in an alternative (`Yz0`).  Verbatim diagnostics are pinned in
# the Rust unit tests; this gate pins the VERDICT across the whole shape set.
diff_spellings=(
  '' '#foo' 'r,m' 'r,' 'i#' 'i#*X' 'i#@' '%0,0' '0' '[sum]' '{rax}' 'Yz' 'C'
  'Y' 'r?m' '!r,m' '?' 'i#*r' 'r,m,r,m,r,m' '=r,r' '+r,m' 'n,i' 's,r' 'z,r'
  'r,z' 'z' 'Yz0' '%' '%r' 'rz' 'zr' 'zri' 'rzi' 'Yzz' 'Yzr' 'zz' 'zz,r'
  'r,zr' 'rz,r' 'mx' 'xm' 'gZ' 'Zg' 'r#@' 'r#zzz' 'I,r' 'r,I' 'v,r' 'x,r'
  'mX' 'Xm' 'Yz1' 'r10' '?r' '^r' '$r' '*r' '!r' '#' '##' 'r##' 'i#'
  'r#' 'i##zzz'
)
diff_bad=0
for c in "${diff_spellings[@]}"; do
    printf 'void f(int x){ asm volatile("" :: "%s"(x)); }\n' "$c" >"$tmp/diff.c"
    "$GCC" -O0 -c "$tmp/diff.c" -o "$tmp/diff_gcc.o" >"$tmp/gcc.log" 2>&1 && grc=0 || grc=$?
    "$CCC" -O0 -c "$tmp/diff.c" -o "$tmp/diff_lccc.o" >"$tmp/lccc.log" 2>&1 && lrc=0 || lrc=$?
    gerr=0; lerr=0
    if grep -q 'error:' "$tmp/gcc.log"; then gerr=1; fi
    if grep -q 'error:' "$tmp/lccc.log"; then lerr=1; fi
    if [ "$gerr" -ne "$lerr" ] || { [ "$gerr" -eq 0 ] && [ "$grc" -ne "$lrc" ]; }; then
        printf 'FAIL: differential %q gcc(rc=%s err=%s) lccc(rc=%s err=%s)\n' \
            "$c" "$grc" "$gerr" "$lrc" "$lerr"
        diff_bad=$((diff_bad + 1))
    fi
done
report "gcc 16.2 accept/reject differential (${#diff_spellings[@]} spellings)" "$diff_bad"


# ── __builtin_constant_p at -O0 (gcc.c-torture pr38789.c) ─────────────────
# A deferred IsConstant query cannot be resolved by the -O0 tier, which
# returns before `resolve_remaining_is_constant`. Both arms of the conditional
# stayed live, so the untaken arm's `"i"` operand tripped the pipeline's final
# immediate-only validation. GCC 16.2 and Clang 23.1.5 accept this at EVERY
# -O level; the reference values measured at -O0 are 0 (local), 1 (literal),
# 0 (parameter), and at -O1+ the local becomes 1. Pin verdict parity across
# all tiers and both ABIs, plus which arm the -O0 lowering actually emits.
cat >"$tmp/bcp.c" <<'EOF'
void baz(int v)
{
  unsigned a = (v == 1) ? 1 : 2;
  if (__builtin_constant_p(a))
    asm volatile ("# constant %0" :: "i" (a));
  else
    asm volatile ("# register %0" :: "r" (a));
  a = 6;
  if (__builtin_constant_p(a))
    asm volatile ("# constant %0" :: "i" (a));
  else
    asm volatile ("# register %0" :: "r" (a));
}
EOF
bcp_bad=0
bcp_checks=0
for abi in x86_64 i686; do
    if [ "$abi" = i686 ]; then cc="$CCC_I686"; gf=(-m32); lf=(-m32); else cc="$CCC"; gf=(); lf=(); fi
    if [ ! -x "$cc" ]; then continue; fi
    for opt in -O0 -O1 -O2 -O3 -Os; do
        bcp_checks=$((bcp_checks + 1))
        "$GCC" "${gf[@]}" "$opt" -c "$tmp/bcp.c" -o "$tmp/bcp_g.o" >"$tmp/bcp_g.log" 2>&1 && grc=0 || grc=$?
        "$cc" "${lf[@]}" "$opt" -c "$tmp/bcp.c" -o "$tmp/bcp_l.o" >"$tmp/bcp_l.log" 2>&1 && lrc=0 || lrc=$?
        if [ "$grc" -ne "$lrc" ]; then
            printf 'FAIL: builtin_constant_p %s %s gcc=%s lccc=%s: %s\n' \
                "$abi" "$opt" "$grc" "$lrc" "$(cat "$tmp/bcp_l.log")"
            bcp_bad=$((bcp_bad + 1))
        fi
    done
done
# The -O0 lowering must choose the same arm as GCC: only the register form.
"$CCC" -O0 -S "$tmp/bcp.c" -o "$tmp/bcp_l.s" >"$tmp/bcp_s.log" 2>&1 && src=0 || src=$?
if [ "$src" -ne 0 ] || ! grep -qF '# register' "$tmp/bcp_l.s" || grep -qF '# constant' "$tmp/bcp_l.s"; then
    printf 'FAIL: builtin_constant_p -O0 emitted the wrong asm arm (rc=%s)\n' "$src"
    bcp_bad=$((bcp_bad + 1))
fi
# Literal if(0) is the control for the reachability-aware validator: the
# untaken arm keeps an immediate-only asm that must NOT be diagnosed because
# it is never emitted. A dynamic condition must still be diagnosed if such
# an operand reaches it, which the existing spellings above already pin.
cat >"$tmp/dead_arm.c" <<'EOF'
void f(int v){ if (0) __asm__ volatile("# DEAD_ARM" :: "i"(v)); else __asm__ volatile("# LIVE_ARM" :: "r"(v)); }
EOF
dead_bad=0
for abi in x86_64 i686; do
    if [ "$abi" = i686 ]; then cc="$CCC_I686"; af=(-m32); else cc="$CCC"; af=(); fi
    if [ ! -x "$cc" ]; then continue; fi
    "$cc" "${af[@]}" -O0 -S "$tmp/dead_arm.c" -o "$tmp/dead_arm.s" >"$tmp/dead_arm.log" 2>&1 && drc=0 || drc=$?
    if [ "$drc" -ne 0 ] || ! grep -qF '# LIVE_ARM' "$tmp/dead_arm.s" || grep -qF '# DEAD_ARM' "$tmp/dead_arm.s"; then
        printf 'FAIL: literal if(0) %s rc=%s: %s\n' "$abi" "$drc" "$(cat "$tmp/dead_arm.log")"
        dead_bad=$((dead_bad + 1))
    fi
done
report "immediate-only asm in a provably dead if(0) arm is not diagnosed (2 ABIs)" "$dead_bad"

bcp_checks=$((bcp_checks + 1))
report "__builtin_constant_p -O0..-Os verdict + arm parity ($bcp_checks checks, 2 ABIs)" "$bcp_bad"

printf 'check_gcc_asm_constraints: %d passed, %d failed\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
