#!/usr/bin/env bash
# A malformed `#define` must be diagnosed at the definition, and must never hang.
#
# Background (backlog AU-01): the parameter-list loop of `parse_define` advanced
# only for whitespace, `)`, `...`, an identifier or `,`.  Any other byte -- `;`, a
# digit, `+` -- neither consumed input nor left the loop, so `#define F(a;b) a`
# made every `lccc -E/-S/-c` run forever.  The same parser accepted `F(a b)` as two
# parameters, ignored duplicates, and had no error channel at all.
#
# Four legs, all against $CCC with `timeout`, so a regression of the hang fails
# in seconds instead of stalling a job:
#
#   1. malformed definitions: exit status 1 (never 124 = hang, never 0), and the
#      diagnostic names the problem with cpplib's wording;
#   2. GCC (when available) rejects every one of them too -- the oracle for "this
#      is an error", independent of the message text, which differs by version;
#   3. valid parameter lists (`()`, `(a,b)`, `(...)`, `(a,...)`, GNU named
#      variadics with and without a space, tabs and vertical tabs between tokens)
#      preprocess to GCC's token stream, so the stricter parser did not start
#      rejecting or re-reading anything GCC accepts;
#   4. the same grammar on the command line: a malformed `-D` is an error against
#      <command-line> (exit 1, defines nothing), a valid `-DP(x)=L_##x` still
#      expands like GCC's.
#
# Usage: env CCC=target/fastbuild/lccc [GCC=gcc] bash tests/regression/check_malformed_define.sh
set -u

CCC=${CCC:-target/fastbuild/lccc}
GCC=${GCC:-gcc}
LIMIT=${LIMIT:-10}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

have_gcc=1
command -v "$GCC" >/dev/null 2>&1 || have_gcc=0
fail=0
checked=0

die() { echo "FAIL $*" >&2; fail=$((fail + 1)); }

# --- 1 + 2: malformed definitions -------------------------------------------------
# name | source (printf format) | fragment of lccc's diagnostic
cases=(
  'semicolon-in-list|#define F(a;b) a\nint x;\n|expected '"'"','"'"' or '"'"')'"'"', found ";"'
  'semicolon-after-comma|#define F(a,;) a\n|expected parameter name, found ";"'
  'missing-comma|#define F(a b) a\nint y;\n|expected '"'"','"'"' or '"'"')'"'"', found "b"'
  'trailing-comma|#define F(a,) a\n|expected parameter name, found ")"'
  'leading-comma|#define F(,a) a\n|expected parameter name, found ","'
  'number-as-parameter|#define F(1) a\n|expected parameter name, found "1"'
  'duplicate-parameter|#define F(a, a) a\n|duplicate macro parameter "a"'
  'unterminated-list|#define F(a\n|expected '"'"')'"'"' before end of line'
  'unterminated-after-comma|#define F(a,\n|expected parameter name before end of line'
  'junk-after-ellipsis|#define F(...x) a\n|expected '"'"')'"'"' after "..."'
  'no-macro-name|#define\nint z;\n|no macro name given in #define directive'
  'numeric-macro-name|#define 123 x\n|macro names must be identifiers'
)
for entry in "${cases[@]}"; do
  IFS='|' read -r name src fragment <<<"$entry"
  printf "$src" >"$tmp/$name.c"
  timeout "$LIMIT" "$CCC" -E "$tmp/$name.c" >"$tmp/out" 2>"$tmp/err"
  rc=$?
  checked=$((checked + 1))
  if [ "$rc" -eq 124 ]; then
    die "$name: hung (killed after ${LIMIT}s)"
  elif [ "$rc" -ne 1 ]; then
    die "$name: exit status $rc, expected 1"
  elif ! grep -qF -- "$fragment" "$tmp/err"; then
    die "$name: diagnostic lacks '$fragment': $(head -c 200 "$tmp/err")"
  fi
  if [ "$have_gcc" -eq 1 ]; then
    if "$GCC" -E "$tmp/$name.c" >/dev/null 2>&1; then
      die "$name: GCC accepts it, so the case is not an error (fix the case)"
    fi
  fi
done

