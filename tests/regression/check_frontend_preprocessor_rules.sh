#!/usr/bin/env bash
# Preprocessor behaviour that the frontend differential audit pins, asserted
# without needing an oracle compiler so the check runs in every lane.
#
# Each case below was a measured divergence from gcc-16.2 and clang-23.1.5:
#
#  1. C11 6.10.3.4p1 -- the rescan of a replacement list continues with the rest
#     of the source, so an invocation begun inside a replacement list may take
#     its arguments from the tokens that follow. `#define h g(~` followed by
#     `h 5);` is the shape C11 6.10.3.5 EXAMPLE 3 uses, and it produced
#     `f(2 * (~)) 5);` instead of `f(2 * (~ 5));`.
#  2. A bare `(` replacement list is a *token* paren, not an invocation: it must
#     not swallow the following tokens.
#  3. `#if (A, B)` evaluates to `B` in both oracles; the comma production was
#     missing, so lccc tested `A`.
#  4. `-E` output must be a valid translation unit: the synthetic pragma tokens
#     the lexer consumes are rendered back as `#pragma` directives.
#  5. A character constant in `#if` has the value it has in C, and its type's
#     signedness decides the comparison: `L'\400'` is 256, not the identifier
#     `L` (0) followed by the truncated narrow `'\400'` (0) -- the shape
#     gcc.c-torture/execute/widechar-1.c pins -- and `'\377'`, `u'a'`,
#     `L'\xffffffff'` and `'ab'` follow the target's `char` and `wchar_t`.
#     The expected rows are gcc-16.2's `-E` answers (x86-64 and `-m32`, with
#     and without `-funsigned-char` and `-fshort-wchar`); the AArch64 and
#     RISC-V rows apply the same rules to those ABIs.
#  6. `wchar_t` is the psABI's: `int` on x86 and RISC-V, `unsigned int` on
#     AArch64 (GCC and Clang predefine `__WCHAR_TYPE__ unsigned int` there),
#     `unsigned short` under `-fshort-wchar`; `L'x'` has that type, so
#     `L'\xffffffff'` is the int -1 on x86-64 (it used to be the long
#     4294967295), and `L"..."` decays to a pointer to it.
#  7. The byte scanners that rewrite a line copy non-ASCII text whole: a code
#     line holding both `__has_builtin(...)` and the string "é" kept the
#     string as "Ã©" (strlen 4 instead of 2), because each UTF-8 byte was
#     re-encoded as its own Latin-1 character; `#if defined(X) && 'é' == ...`
#     had the same defect in the `defined` resolver.
#  8. The `_Pragma` rescans copy non-ASCII text whole too: a line that formed
#     `_Pragma` through a macro AND held the identifier `café` came out as
#     `cafÃ©` ("stray '\302' in program"), and `_Pragma("message(\"é\")")`
#     de-stringized its operand the same way.
#  9. The message pragmas, as gcc-16.2 implements them (measured case by
#     case): `#pragma GCC error "m"` is an error that fails the translation
#     unit in every mode, `-E` included (it used to be ignored: the unit
#     compiled); `#pragma GCC warning "m"` is a warning no `-W` option
#     controls (`-Wno-cpp` keeps it); a non-literal operand is an error for
#     both; `#pragma message` is macro-expanded, joins adjacent literals,
#     prints the note `'#pragma message: m'` when parsing (never under `-E`),
#     and a malformed one is a `-Wpragmas` warning.  A `.S` file reports
#     `#warning` and `#pragma GCC warning` too (the builtin assembler path
#     dropped every preprocessor warning) and no `#pragma message` note.
#
# Sections 5 and 6 also run the sibling compilers next to $CCC (lccc-i686,
# lccc-arm, lccc-riscv; override with CCC_I686/CCC_ARM/CCC_RISCV) when present.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
tmp=${TMPDIR:-/tmp}/lccc-frontend-pp.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"
status=0

# Compare preprocessed text with every space and newline removed. The exact
# separator an expander chooses is not part of the contract (the audit script
# `scripts/frontend_differential_audit.py` compares real token streams against
# gcc-16 and clang-23); this script is the oracle-free smoke check, so it asserts
# the tokens and their order only.
compact() { tr -d ' \n\t'; }

fail() {
    echo "FAIL: $1" >&2
    shift
    for line in "$@"; do echo "    $line" >&2; done
    status=1
}

# --- 1. invocation completed from the source -------------------------------
cat >"$tmp/continuation.c" <<'C'
#define x 2
#define f(a) f(x * (a))
#define g f
#define h g(~
h 5);
C
"$CCC" -E "$tmp/continuation.c" | grep -v '^#' | compact >"$tmp/continuation.out"
if ! grep -qF 'f(2*(~5));' "$tmp/continuation.out"; then
    fail "replacement list did not complete its invocation from the source" \
         "got: $(cat "$tmp/continuation.out")" \
         "want: f(2*(~5));"
fi

# --- 1b. the standard's own example ---------------------------------------
cat >"$tmp/std3.c" <<'C'
#define x 3
#define f(a) f(x * (a))
#undef x
#define x 2
#define g f
#define z z[0]
#define h g(~
#define m(a) a(w)
#define w 0,1
#define t(a) a
#define p() int
#define q(x) x
#define r(x,y) x ## y
#define str(x) # x
f(y+1) + f(f(z)) % t(t(g)(0) + t)(1);
g(x+(3,4)-w) | h 5) & m
(f)^m(m);
p() i[q()] = { q(1), r(2,3), r(4,), r(,5), r(,) };
char c[2][6] = { str(hello), str() };
C
"$CCC" -E "$tmp/std3.c" | grep -v '^#' | compact >"$tmp/std3.out"
for want in \
        'f(2*(y+1))+f(2*(f(2*(z[0]))))%f(2*(0))+t(1);' \
        'f(2*(2+(3,4)-0,1))|f(2*(~5))&f(2*(0,1))^m(0,1);' \
        'inti[]={1,23,4,5,};' \
        'charc[2][6]={"hello",""};'
