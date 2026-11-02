#!/usr/bin/env bash
# Preprocessor behaviour that the frontend differential audit pins, asserted
# without needing an oracle compiler so the check runs in every lane.
#
# Each case below was a measured divergence from gcc-16.2 and clang-23.1.4:
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

[ "$status" -eq 0 ] && echo "frontend-preprocessor: ok"
exit "$status"
