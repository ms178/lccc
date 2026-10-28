#!/usr/bin/env bash
# Lexer extensions that GCC accepts and this compiler must accept too, plus the
# diagnostics GCC raises for the shapes it rejects.
#
# Covered here (each measured against gcc 16.2 on x86-64):
#   * C95 digraphs in expression position AND as the directive introducer
#     (`%:include <stdio.h>`), plus `%:%:` and `:>`;
#   * `$` in identifiers (`-fdollars-in-identifiers`, on by default) end to
#     end — the front end accepts it, so the *assembler* and the linker must
#     round-trip a symbol whose name starts with `$`;
#   * universal character names in identifiers: `\u00e9`/`\u20ac` are
#     identifiers, `\u0041`/`\u0024`/`\u00a0` are errors with GCC's wording.
set -uo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
TMP=${TMPDIR:-/tmp}/lccc-lexext.$$
trap 'rm -rf "$TMP"' EXIT
mkdir -p "$TMP"
fail=0

run_ok() { # run_ok <name> <source> <expected-stdout>
    local name=$1 src=$2 want=$3
    printf '%s\n' "$src" >"$TMP/t.c"
    if ! "$CCC" "$TMP/t.c" -o "$TMP/t" 2>"$TMP/err"; then
        echo "FAIL $name: compile failed:" >&2; cat "$TMP/err" >&2; fail=1; return
    fi
    local got
    got=$("$TMP/t")
    [[ $got == "$want" ]] || { echo "FAIL $name: want '$want', got '$got'" >&2; fail=1; }
}

run_err() { # run_err <name> <source> <expected-message-fragment>
    local name=$1 src=$2 want=$3 out
    printf '%s\n' "$src" >"$TMP/t.c"
    out=$("$CCC" -fsyntax-only "$TMP/t.c" 2>&1)
    [[ $out == *"$want"* ]] && return
    echo "FAIL $name: expected '$want', got: $out" >&2
    fail=1
}

# --- digraphs, expression position and as the directive introducer ----------
run_ok digraphs '%:include <stdio.h>
int main(void)<%
  int a<:2:> = <% 1, 2 %>;
  int b = a<:0:> + a<:1:>;
  printf("%d\n", b);
  return 0;
%>' 3

run_ok digraph_hashhash '%:include <stdio.h>
%:define CAT(a, b) a %:%: b
int main(void)<% int xy = 42; printf("%d\n", CAT(x, y)); return 0; %>' 42

# `:>` is a digraph even where the ternary colon is legal, exactly as in GCC.
run_ok digraph_ternary 'int main(void){ int a<:2:> = <%7, 1%>;
  int r = a<:0:> > a<:1:> ? a<:0:> : a<:1:>; return r == 7 ? 0 : 1; }' ''

# --- `$` identifiers: front end, assembler and linker ----------------------
run_ok dollar_symbol 'int $x = 41;
int $fn(void) { return 1; }
int main(void) { return $x + $fn() - 42; }' ''
# Two translation units must agree on a `$` symbol, which also exercises the
# linker's symbol-table path.
printf '%s\n' 'int $shared = 7;' >"$TMP/a.c"
printf '%s\n' 'extern int $shared;
int main(void) { return $shared - 7; }' >"$TMP/b.c"
if ! "$CCC" -c "$TMP/a.c" -o "$TMP/a.o" || ! "$CCC" "$TMP/b.c" "$TMP/a.o" -o "$TMP/dollar"; then
    echo "FAIL dollar_cross_tu: link failed" >&2; fail=1
else
    "$TMP/dollar" || { echo "FAIL dollar_cross_tu: bad result" >&2; fail=1; }
fi

# --- universal character names -------------------------------------------
run_ok ucn_latin1 'int caf\u00e9 = 5;
int main(void) { return caf\u00e9 - 5; }' ''
run_ok ucn_and_utf8_agree 'int caf\u00e9 = 5;
int main(void) { return café - 5; }' ''
run_ok ucn_non_alphabetic 'int \u20ac = 3;
int main(void) { return \u20ac - 3; }' ''
run_err ucn_basic_uppercase 'int \u0041bc = 1;' \
    'universal character \u0041 is not valid in an identifier'
run_err ucn_basic_dollar 'int \u0024x = 1;' \
    'universal character \u0024 is not valid in an identifier'
run_err ucn_no_break_space 'int a\u00a0 = 1;' \
    'universal character \u00a0 is not valid in an identifier'

# A stray backslash is GCC's "stray '\' in program".
run_err stray_backslash 'int a = 1 \ ;' "stray '\' in program"

if ((fail)); then
    echo "check_lexer_extensions: FAILED" >&2
    exit 1
fi
echo "check_lexer_extensions: PASSED"