do
    if ! grep -qF "$want" "$tmp/std3.out"; then
        fail "C11 6.10.3.5 EXAMPLE 3 line missing: $want" "got: $(cat "$tmp/std3.out")"
    fi
done

# --- 2. a bare paren absorbs nothing --------------------------------------
cat >"$tmp/bare_paren.c" <<'C'
#define LP (
#define twice(a) a + a
LP twice(3))
C
"$CCC" -E "$tmp/bare_paren.c" | grep -v '^#' | compact >"$tmp/bare_paren.out"
if ! grep -qxF '(3+3)' <<<"$(cat "$tmp/bare_paren.out")"; then
    fail "a bare '(' replacement list swallowed following tokens" \
         "got: $(cat "$tmp/bare_paren.out")" "want: (3+3)"
fi

# --- 3. the comma operator in #if -----------------------------------------
cat >"$tmp/comma.c" <<'C'
#if (3, 0)
comma_true
#else
comma_false
#endif
#if (0, 1)
comma_true2
#else
comma_false2
#endif
C
"$CCC" -E "$tmp/comma.c" | grep -v '^#' | compact >"$tmp/comma.out"
if ! grep -qxF 'comma_falsecomma_true2' <<<"$(cat "$tmp/comma.out")"; then
    fail "#if comma operator does not yield its right operand" \
         "got: $(cat "$tmp/comma.out")" "want: comma_falsecomma_true2"
fi

# --- 4. -E renders pragmas as directives ----------------------------------
cat >"$tmp/pack.c" <<'C'
#pragma pack(push, 1)
struct S { char c; int i; };
#pragma pack(pop)
int z = sizeof(struct S);
C
"$CCC" -E "$tmp/pack.c" >"$tmp/pack.out"
for want in '#pragma pack(push, 1)' '#pragma pack(pop)'; do
    if ! grep -qxF "$want" "$tmp/pack.out"; then
        fail "-E output lost the pragma directive: $want" "got: $(grep -n 'pragma\|__ccc_' "$tmp/pack.out" || true)"
    fi
done
if grep -q '__ccc_' "$tmp/pack.out"; then
    fail "-E output leaks internal lexer tokens" "$(grep -n '__ccc_' "$tmp/pack.out")"
fi
# The pragma must still *mean* something: packed is 5 bytes, unpackaged 8.
cat >"$tmp/pack_size.c" <<'C'
#include <stdio.h>
#pragma pack(push, 1)
struct S { char c; int i; };
#pragma pack(pop)
struct T { char c; int i; };
int main(void) { printf("%zu %zu\n", sizeof(struct S), sizeof(struct T)); return 0; }
C
"$CCC" "$tmp/pack_size.c" -o "$tmp/pack_size"
out=$("$tmp/pack_size")
[ "$out" = "5 8" ] || fail "#pragma pack changed meaning: got '$out', want '5 8'"

# Invalid numeric pack operands are ignored, not fed to the power-of-two
# layout machinery. An invalid push must not create a stack entry either.
cat >"$tmp/pack_invalid.c" <<'C'
#pragma pack(2)
#pragma pack(3)
struct A { char c; int i; };
_Static_assert(sizeof(struct A) == 6, "invalid set changed packing");
#pragma pack(32)
struct B { char c; int i; };
_Static_assert(sizeof(struct B) == 6, "out-of-range set changed packing");
#pragma pack(push, 4)
#pragma pack(push, 3)
#pragma pack(pop)
struct C { char c; int i; };
_Static_assert(sizeof(struct C) == 6, "invalid push changed the stack");
#pragma pack(0)
struct D { char c; int i; };
_Static_assert(sizeof(struct D) == 8, "zero did not reset packing");
C
"$CCC" -fsyntax-only "$tmp/pack_invalid.c" >"$tmp/pack_invalid.log" 2>&1 || fail "invalid pack operands failed without -Werror" "$(cat "$tmp/pack_invalid.log")"
[ "$(grep -c 'alignment must be a small power of two' "$tmp/pack_invalid.log" || true)" -eq 3 ] || fail "invalid pack operands did not each warn" "$(cat "$tmp/pack_invalid.log")"
if "$CCC" -fsyntax-only -Werror=pragmas "$tmp/pack_invalid.c" >"$tmp/pack_error.log" 2>&1; then
    fail "-Werror=pragmas did not reject invalid pack operands"
fi
"$CCC" -fsyntax-only -Werror -Wno-pragmas "$tmp/pack_invalid.c" >"$tmp/pack_suppressed.log" 2>&1 || fail "-Wno-pragmas did not suppress pack warnings" "$(cat "$tmp/pack_suppressed.log")"
"$CCC" -E "$tmp/pack_invalid.c" >"$tmp/pack_invalid.out" 2>"$tmp/pack_invalid_e.log" || fail "preprocessing invalid pack operands failed"
[ ! -s "$tmp/pack_invalid_e.log" ] || fail "-E unexpectedly diagnosed frontend pack warnings" "$(cat "$tmp/pack_invalid_e.log")"
for want in '#pragma pack(3)' '#pragma pack(32)' '#pragma pack(push, 3)'; do
    grep -qxF "$want" "$tmp/pack_invalid.out" || fail "-E lost invalid pack directive: $want"
done

# --- 5. character constants in #if ----------------------------------------
bindir=$(dirname "$CCC")
CCC_I686=${CCC_I686:-$bindir/lccc-i686}
CCC_ARM=${CCC_ARM:-$bindir/lccc-arm}
CCC_RISCV=${CCC_RISCV:-$bindir/lccc-riscv}