# --- 3: valid lists keep GCC's token stream ----------------------------------------
valid=(
  'empty-list|#define F() 1\nF()\n'
  'two-params|#define G(a,b) a+b\nG(1,2)\n'
  'spaced-params|#define M( a\t, b ) a b\nM(1,2)\n'
  'vtab-and-ff|#define V(a\v,\fb) a b\nV(1,2)\n'
  'variadic-only|#define H(...) __VA_ARGS__\nH(1,2)\n'
  'named-and-variadic|#define I(a,...) a __VA_ARGS__\nI(1,2,3)\n'
  'gnu-named-variadic|#define J(args...) args\nJ(1,2)\n'
  'gnu-named-variadic-space|#define K(args ...) args\nK(1,2)\n'
  'object-like-with-parens|#define L (a;b)\nL\n'
)
norm() { tr -d ' \t\n'; }
for entry in "${valid[@]}"; do
  IFS='|' read -r name src <<<"$entry"
  printf "$src" >"$tmp/$name.c"
  checked=$((checked + 1))
  timeout "$LIMIT" "$CCC" -E -P "$tmp/$name.c" >"$tmp/lccc.out" 2>"$tmp/err"
  rc=$?
  if [ "$rc" -ne 0 ]; then
    die "$name: valid definition rejected (exit $rc): $(head -c 200 "$tmp/err")"
    continue
  fi
  if [ "$have_gcc" -eq 1 ]; then
    "$GCC" -E -P "$tmp/$name.c" >"$tmp/gcc.out" 2>/dev/null
    if [ "$(norm <"$tmp/lccc.out")" != "$(norm <"$tmp/gcc.out")" ]; then
      die "$name: token stream differs from GCC: lccc='$(norm <"$tmp/lccc.out")' gcc='$(norm <"$tmp/gcc.out")'"
    fi
  fi
done

# --- 4: the same grammar on the command line (-D) ---------------------------------
# GCC reports a malformed -D against <command-line>, exits 1 and defines nothing;
# lccc used to hang on the first one and silently define an object-like macro
# named `1X` for the second.
printf 'int main(void){return 0;}\n' >"$tmp/d.c"
dcases=(
  'F(a;b)=1|expected '"'"','"'"' or '"'"')'"'"', found ";"'
  'F(a,a)=1|duplicate macro parameter "a"'
  '1X=1|macro names must be identifiers'
)
for entry in "${dcases[@]}"; do
  IFS='|' read -r def fragment <<<"$entry"
  checked=$((checked + 1))
  timeout "$LIMIT" "$CCC" -E "-D$def" "$tmp/d.c" >"$tmp/out" 2>"$tmp/err"
  rc=$?
  if [ "$rc" -eq 124 ]; then
    die "-D$def: hung (killed after ${LIMIT}s)"
  elif [ "$rc" -ne 1 ]; then
    die "-D$def: exit status $rc, expected 1"
  elif ! grep -qF -- "$fragment" "$tmp/err"; then
    die "-D$def: diagnostic lacks '$fragment': $(head -c 200 "$tmp/err")"
  fi
  if [ "$have_gcc" -eq 1 ] && "$GCC" -E "-D$def" "$tmp/d.c" >/dev/null 2>&1; then
    die "-D$def: GCC accepts it, so the case is not an error (fix the case)"
  fi
done
# A valid function-like -D still expands exactly like GCC's.
printf 'P(1) P(abc)\n' >"$tmp/p.c"
checked=$((checked + 1))
timeout "$LIMIT" "$CCC" -E -P '-DP(x)=L_##x' "$tmp/p.c" >"$tmp/lccc.out" 2>"$tmp/err"
rc=$?
if [ "$rc" -ne 0 ]; then
  die "-DP(x)=L_##x: valid definition rejected (exit $rc): $(head -c 200 "$tmp/err")"
elif [ "$have_gcc" -eq 1 ]; then
  "$GCC" -E -P '-DP(x)=L_##x' "$tmp/p.c" >"$tmp/gcc.out" 2>/dev/null
  [ "$(norm <"$tmp/lccc.out")" = "$(norm <"$tmp/gcc.out")" ] \
    || die "-DP(x)=L_##x: token stream differs from GCC: lccc='$(norm <"$tmp/lccc.out")' gcc='$(norm <"$tmp/gcc.out")'"
fi

if [ "$fail" -ne 0 ]; then
  echo "check_malformed_define: $fail of $checked case(s) failed" >&2
  exit 1
fi
echo "check_malformed_define: all $checked cases ok (gcc oracle: $([ "$have_gcc" -eq 1 ] && echo yes || echo no))"