# One `#if` per row; the answers form a bit string, row 1 first.
rows=(
    "L'\400'"
    "L'\400' == 256"
    "L'\xffffffff' < 0"
    "L'\xffffffff' > -1"
    "u'\xffff' == 65535"
    "u'a' > -1"
    "U'\xffffffff' > 0"
    "U'a' > -1"
    "'\377' < 0"
    "'a' > -1"
    "'\377' > -1"
    "'ab' == 24930"
    "'\377\377\377\377' == -1"
    "'ab' > -1"
    "L'ab' == 'b'"
    "L'a' > -1"
    "'\x41' == 65 && '\101' == 65 && '\n' == 10 && '\\\\' == 92 && '\\'' == 39"
    "L'\x10FFFF' == 0x10FFFF && U'\U0010FFFF' == 0x10FFFF && u'\u00e9' == 0xe9"
    "'\x80' == -128"
    "L'\u00e9' == 233"
    "'abcd' == 0x61626364"
    "'\0' == 0 && !'\0'"
    "(L'\xffffffff' >> 31) == -1"
    "L'\x80000000' == -2147483647 - 1"
    "u8'a' == 97"
    "u8'a' > -1"
    "L'\U0001F600' == 0xDE00"
    "'é' == 0xC3A9 && L'é' == 0xE9"
)
: >"$tmp/charconst.c"
for i in "${!rows[@]}"; do
    printf '#if %s\nrow%d=1\n#else\nrow%d=0\n#endif\n' "${rows[$i]}" "$i" "$i" >>"$tmp/charconst.c"
done
#                  1   5    10   15   20   25
want_x86=1110101011011111111111111001       # x86-64, i686
want_uchar=1110101000011111110111111001     # RISC-V, -funsigned-char
want_short=1100101011011110101111001011     # -fshort-wchar
want_aarch64=1100101000011110110111001001   # AArch64
charconst_bits() { # $@ = compiler and flags
    "$@" -E -P -w "$tmp/charconst.c" | sed -n 's/^row[0-9]*=\([01]\)$/\1/p' | tr -d '\n'
}
check_charconst() { # $1 = label, $2 = expected bits, rest = compiler and flags
    local label=$1 want=$2 got
    shift 2
    got=$(charconst_bits "$@")
    if [ "$got" != "$want" ]; then
        fail "#if character constants under $label" "got:  $got" "want: $want" \
             "(row N is bit N of ${#rows[@]}; see the rows array)"
    fi
}
check_charconst "x86-64" "$want_x86" "$CCC"
check_charconst "x86-64 -funsigned-char" "$want_uchar" "$CCC" -funsigned-char
check_charconst "x86-64 -fshort-wchar" "$want_short" "$CCC" -fshort-wchar
for pair in "i686:$CCC_I686:$want_x86" "aarch64:$CCC_ARM:$want_aarch64" \
            "riscv64:$CCC_RISCV:$want_uchar"; do
    IFS=: read -r label bin want <<<"$pair"
    if [ -x "$bin" ]; then
        check_charconst "$label" "$want" "$bin"
    else
        echo "note: $bin not built; skipping the $label rows" >&2
    fi
done

# An encoding prefix is part of its literal, never a macro, while the same
# spellings alone are still ordinary identifiers.
cat >"$tmp/prefix.c" <<'C'
#define L 0
#define u 7
#define U 9
#define u8 3
#if L'\400' == 256 && u'a' == 97 && U'b' == 98 && u8'c' == 99
prefixes_are_literals
#endif
#if L + u + U + u8 == 19
identifiers_still_expand
#endif
#define WIDE(x) L ## x
#if WIDE('\400') == 256
pasted_prefix
#endif
C
got=$("$CCC" -E -P -w "$tmp/prefix.c" | compact)
[ "$got" = "prefixes_are_literalsidentifiers_still_expandpasted_prefix" ] ||
    fail "encoding prefixes in #if" "got: $got" \
         "want: prefixes_are_literalsidentifiers_still_expandpasted_prefix"

# torture/execute/widechar-1.c, which aborted at every optimization level.
cat >"$tmp/widechar.c" <<'C'
void abort(void);
#define C L'\400'
#if C
#define zero (!C)
#else
#define zero C
#endif
int main(void) { if (zero != 0) abort(); return 0; }
C
"$CCC" -O2 "$tmp/widechar.c" -o "$tmp/widechar"
"$tmp/widechar" || fail "widechar-1: #if L'\\400' was not taken"

# --- 6. wchar_t follows the psABI ------------------------------------------
cat >"$tmp/wchar.c" <<'C'
typedef __WCHAR_TYPE__ wchar_t;
#ifdef WCHAR_UNSIGNED
_Static_assert(L'\xffffffff' == 0xffffffffu && L'\xffffffff' > 0, "unsigned value");
_Static_assert(_Generic(L'a', unsigned int: 1, default: 0), "L'a' is unsigned int");
_Static_assert(_Generic(&L"x"[0], unsigned int *: 1, default: 0), "L\"x\" is unsigned int[]");
_Static_assert((wchar_t)-1 > 0, "wchar_t is unsigned");
_Static_assert(__WCHAR_MAX__ == 0xffffffffu && __WCHAR_MIN__ == 0, "limits");
#else
_Static_assert(L'\xffffffff' == -1 && L'\x80000000' == -2147483647 - 1, "signed value");
_Static_assert(_Generic(L'a', int: 1, default: 0), "L'a' is int");
_Static_assert(_Generic(&L"x"[0], int *: 1, default: 0), "L\"x\" is int[]");
_Static_assert((wchar_t)-1 < 0, "wchar_t is signed");
_Static_assert(__WCHAR_MAX__ == 2147483647 && __WCHAR_MIN__ == -2147483647 - 1, "limits");
#endif
_Static_assert(sizeof(L'a') == sizeof(wchar_t) && sizeof(L'a') == 4, "L'a' is a wchar_t");
_Static_assert(sizeof(L"ab") == 12 && L'\u00e9' == 0xe9 && L'\400' == 256, "32-bit units");
const wchar_t *p = L"wide";
wchar_t arr[] = L"abc";
_Static_assert(sizeof(arr) == 16, "array of wchar_t");
C
cat >"$tmp/wchar16.c" <<'C'
typedef __WCHAR_TYPE__ wchar_t;
_Static_assert(sizeof(L'a') == 2 && sizeof(wchar_t) == 2 && sizeof(L"ab") == 6, "16-bit");
_Static_assert(_Generic(L'a', unsigned short: 1, default: 0), "L'a' is unsigned short");
_Static_assert(L'\xffff' == 65535 && L'\U0001F600' == 0xDE00, "UTF-16 units");
_Static_assert(__WCHAR_MAX__ == 0xffff && __WCHAR_MIN__ == 0, "limits");
C
for pair in "x86-64:$CCC:" "i686:$CCC_I686:" "aarch64:$CCC_ARM:-DWCHAR_UNSIGNED" \
            "riscv64:$CCC_RISCV:"; do
    IFS=: read -r label bin define <<<"$pair"
    [ -x "$bin" ] || continue
    # shellcheck disable=SC2086 # $define is empty or one word
    "$bin" -Wall -Wextra -Werror -fsyntax-only $define "$tmp/wchar.c" 2>"$tmp/wchar.err" ||
        fail "wchar_t is not the $label psABI's" "$(cat "$tmp/wchar.err")"
    "$bin" -w -fshort-wchar -fsyntax-only "$tmp/wchar16.c" 2>"$tmp/wchar16.err" ||
        fail "-fshort-wchar on $label" "$(cat "$tmp/wchar16.err")"
done

# The same constants at run time, against gcc-16.2's output.
cat >"$tmp/wchar_run.c" <<'C'
#include <stdio.h>
int main(void) {
    printf("%d %zu %lld\n", L'\xffffffff' < 0, sizeof(L'\xffffffff'), (long long)L'\xffffffff');
    printf("%d %zu %lld\n", L'\x80000000' < 0, sizeof(L'\x80000000'), (long long)L'\x80000000');
    printf("%d %zu %lld\n", u'\xffff' > -1, sizeof(u'\xffff'), (long long)u'\xffff');
    printf("%d %zu %lld\n", U'\xffffffff' > 0, sizeof(U'\xffffffff'), (long long)U'\xffffffff');
    printf("%d %zu %lld\n", '\377' < 0, sizeof('\377'), (long long)'\377');
    printf("%lld %lld %lld\n", (long long)'ab', (long long)'\377\377\377\377', (long long)'abcde');
    return 0;
}
C
"$CCC" -w "$tmp/wchar_run.c" -o "$tmp/wchar_run"
got=$("$tmp/wchar_run" | tr '\n' '|')
want='1 4 -1|1 4 -2147483648|1 2 65535|1 4 4294967295|1 4 -1|24930 -1 1650680933|'
[ "$got" = "$want" ] || fail "character constants at run time" "got:  $got" "want: $want"

# --- 7. line rewriters keep UTF-8 intact -----------------------------------
cat >"$tmp/utf8_line.c" <<'C'
#include <stdio.h>
#define X 1
#if defined(X) && 'é' == 0xC3A9 && L'é' == 0xE9
static const int pp_utf8 = 1;
#else
static const int pp_utf8 = 0;
#endif
int main(void) {
    const char *s = "é"; int h = __has_builtin(__builtin_expect);
    printf("%zu %d %d\n", __builtin_strlen(s), h, pp_utf8);
    return 0;
}
C
"$CCC" "$tmp/utf8_line.c" -o "$tmp/utf8_line"
got=$("$tmp/utf8_line")
[ "$got" = "2 1 1" ] || fail "UTF-8 text on a rewritten line" "got: $got" "want: 2 1 1"

# --- 8. _Pragma rescans keep UTF-8 intact ---------------------------------
cat >"$tmp/utf8_pragma.c" <<'C'
#define P _Pragma
int café = 41; P("GCC diagnostic push") int get(void) { return café + 1; }
P("GCC diagnostic pop")
int main(void) { return get() == 42 ? 0 : 1; }
C
if out=$("$CCC" -w "$tmp/utf8_pragma.c" -o "$tmp/utf8_pragma" 2>&1); then
    "$tmp/utf8_pragma" || fail "UTF-8 identifier beside a macro-formed _Pragma" "exit $?"
else
    fail "UTF-8 identifier beside a macro-formed _Pragma" "$out"
fi

# --- 9. the message pragmas -------------------------------------------------
# Run `$CCC "$@"` with stderr captured into `diag` and its status in `rc`.
run_diag() {
    rc=0
    diag=$("$CCC" "$@" 2>&1 >/dev/null) || rc=$?
}
# Assert that `diag` holds the fixed string $2 (label $1).
want_diag() {
    if ! grep -qF -- "$2" <<<"$diag"; then
        fail "$1" "missing: $2" "stderr: $diag"
    fi
}
# Assert that `diag` does not hold the fixed string $2 (label $1).
deny_diag() {
    if grep -qF -- "$2" <<<"$diag"; then
        fail "$1" "unexpected: $2" "stderr: $diag"
    fi
}

printf '#pragma GCC error "stop: unsupported configuration"\nint x;\n' >"$tmp/pgerr.c"
for mode in -c -E -fsyntax-only; do
    run_diag "$mode" "$tmp/pgerr.c" -o "$tmp/pgerr.out"
    [ "$rc" -ne 0 ] || fail "#pragma GCC error fails the unit ($mode)" "exit 0"
    want_diag "#pragma GCC error text ($mode)" "error: stop: unsupported configuration"
done

printf '#pragma GCC warning "regenerate \\"me\\""\nint x;\n' >"$tmp/pgwarn.c"
for flags in "" "-Wno-cpp" "-Werror=cpp"; do
    # shellcheck disable=SC2086 # $flags is one optional word
    run_diag -c $flags "$tmp/pgwarn.c" -o "$tmp/pgwarn.o"
    [ "$rc" -eq 0 ] || fail "#pragma GCC warning keeps the unit (${flags:-default})" "exit $rc" "$diag"
    want_diag "#pragma GCC warning text (${flags:-default})" 'warning: regenerate "me"'
done
run_diag -c -Werror "$tmp/pgwarn.c" -o "$tmp/pgwarn.o"
[ "$rc" -ne 0 ] || fail "-Werror promotes #pragma GCC warning" "exit 0"

for operand in '' 'MSG' '("paren")' 'L"wide"'; do
    printf '#define MSG "m"\n#pragma GCC warning %s\nint x;\n' "$operand" >"$tmp/pginv.c"
    run_diag -c "$tmp/pginv.c" -o "$tmp/pginv.o"
    [ "$rc" -ne 0 ] || fail "invalid #pragma GCC warning operand <$operand> is an error" "exit 0"
    want_diag "invalid #pragma GCC warning operand <$operand>" "error: invalid '#pragma GCC warning' directive"
done

cat >"$tmp/pmsg.c" <<'C'
#define WHO "wor" "ld"
#pragma message "hello " WHO
#pragma message ("tab\there\x21")
_Pragma("message(\"\303\251 \303\274\")")
#define P _Pragma
P("message(\"through a macro: é\")")
#pragma message ""
int x;
C
run_diag -c "$tmp/pmsg.c" -o "$tmp/pmsg.o"
[ "$rc" -eq 0 ] || fail "#pragma message keeps the unit" "exit $rc" "$diag"
want_diag "#pragma message expands and joins" "note: '#pragma message: hello world'"
want_diag "#pragma message decodes escapes" "note: '#pragma message: tab$(printf '\t')here!'"
want_diag "_Pragma message keeps UTF-8" "note: '#pragma message: é ü'"
want_diag "macro-formed _Pragma message keeps UTF-8" "note: '#pragma message: through a macro: é'"
# Four notes: the empty message prints none.
notes=$(grep -c "note: '#pragma message:" <<<"$diag" || true)
[ "$notes" -eq 4 ] || fail "#pragma message note count" "got $notes, want 4" "$diag"
run_diag -E "$tmp/pmsg.c" -o "$tmp/pmsg.i"
deny_diag "-E prints no #pragma message note" "#pragma message:"

printf '#pragma message\n#pragma message ("x" 1)\n#pragma message ("y") junk\nint x;\n' >"$tmp/pmbad.c"
run_diag -c "$tmp/pmbad.c" -o "$tmp/pmbad.o"
[ "$rc" -eq 0 ] || fail "malformed #pragma message only warns" "exit $rc" "$diag"
want_diag "#pragma message without a string" "warning: expected a string after '#pragma message' [-Wpragmas]"
want_diag "#pragma message unclosed" "warning: malformed '#pragma message', ignored [-Wpragmas]"
want_diag "#pragma message junk" "warning: junk at end of '#pragma message' [-Wpragmas]"
want_diag "#pragma message junk still prints" "note: '#pragma message: y'"
run_diag -c -Wno-pragmas "$tmp/pmbad.c" -o "$tmp/pmbad.o"
deny_diag "-Wno-pragmas silences malformed #pragma message" "[-Wpragmas]"
run_diag -c -Werror=pragmas "$tmp/pmbad.c" -o "$tmp/pmbad.o"
[ "$rc" -ne 0 ] || fail "-Werror=pragmas promotes a malformed #pragma message" "exit 0"
want_diag "-Werror=pragmas label" "[-Werror=pragmas]"

# A `.S` file reports `#warning` and `#pragma GCC warning` like C (both used
# to be dropped by the builtin assembler path) and prints no
# `#pragma message` note: that pragma belongs to the C front end.
printf '#warning hi-S\n#pragma GCC warning "pw-S"\n#pragma message "pm-S"\n\tnop\n' >"$tmp/pwarn.S"
run_diag -c "$tmp/pwarn.S" -o "$tmp/pwarn.o"
[ "$rc" -eq 0 ] || fail ".S with warnings assembles" "exit $rc" "$diag"
want_diag ".S #warning" "warning: #warning hi-S [-Wcpp]"
want_diag ".S #pragma GCC warning" "warning: pw-S"
deny_diag ".S #pragma message" "pm-S"
run_diag -c -Werror "$tmp/pwarn.S" -o "$tmp/pwarn.o"
[ "$rc" -ne 0 ] || fail "-Werror promotes a .S warning" "exit 0"

# --- 10. __has_feature / __has_extension capability queries -----------------
# GCC 16.2 mode rules, gated on LCCC's implemented capabilities.  Three
# independent gates are pinned here because conflating them is the defect this
# section exists to prevent:
#   * implemented here?  unimplemented rows answer 0 in EVERY mode;
#   * available in this TU?  a sanitizer row gates BOTH operators;
#   * standard in this dialect?  gates the FEATURE answer only -- C99 still
#     accepts _Atomic as an extension, so __has_extension(c_atomic) is 1 there.
# -pedantic-errors collapses the extension answer onto the feature answer;
# -Werror=pedantic does not.  GCC macro-expands the operand (Clang does not);
# LCCC follows its GCC persona.  Expected values were measured against
# gcc-16.2 and clang-23.1.5 and are retained in
# engineering/evidence/2026-11-08-rucc-audit/feature-query-oracle.json.
cat >"$tmp/feature.c" <<'C'
#define FEAT c_generic_selections
#define CHAIN FEAT
#define ARMOURED __c_generic_selections__
int hf_c_alignas = __has_feature(c_alignas);
int he_c_alignas = __has_extension(c_alignas);
int hf_c_alignof = __has_feature(c_alignof);
int he_c_alignof = __has_extension(c_alignof);
int hf_c_atomic = __has_feature(c_atomic);
int he_c_atomic = __has_extension(c_atomic);
int hf_c_generic = __has_feature(c_generic_selections);
int he_c_generic = __has_extension(c_generic_selections);
int hf_c_static_assert = __has_feature(c_static_assert);
int he_c_static_assert = __has_extension(c_static_assert);
int hf_c_thread_local = __has_feature(c_thread_local);
int he_c_thread_local = __has_extension(c_thread_local);
int hf_binlit = __has_feature(cxx_binary_literals);
int he_binlit = __has_extension(cxx_binary_literals);
int hf_tls = __has_feature(tls);
int he_tls = __has_extension(tls);
int hf_asmgoto = __has_feature(gnu_asm_goto_with_outputs);
int he_asmgoto = __has_extension(gnu_asm_goto_with_outputs);
int hf_asmgoto_full = __has_feature(gnu_asm_goto_with_outputs_full);
int he_asmgoto_full = __has_extension(gnu_asm_goto_with_outputs_full);
int hf_ubsan = __has_feature(undefined_behavior_sanitizer);
int he_ubsan = __has_extension(undefined_behavior_sanitizer);
int hf_asan = __has_feature(address_sanitizer);
int he_asan = __has_extension(address_sanitizer);
int hf_hwasan = __has_feature(hwaddress_sanitizer);
int hf_lsan = __has_feature(leak_sanitizer);
int hf_tsan = __has_feature(thread_sanitizer);
int hf_depmsg = __has_feature(attribute_deprecated_with_message);
int he_depmsg = __has_extension(attribute_deprecated_with_message);
int hf_unavailmsg = __has_feature(attribute_unavailable_with_message);
int hf_enumattr = __has_feature(enumerator_attributes);
int he_enumattr = __has_extension(enumerator_attributes);
int hf_unknown = __has_feature(modules);
int he_unknown = __has_extension(modules);
int hf_armoured = __has_feature(__c_generic_selections__);
int he_armoured = __has_extension(__c_generic_selections__);
int hf_half_armour = __has_feature(__c_atomic);
int he_half_armour = __has_extension(c_atomic__);
int hf_macro = __has_feature(FEAT);
int he_macro = __has_extension(FEAT);
int hf_chain = __has_feature(CHAIN);
int hf_armoured_macro = __has_feature(ARMOURED);
#define QUERY(name) __has_feature(name)
int hf_alias = QUERY(c_generic_selections);
#define OP __has_feature
int hf_op_alias = OP(FEAT);
#if __has_feature(c_atomic)
int if_c_atomic = 1;
#else
int if_c_atomic = 0;
#endif
#if __has_extension(c_atomic)
int if_ext_c_atomic = 1;
#else
int if_ext_c_atomic = 0;
#endif
#if __has_feature(c_generic_selections)
int if_generic = 1;
#else
int if_generic = 0;
#endif
#if __has_extension(c_generic_selections)
int if_ext_generic = 1;
#else
int if_ext_generic = 0;
#endif
#if __has_feature(FEAT)
int if_macro = 1;
#else
int if_macro = 0;
#endif
#if __has_feature(cxx_binary_literals)
int if_binlit = 1;
#else
int if_binlit = 0;
#endif
#ifdef __has_feature
int ifdef_op = 1;
#else
int ifdef_op = 0;
#endif
C

# Per-key assertion: order-independent and precise about which row diverged.
# A contiguous-fragment match would silently depend on declaration order.
expect_feature() {
    local label=$1 want=$2
    shift 2
    local got kv key val
    got=$("$CCC" -E "$tmp/feature.c" "$@" 2>/dev/null | grep -v '^#' | compact)
    for kv in $want; do
        key=${kv%%=*}
        val=${kv#*=}
        if ! grep -qE "(^|;)int${key}=${val};" <<<"$got"; then
            fail "$label [$key]" "got: $got" "want: int${key}=${val};"
        fi
    done
}

# Expectation sets are DATA measured from gcc-16.2, not recomputed from the
# implementation's own logic (that would mirror the bug it is meant to catch).
# Pre-C11: the C11/C23 rows are extensions only.
F_PRE11='hf_c_alignas=0 he_c_alignas=1 hf_c_alignof=0 he_c_alignof=1 hf_c_atomic=0 he_c_atomic=0 hf_c_generic=0 he_c_generic=1 hf_c_static_assert=0 he_c_static_assert=1 hf_c_thread_local=0 he_c_thread_local=1 hf_binlit=0 he_binlit=1 hf_tls=1 he_tls=1 hf_asmgoto=0 he_asmgoto=1 hf_asmgoto_full=0 he_asmgoto_full=1 hf_armoured=0 he_armoured=1 hf_macro=0 he_macro=1 hf_chain=0 hf_armoured_macro=0 if_c_atomic=0 if_ext_c_atomic=0 if_generic=0 if_ext_generic=1 if_macro=0 if_binlit=0 hf_half_armour=0 he_half_armour=0 hf_alias=0 hf_op_alias=0 ifdef_op=1'
F_PRE11_PE='hf_c_alignas=0 he_c_alignas=0 hf_c_alignof=0 he_c_alignof=0 hf_c_atomic=0 he_c_atomic=0 hf_c_generic=0 he_c_generic=0 hf_c_static_assert=0 he_c_static_assert=0 hf_c_thread_local=0 he_c_thread_local=0 hf_binlit=0 he_binlit=0 hf_tls=1 he_tls=1 hf_asmgoto=0 he_asmgoto=0 hf_asmgoto_full=0 he_asmgoto_full=0 hf_armoured=0 he_armoured=0 hf_macro=0 he_macro=0 hf_chain=0 hf_armoured_macro=0 if_c_atomic=0 if_ext_c_atomic=0 if_generic=0 if_ext_generic=0 if_macro=0 if_binlit=0 hf_half_armour=0 he_half_armour=0 hf_alias=0 hf_op_alias=0 ifdef_op=1'
# C11..C17: the C11 rows are features; binary literals are still an extension.
F_C11='hf_c_alignas=1 he_c_alignas=1 hf_c_alignof=1 he_c_alignof=1 hf_c_atomic=0 he_c_atomic=0 hf_c_generic=1 he_c_generic=1 hf_c_static_assert=1 he_c_static_assert=1 hf_c_thread_local=1 he_c_thread_local=1 hf_binlit=0 he_binlit=1 hf_tls=1 he_tls=1 hf_asmgoto=0 he_asmgoto=1 hf_asmgoto_full=0 he_asmgoto_full=1 hf_armoured=1 he_armoured=1 hf_macro=1 he_macro=1 hf_chain=1 hf_armoured_macro=1 if_c_atomic=0 if_ext_c_atomic=0 if_generic=1 if_ext_generic=1 if_macro=1 if_binlit=0 hf_half_armour=0 he_half_armour=0 hf_alias=1 hf_op_alias=1 ifdef_op=1'
F_C11_PE='hf_c_alignas=1 he_c_alignas=1 hf_c_alignof=1 he_c_alignof=1 hf_c_atomic=0 he_c_atomic=0 hf_c_generic=1 he_c_generic=1 hf_c_static_assert=1 he_c_static_assert=1 hf_c_thread_local=1 he_c_thread_local=1 hf_binlit=0 he_binlit=0 hf_tls=1 he_tls=1 hf_asmgoto=0 he_asmgoto=0 hf_asmgoto_full=0 he_asmgoto_full=0 hf_armoured=1 he_armoured=1 hf_macro=1 he_macro=1 hf_chain=1 hf_armoured_macro=1 if_c_atomic=0 if_ext_c_atomic=0 if_generic=1 if_ext_generic=1 if_macro=1 if_binlit=0 hf_half_armour=0 he_half_armour=0 hf_alias=1 hf_op_alias=1 ifdef_op=1'
# C23: binary literals become a feature too.
F_C23='hf_c_alignas=1 he_c_alignas=1 hf_c_alignof=1 he_c_alignof=1 hf_c_atomic=0 he_c_atomic=0 hf_c_generic=1 he_c_generic=1 hf_c_static_assert=1 he_c_static_assert=1 hf_c_thread_local=1 he_c_thread_local=1 hf_binlit=1 he_binlit=1 hf_tls=1 he_tls=1 hf_asmgoto=0 he_asmgoto=1 hf_asmgoto_full=0 he_asmgoto_full=1 hf_armoured=1 he_armoured=1 hf_macro=1 he_macro=1 hf_chain=1 hf_armoured_macro=1 if_c_atomic=0 if_ext_c_atomic=0 if_generic=1 if_ext_generic=1 if_macro=1 if_binlit=1 hf_half_armour=0 he_half_armour=0 hf_alias=1 hf_op_alias=1 ifdef_op=1'
F_C23_PE='hf_c_alignas=1 he_c_alignas=1 hf_c_alignof=1 he_c_alignof=1 hf_c_atomic=0 he_c_atomic=0 hf_c_generic=1 he_c_generic=1 hf_c_static_assert=1 he_c_static_assert=1 hf_c_thread_local=1 he_c_thread_local=1 hf_binlit=1 he_binlit=1 hf_tls=1 he_tls=1 hf_asmgoto=0 he_asmgoto=0 hf_asmgoto_full=0 he_asmgoto_full=0 hf_armoured=1 he_armoured=1 hf_macro=1 he_macro=1 hf_chain=1 hf_armoured_macro=1 if_c_atomic=0 if_ext_c_atomic=0 if_generic=1 if_ext_generic=1 if_macro=1 if_binlit=1 hf_half_armour=0 he_half_armour=0 hf_alias=1 hf_op_alias=1 ifdef_op=1'
# Rows that must stay 0 in every mode until LCCC implements the capability.
F_ZEROS='hf_ubsan=0 he_ubsan=0 hf_asan=0 he_asan=0 hf_hwasan=0 hf_lsan=0 hf_tsan=0 hf_depmsg=0 he_depmsg=0 hf_unavailmsg=0 hf_enumattr=0 he_enumattr=0 hf_unknown=0 he_unknown=0'

for std in c89 gnu89 c99 gnu99; do
    expect_feature "feature $std" "$F_PRE11" "-std=$std"
    expect_feature "feature $std -pedantic-errors" "$F_PRE11_PE" "-std=$std" -pedantic-errors
done
for std in c11 gnu11 c17 gnu17; do
    expect_feature "feature $std" "$F_C11" "-std=$std"
    expect_feature "feature $std -pedantic-errors" "$F_C11_PE" "-std=$std" -pedantic-errors
done
for std in c23 gnu23; do
    expect_feature "feature $std" "$F_C23" "-std=$std"
    expect_feature "feature $std -pedantic-errors" "$F_C23_PE" "-std=$std" -pedantic-errors
done
# The default dialect is gnu23, and a user -D__STDC_VERSION__ must not move it.
expect_feature "feature default dialect" "$F_C23"
expect_feature "feature ignores -D__STDC_VERSION__" "$F_C23" -D__STDC_VERSION__=199901L
# -Werror=pedantic upgrades diagnostics; it does not withdraw extensions.
expect_feature "feature -Werror=pedantic keeps extensions" "$F_C11" -std=gnu11 -Werror=pedantic
expect_feature "feature -Werror=pedantic c99 keeps extensions" "$F_PRE11" -std=c99 -Werror=pedantic
# Truthful zeros hold in every dialect class and under -pedantic-errors.
for std in c89 gnu11 gnu23; do
    expect_feature "feature $std truthful zeros" "$F_ZEROS" "-std=$std"
    expect_feature "feature $std truthful zeros -pedantic-errors" "$F_ZEROS" "-std=$std" -pedantic-errors
done
# Sanitizer rows: the undefined-behavior group and its members gate BOTH
# operators; float-divide-by-zero alone is not that row in GCC 16.2.  LCCC
# implements UBSan in TRAP mode only (no libubsan runtime), so the active
# sanitizer is spelled with -fsanitize-trap; without it the driver rejects the
# flag, which is the truthful answer for an unavailable runtime.
expect_feature "feature ubsan group" 'hf_ubsan=1 he_ubsan=1' -fsanitize=undefined -fsanitize-trap=all
expect_feature "feature ubsan member" 'hf_ubsan=1 he_ubsan=1' -fsanitize=signed-integer-overflow -fsanitize-trap=all
expect_feature "feature ubsan bounds-strict" 'hf_ubsan=1 he_ubsan=1' -fsanitize=bounds-strict -fsanitize-trap=all
expect_feature "feature ubsan survives -pedantic-errors" 'hf_ubsan=1 he_ubsan=1' -fsanitize=undefined -fsanitize-trap=all -pedantic-errors
expect_feature "feature ubsan legacy trap flag" 'hf_ubsan=1 he_ubsan=1' -fsanitize=undefined -fsanitize-undefined-trap-on-error
expect_feature "feature float-divide-by-zero is not the ubsan row" 'hf_ubsan=0 he_ubsan=0' -fsanitize=float-divide-by-zero -fsanitize-trap=all
expect_feature "feature no sanitizer means no ubsan row" 'hf_ubsan=0 he_ubsan=0' -fsanitize-trap=all
# An unavailable sanitizer runtime is rejected, never silently advertised.
run_diag -E -fsanitize=undefined "$tmp/feature.c"
[ "$rc" -ne 0 ] || fail "ubsan without trap mode must be rejected" "exit 0"
want_diag "ubsan runtime is unavailable" "requires trap mode"
run_diag -E -fsanitize=address "$tmp/feature.c"
[ "$rc" -ne 0 ] || fail "address sanitizer must be rejected" "exit 0"
want_diag "address sanitizer is unavailable" "is not supported"
# A string literal is not an operator invocation.
if ! "$CCC" -E -std=gnu23 "$tmp/feature.c" 2>/dev/null | grep -qF 'int hf_c_generic = 1;'; then
    fail "feature code-line expansion" "expected 'int hf_c_generic = 1;' in -E output"
fi
cat >"$tmp/feature_literal.c" <<'C'
const char *s = "__has_feature(c_atomic) __has_extension(tls)";
int c = '__has_feature';
C
if ! "$CCC" -E -std=gnu23 "$tmp/feature_literal.c" 2>/dev/null \
    | grep -qF '"__has_feature(c_atomic) __has_extension(tls)"'; then
    fail "feature literals untouched" "string literal was rewritten"
fi

# Malformed operands are constraint violations: GCC errors and answers 0.
# Recovery must consume the whole balanced operand (no stray ')' downstream).
cat >"$tmp/feature_bad.c" <<'C'
int a = __has_feature();
int b = __has_feature(123);
int c = __has_feature((c_atomic));
int d = __has_feature c_atomic;
int e = __has_feature(c_atomic tls);
int g = __has_extension();
int after = 1;
int f = __has_feature(c_atomic;
C
run_diag -E -std=gnu23 "$tmp/feature_bad.c"
[ "$rc" -ne 0 ] || fail "feature malformed operands are errors" "exit 0"
want_diag "feature empty operand" "macro '__has_feature' requires an identifier"
want_diag "feature numeric operand" "macro '__has_feature' requires an identifier"
want_diag "feature nested operand" "macro '__has_feature' requires an identifier"
want_diag "feature missing paren" "missing '(' after '__has_feature'"
want_diag "feature two names" "expected ')' after 'c_atomic'"
want_diag "feature missing close" "expected ')' after 'c_atomic'"
want_diag "extension empty operand" "macro '__has_extension' requires an identifier"
# A macro operand that expands to junk is diagnosed with the FIRST EXPANDED
# token (GCC), and the #if path must print the same text as the code-line path.
cat >"$tmp/feature_junk.c" <<'C'
#define NOTIDENT c_generic_selections tls
int f = __has_feature(NOTIDENT);
C
run_diag -E -std=gnu23 "$tmp/feature_junk.c"
[ "$rc" -ne 0 ] || fail "junk macro operand is an error (code line)" "exit 0"
want_diag "junk macro operand names the expanded token" "expected ')' after 'c_generic_selections'"
cat >"$tmp/feature_junk_if.c" <<'C'
#define NOTIDENT c_generic_selections tls
#if __has_feature(NOTIDENT)
int x = 1;
#else
int x = 0;
#endif
C
run_diag -E -std=gnu23 "$tmp/feature_junk_if.c"
[ "$rc" -ne 0 ] || fail "junk macro operand is an error (#if)" "exit 0"
want_diag "junk macro operand: #if agrees with code line" "expected ')' after 'c_generic_selections'"

# GCC macro-expands __has_builtin / __has_attribute operands too (measured);
# the #if path must agree with the code-line path.
cat >"$tmp/has_macro_arg.c" <<'C'
#define BUILTIN memcpy
#define ATTR deprecated
#define SCOPED gnu::deprecated
int b_code = __has_builtin(BUILTIN);
int b_unprefixed = __has_builtin(memcpy);
int b_prefixed = __has_builtin(__builtin_memcpy);
int b_unknown = __has_builtin(not_a_builtin);
int a_code = __has_attribute(ATTR);
int s_code = __has_attribute(SCOPED);
#if __has_builtin(BUILTIN)
int b_if = 1;
#else
int b_if = 0;
#endif
#if __has_attribute(ATTR)
int a_if = 1;
#else
int a_if = 0;
#endif
#if __has_attribute(SCOPED)
int s_if = 1;
#else
int s_if = 0;
#endif
C
got=$("$CCC" -E -std=gnu23 "$tmp/has_macro_arg.c" 2>/dev/null | grep -v '^#' | compact)
for kv in b_code=1 b_if=1 b_unprefixed=1 b_prefixed=1 b_unknown=0 a_if=1 s_code=1 s_if=1; do
    key=${kv%%=*}; val=${kv#*=}
    grep -qE "(^|;)int${key}=${val};" <<<"$got" \
        || fail "has_* macro operand [$key]" "got: $got" "want: int${key}=${val};"
done

# Data-driven differential against the committed GCC 16.2 oracle matrix:
# every implemented row must match GCC, every unimplemented row must be 0,
# and the #if path must agree with the code-line path in all 18 modes.
here=$(cd "$(dirname "$0")" && pwd)
if ! python3 "$here/check_feature_query_oracle.py" --lccc "$CCC"; then
    status=1
fi
# Capability truthfulness: every row that answers 1 must work end to end.
if ! env CCC="$CCC" bash "$here/check_feature_capabilities.sh"; then
    status=1
fi

[ "$status" -eq 0 ] && echo "frontend-preprocessor: ok"
exit "$status"
