#!/usr/bin/env bash
# GNU declaration-corner regression gate (gcc 16.2 oracle, both ABIs).
#
# Six gcc.c-torture/compile cases failed on TWO root causes, not six:
#
#  A. `-fpermissive` implicit-int file-scope declarations: 20101216-1.c,
#     pr72802.c, pr90275.c.  The parser only recovered implicit int for
#     `identifier (` (K&R function definitions); `a, b, c;`, `two52 = ...;`
#     and `*t29;` produced "expected declaration before ...".  GCC 16.2 in
#     gnu23 rejects them by DEFAULT with -Wimplicit-int errors, but accepts
#     them with -fpermissive as warnings.  The diagnostic engine already owns
#     that promotion (WARNING_REGISTRY: ImplicitInt is c99_default_error), so
#     the parser now emits ONE correctly classified warning and yields
#     TypeSpecifier::Int.  `struct S0 { signed f5 };` is a GCC PEDWARN
#     ("no semicolon at end of struct or union"), not a parse error.
#
#  B. Tentative definitions of a record completed LATER in the TU:
#     930525-1.c, pr85401.c, pr96998.c.  C11 6.9.2p2 makes
#     `struct c d; struct c { int e; };` valid; lccc emitted "storage size of
#     'd' isn't known" at the DECLARATION instead of at the END of the TU.
#
# This gate asserts both the ACCEPTANCE and the still-correct REFUSAL, so a
# future change cannot silently loosen C23 or hide a genuinely incomplete
# object.  Every fixture is generated here (self-contained, no corpus needed).
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
CCC_I686=${CCC_I686:-./target/fastbuild/lccc-i686}
GCC=${GCC:-gcc-16}
command -v "$GCC" >/dev/null 2>&1 || GCC=gcc
[ -x "$CCC" ] || { echo "check_gnu_declaration_corners: SKIP ($CCC not built)"; exit 0; }

tmp=${TMPDIR:-/tmp}/lccc-gnu-decl.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"

pass=0
fail=0
check() {
    if [ "$2" -eq 0 ]; then pass=$((pass + 1)); else printf 'FAIL: %s\n' "$1"; fail=$((fail + 1)); fi
}
ok() { # ok <label> <compiler> <flags...> -- <file>
    local label=$1 cc=$2; shift 2
    local out rc
    out=$("$cc" "$@" -c "$tmp/case.c" -o "$tmp/out.o" 2>&1) && rc=0 || rc=$?
    if [ "$rc" -eq 0 ] && ! grep -q 'error:' <<<"$out"; then
        check "$label" 0
    else
        printf 'FAIL: %s rc=%s: %s\n' "$label" "$rc" "$out"
        check "$label" 1
    fi
}
must_fail() { # must_fail <label> <compiler> <needle> <flags...>
    local label=$1 cc=$2 needle=$3; shift 3
    local out rc
    out=$("$cc" "$@" -c "$tmp/case.c" -o "$tmp/out.o" 2>&1) && rc=0 || rc=$?
    if [ "$rc" -ne 0 ] && grep -qF "$needle" <<<"$out"; then
        check "$label" 0
    else
        printf 'FAIL: %s rc=%s expected "%s": %s\n' "$label" "$rc" "$needle" "$out"
        check "$label" 1
    fi
}

# ── A. Implicit int, -fpermissive acceptance / C23 default refusal ─────────
cat >"$tmp/case.c" <<'C'
typedef signed int __int32_t;
typedef union { double value; struct { unsigned msw; unsigned lsw; } parts; } shape;
two52 = 4.50359962737049600000e+15, w6 = -1.63092934096575273989e-03;
a, b, c;
*t29;
static volatile d;
const volatile e;
static fn(void) { return 0; }
struct S0 { short f3; float f4; signed f5 };
const struct S0 t7[] = {};
int main(void) { return 0; }
C
for cc in "$CCC" "$CCC_I686"; do
    [ -x "$cc" ] || continue
    ok "implicit-int -fpermissive $(basename "$cc")" "$cc" -O2 -fpermissive
    must_fail "implicit-int C23 default still errors $(basename "$cc")" "$cc" \
        "type defaults to 'int' in declaration" -O2
done

# ── B. Tentative definition completed later, with runtime proof ────────────
cat >"$tmp/case.c" <<'C'
typedef struct foo foo_t;
foo_t x;
struct foo { int i; };
struct c d;
struct c { int e[1]; };
int main(void) { d.e[0] = 42; return (x.i + d.e[0]) == 42 ? 0 : 1; }
C
for cc in "$CCC" "$CCC_I686"; do
    [ -x "$cc" ] || continue
    ok "late-completed tentative definition $(basename "$cc")" "$cc" -O2
done
if "$CCC" -O2 -c "$tmp/case.c" -o "$tmp/runtime.o" 2>"$tmp/runtime.err" \
   && ! grep -q 'error:' "$tmp/runtime.err" \
   && "$GCC" -o "$tmp/runtime" "$tmp/runtime.o" 2>/dev/null \
   && "$tmp/runtime"; then
    check 'late-completed tentative definition runtime' 0
else
    printf 'FAIL: runtime: %s\n' "$(cat "$tmp/runtime.err" 2>/dev/null)"
    check 'late-completed tentative definition runtime' 1
fi

# A tentative definition that is STILL incomplete at end of TU must error.
cat >"$tmp/case.c" <<'C'
struct c d;
int main(void) { return 0; }
C
must_fail 'still-incomplete tentative definition errors' "$CCC" \
    "storage size of 'd' isn't known" -O2

# Compiler and oracle agree on the refusal.
# The producer's exit status is ASSERTED, not swallowed with `|| true`:
# scripts/check_pipefail_sigpipe.py flags a swallowed producer, and a GCC
# that silently starts accepting this program must turn the gate RED.
gcc_out=$("$GCC" -O2 -c "$tmp/case.c" -o "$tmp/out.o" 2>&1) && gcc_rc=0 || gcc_rc=$?
if [ "$gcc_rc" -ne 0 ] && grep -qF "storage size of 'd' isn't known" <<<"$gcc_out"; then
    check 'gcc 16.2 refuses the same incomplete tentative definition' 0
else
    printf 'FAIL: gcc 16.2 oracle rc=%s: %s\n' "$gcc_rc" "$gcc_out"
    check 'gcc 16.2 refuses the same incomplete tentative definition' 1
fi

# ── Missing semicolon before `}` is a warning, not an error ────────────────
cat >"$tmp/case.c" <<'C'
struct S { int x };
int main(void) { struct S s; s.x = 1; return s.x != 1; }
C
out=$("$CCC" -O2 -c "$tmp/case.c" -o "$tmp/out.o" 2>&1) && rc=0 || rc=$?
if [ "$rc" -eq 0 ] && grep -qF 'no semicolon at end of struct or union' <<<"$out" \
   && ! grep -q 'error:' <<<"$out"; then
    check 'missing struct-field semicolon is a GCC pedwarn' 0
else
    printf 'FAIL: missing semicolon rc=%s: %s\n' "$rc" "$out"
    check 'missing struct-field semicolon is a GCC pedwarn' 1
fi

# ── `__attribute__((mode(M)))` on a POINTER typedef ───────────────────────
# gcc 16.2: `mode(SI)` is a no-op on a 32-bit target (SI = pointer mode) and
# an ERROR "invalid pointer mode 'SI'" on x86-64; `mode(DI)` is the 64-bit
# spelling.  Clang 23.1.4 rejects pointer modes altogether, so GCC is the
# oracle.  gcc.c-torture/compile/20111209-1.c is accepted by GCC on i686.
cat >"$tmp/case.c" <<'C'
typedef char* P __attribute__((mode(SI)));
P getenv_like(const char *s);
char *use(const char *s) { return getenv_like(s); }
C
out=$("$CCC_I686" -O2 -c "$tmp/case.c" -o "$tmp/out.o" 2>&1) && rc=0 || rc=$?
if [ "$rc" -eq 0 ] && ! grep -q 'error:' <<<"$out"; then
    check 'i686 pointer mode(SI) keeps char *' 0
else
    printf 'FAIL: i686 pointer mode(SI) rc=%s: %s\n' "$rc" "$out"
    check 'i686 pointer mode(SI) keeps char *' 1
fi
out=$("$CCC" -O2 -c "$tmp/case.c" -o "$tmp/out.o" 2>&1) && rc=0 || rc=$?
if [ "$rc" -ne 0 ] && grep -qF "invalid pointer mode 'SI'" <<<"$out"; then
    check "x86_64 pointer mode(SI) refused like gcc 16.2" 0
else
    printf 'FAIL: x86_64 pointer mode(SI) rc=%s: %s\n' "$rc" "$out"
    check 'x86_64 pointer mode(SI) refused like gcc 16.2' 1
fi
cat >"$tmp/case.c" <<'C'
typedef char* P __attribute__((mode(DI)));
P getenv_like(const char *s);
char *use(const char *s) { return getenv_like(s); }
C
out=$("$CCC" -O2 -c "$tmp/case.c" -o "$tmp/out.o" 2>&1) && rc=0 || rc=$?
if [ "$rc" -eq 0 ] && ! grep -q 'error:' <<<"$out"; then
    check 'x86_64 pointer mode(DI) accepted like gcc 16.2' 0
else
    printf 'FAIL: x86_64 pointer mode(DI) rc=%s: %s\n' "$rc" "$out"
    check 'x86_64 pointer mode(DI) accepted like gcc 16.2' 1
fi

# ── RED-TEAM ratchet: C11 6.9.2p2 definitions vs tentative definitions ────
# The first version of the deferred-incomplete fix deferred INITIALIZED
# definitions too, silently accepting `struct c x = {0}; struct c {int e;};`
# (gcc 16.2 refuses).  Pin all eight measured shapes, including the exact
# GCC diagnostic wording, so this semantic hole cannot reappear.
redteam_case() { # redteam_case <label> <expected_rc> <needle> <source>
    local label=$1 want=$2 needle=$3 src=$4
    printf '%s\n' "$src" >"$tmp/case.c"
    local out rc
    out=$("$CCC" -O2 -c "$tmp/case.c" -o "$tmp/out.o" 2>&1) && rc=0 || rc=$?
    local ok=1
    [ "$rc" -ne 0 ] && want=1
    if [ "$want" -eq 1 ]; then
        [ "$rc" -ne 0 ] || ok=0
        if [ -n "$needle" ] && ! grep -qF "$needle" <<<"$out"; then ok=0; fi
    else
        [ "$rc" -eq 0 ] || ok=0
    fi
    if [ "$ok" -eq 1 ]; then check "$label" 0
    else printf 'FAIL: %s rc=%s expected_rc=%s needle=%s: %s\n' "$label" "$rc" "$want" "$needle" "$out"; check "$label" 1; fi
}
redteam_case 'initialized incomplete object refuses later completion' 1 \
    "variable 'x' has initializer but incomplete type" \
    'struct c x = {0}; struct c { int e; }; int main(void){return 0;}'
redteam_case 'initialized incomplete object, no later completion' 1 \
    "variable 'x' has initializer but incomplete type" \
    'struct c x = {0}; int main(void){return 0;}'
redteam_case 'block-scope initialized incomplete object' 1 \
    "variable 'x' has initializer but incomplete type" \
    'void f(void){ struct c x = {0}; }'
redteam_case 'extern initialized incomplete object' 1 \
    "variable 'x' has initializer but incomplete type" \
    'extern struct c x = {0};'
redteam_case 'tentative external definition completed later' 0 '' \
    'struct c x; struct c {int e;}; int main(void){return 0;}'
redteam_case 'tentative static definition completed later' 0 '' \
    'static struct c x; struct c {int e;}; int main(void){return 0;}'
redteam_case 'extern incomplete declaration allocates nothing' 0 '' \
    'extern struct c x; int main(void){return 0;}'
redteam_case 'tentative definition still incomplete at TU end' 1 \
    "storage size of 'd' isn't known" \
    'struct c d; int main(void){return 0;}'

# ── RED-TEAM: `mode(M)` accept/reject differential, BOTH ABIs ──────────────
# Ten declarator shapes measured against gcc 16.2 on x86-64 AND -m32. The
# central Parser::apply_mode_attribute rule pins: function/array declarators
# -> "mode 'M' applied to inappropriate type"; pointer declarators -> target
# pointer-mode validation with the pointee LEFT ALONE; scalar -> base type
# transform.  Each shape is compiled by both compilers; a verdict mismatch is
# a gate failure.  (Specifier-position mode on a function/array/pointer
# declarator is a known remaining pre-existing gap, documented in §9.4.)
mode_shapes=(
  'int f(void) __attribute__((mode(SI))){return 0;}'
  'int x __attribute__((mode(SI)));'
  'int x[3] __attribute__((mode(SI)));'
  'int (*fp)(void) __attribute__((mode(DI)));'
  'int f(void) __attribute__((mode(QI)));'
  'char *f(void) __attribute__((mode(SI)));'
  'char *f(void) __attribute__((mode(DI)));'
  'typedef char *P __attribute__((mode(SI))); P f(void){return 0;}'
  'typedef char *P __attribute__((mode(DI))); P p;'
  'typedef char *P __attribute__((mode(SI))), *Q __attribute__((mode(SI)));'
)
mode_bad=0
for abi in x86_64 i686; do
    if [ "$abi" = i686 ]; then cc=$CCC_I686; gf=(-m32); else cc=$CCC; gf=(); fi
    for src in "${mode_shapes[@]}"; do
        printf '%s\n' "$src" >"$tmp/mode.c"
        "$GCC" "${gf[@]}" -O2 -c "$tmp/mode.c" -o "$tmp/mode_gcc.o" \
            >"$tmp/gcc.log" 2>&1 && grc=0 || grc=$?
        "$cc" -O2 -c "$tmp/mode.c" -o "$tmp/mode_lccc.o" \
            >"$tmp/lccc.log" 2>&1 && lrc=0 || lrc=$?
        gerr=0; lerr=0
        if grep -q 'error:' "$tmp/gcc.log"; then gerr=1; fi
        if grep -q 'error:' "$tmp/lccc.log"; then lerr=1; fi
        if [ "$gerr" -ne "$lerr" ] || { [ "$gerr" -eq 0 ] && [ "$grc" -ne "$lrc" ]; }; then
            printf 'FAIL: mode differential %s %q gcc(rc=%s err=%s) lccc(rc=%s err=%s)\n' \
                "$abi" "$src" "$grc" "$gerr" "$lrc" "$lerr"
            mode_bad=$((mode_bad + 1))
        fi
    done
done
check "gcc 16.2 mode(M) differential on both ABIs (2x${#mode_shapes[@]} shapes)" "$mode_bad"


# ---------------------------------------------------------------------------
# C. K&R parenthesised-name declarators -- `(* const (name)(params))`.
#
# gcc.c-torture/compile/991213-2.c uses this shape in a PARAMETER. Two
# distinct defects were found here, and BOTH are pinned below:
#
#  1. PARSING. The nested `(name)` group parses its OWN trailing parameter
#     list. The old code consumed the enclosing `)` optimistically looking
#     for another `(` outside, found none, rewound, and `extract_paren_name`
#     left the parameter list unconsumed: "expected ')' before '('".
#  2. TYPING (found by red-teaming the first fix). Acceptance parity alone
#     was NOT enough: GCC 16.2 and Clang 23.1.4 type `int (* const (p)(int))`
#     as `int *const (*)(int)` -- the stars and their qualifiers bind to the
#     function's RETURN type, and C11 6.7.6.3p8 adjusts the function-typed
#     parameter to a pointer. The first fix produced `int (*const)(int)`,
#     which silently accepted the declaration with the WRONG type. Only a
#     discriminating program (`*p(1)`, `p = 0`, a real indirect call) exposes
#     it. This is why the gate has three layers, not one.
#
# Layer 1: verdict parity over 49 declarator shapes (including invalid ones
#          that ALL THREE oracles must reject) x 2 lccc ABIs.
# Layer 2: 12 `p = 0` const-placement shapes + 5 type-sensitive bodies.
# Layer 3: end-to-end host-CPU execution. lccc compiles a K&R function
#          DEFINITION whose `*p(0)` only type-checks if `p` returns `int *`;
#          GCC 16.2 links the object and the executable must return 0.
#          Repeated at -O0/-O1/-O2/-O3 so no optimisation level can hide a
#          lowering bug. Assert lccc emits NO diagnostic for these fixtures.
#
# Oracles: the contracted GCC 16.2, Clang 23.1.4 when installed, and ICX
# 2026.2 when installed. A missing optional oracle is REPORTED, never
# silently treated as agreement (no claim is made for a compiler not run).
# ---------------------------------------------------------------------------
CLANG=${CLANG:-clang}
ICX=${ICX:-icx}
kr_oracles=("$GCC")
kr_names=("gcc")
if command -v "$CLANG" >/dev/null 2>&1; then
    kr_oracles+=("$CLANG")
    kr_names+=("clang $( "$CLANG" --version 2>/dev/null | sed -n '1p' )")
else
    printf 'K&R oracle note: Clang not installed; no Clang result claimed.\n'
fi
if command -v "$ICX" >/dev/null 2>&1; then
    kr_oracles+=("$ICX")
    kr_names+=("icx $( "$ICX" --version 2>/dev/null | sed -n '1p' )")
else
    printf 'K&R oracle note: ICX 2026.2 not installed; no ICX result claimed.\n'
fi

printf 'oracles: %s\n' "$("$GCC" --version 2>/dev/null | sed -n '1p')"
if command -v "$CLANG" >/dev/null 2>&1; then
    printf 'oracles: %s\n' "$("$CLANG" --version 2>/dev/null | sed -n '1p')"
fi

# verdict <cc> <std> <extra-flags...> : 0 = accepted, 1 = rejected.
# A non-zero exit AND an `error:` diagnostic both count as rejection; an
# `error:` with rc=0 (recovered parse) must not be mistaken for success.
verdict() {
    local cc=$1 std=$2; shift 2
    local out rc
    out=$("$cc" "-std=$std" -O2 "$@" -c "$tmp/kr.c" -o "$tmp/kr.o" 2>&1) && rc=0 || rc=$?
    if [ "$rc" -eq 0 ] && ! grep -q 'error:' <<<"$out"; then printf '0'; else printf '1'; fi
}

kr_accept=(
  'void f(void (* const funcp)(int)){}'
  'void f(void (* const (funcp))(int)){}'
  'void f(void (* const (funcp)(int))){}'
  'void f(void (*(funcp))(int)){}'
  'void f(void (*(funcp)(int))){}'
  'void f(void (* const (funcp)(int)), int x){}'
  'int (* const (fp))(int);'
  'void f(int (* const (p))(int));'
  'void f(int (* const (p)(int)));'
  'void f(int (*(p)(int)));'
  'void f(int (** (p)(int)));'
  'void f(int (* const (p)(int)), int y);'
  'int (*(fp))(int);'
  'int (*fp)(int);'
  'void f(int (*p)[4]);'
  'void f(int (*(*p))(int));'
  'void f(int ((*p))(int));'
  'void f(int (* const (*p))(int));'
  'void f(int (fn)(int));'
  'void f(int (fn(int)));'
  'int (* const (fp)(int))(int);'
  'void f(int (* const (p)(int))[4]);'
  'void f(int (*(*p)[4])(int));'
  'void f(int (* const ((p))(int)));'
  'void f(int (* (((p)))(int)));'
  'void f(int (* const (p)(int)), int (*(q)(int)));'
  'int (* const (p)(int))(int);'
  'int (* (p)(int))[4];'
  'void f(int (* const (p)(void)));'
  'void f(int (* const (p)()));'
  'void f(int (* const (p)(int, ...)));'
  'void f(char (* const (p)(int)));'
  'void f(void (* const (p)(int)));'
  'typedef int F(int); void f(F (* const (p)(int)));'
  'int (* const (p)(int x)) { (void)x; return 0; }'
  'static int cell; int * const (p)(int x) { (void)x; return &cell; }'
  'void f(int (* const (* const (p))(int))(int));'
  'void f(int (* const (p)(int)) const);'
  'void f(int (* const (p)(int)), ...);'
  'void f(int (* const (p)(int)), int, char);'
  'void f(int (* const (p)(int)) __attribute__((unused)));'
  'void f(register int (* const (p)(int)));'
  'void f(int (* volatile (p)(int)));'
  'void f(int (* const (p)(const int)));'
  'void f(int (* const (p)(int *)));'
)
# The 12 const-placement shapes: the parameter object's own `const` is a
# completely different question from a `const` on a RETURN pointer.
kr_decls=(
  'int * const p'
  'int ** const p'
  'int * const p(int)'
  'int ** const p(int)'
  'int (* const p)(int)'
  'int * const (*p)(int)'
  'int (* const (*p))(int)'
  'int (* const (p)(int))'
  'int (* (p)(int))'
  'int (** (p)(int))'
  'const int *p'
  'int *p[3]'
)
kr_semantic=(
  'int g(int (* const (p)(int))) { return *p(1); }'
  'int g(int (* (p)(int))) { return *p(1); }'
  'int g(int (** (p)(int))) { return **p(1); }'
  'int g(int (* const (p)(int)), int x) { return *p(x); }'
  'int g(int (* const p)(int)) { return *p(1); }'
  'typedef long T; void f(void (* const (funcp)(T)), T t) { (*funcp)(t); }'
  'int g(int * const p(int)) { return *p(1); }'
)
# C99/C11 spellings that are NOT keywords under -std=gnu89. They must be
# compared under -std=gnu11, or the oracle's verdict is a dialect artifact.
kr_gnu11=(
  'void f(int (* restrict (p)(int)));'
  'int f(int * restrict p){return 0;}'
  'int * restrict p(int);'
  'void f(int (* const (p)(int))[static 4]);'
  'void f(int a[static 4]);'
  'void f(int (* const p)(int)[4]);'
  'void f(int (*p)(int)[4]);'
  'void f(int (* const (p)(int))[4]);'
  'struct S { int m(int); };'
  'struct S { int (*m)(int); };'
  'struct S { int (* const (m)(int)); };'
)

kr_sources=()
for src in "${kr_accept[@]}"; do kr_sources+=("$src"); done
for decl in "${kr_decls[@]}"; do
    kr_sources+=("$(printf 'int g(%s) { p = 0; return 0; }' "$decl")")
done
for src in "${kr_semantic[@]}"; do kr_sources+=("$src"); done

kr_bad=0
kr_checks=0
for abi in x86_64 i686; do
    if [ "$abi" = i686 ]; then lcc=$CCC_I686; lf=(-m32); gf=(-m32); else lcc=$CCC; lf=(); gf=(); fi
    if [ ! -x "$lcc" ]; then
        printf 'K&R ABI note: %s binary %s not built; ABI skipped.\n' "$abi" "$lcc"
        continue
    fi
    for std_group in gnu89 gnu11; do
        if [ "$std_group" = gnu89 ]; then shapes=("${kr_sources[@]}"); else shapes=("${kr_gnu11[@]}"); fi
        for src in "${shapes[@]}"; do
            printf '%s\n' "$src" >"$tmp/kr.c"
            lv=$(verdict "$lcc" "$std_group" "${lf[@]}")
            for i in "${!kr_oracles[@]}"; do
                ov=$(verdict "${kr_oracles[$i]}" "$std_group" "${gf[@]}")
                kr_checks=$((kr_checks + 1))
                if [ "$lv" != "$ov" ]; then
                    printf 'FAIL: K&R %s %s %s %q lccc=%s oracle=%s\n' \
                        "$abi" "$std_group" "${kr_names[$i]}" "$src" "$lv" "$ov"
                    kr_bad=$((kr_bad + 1))
                fi
            done
        done
    done
done
check "K&R 3-oracle verdict parity: $(( ${#kr_sources[@]} + ${#kr_gnu11[@]} )) shapes x 2 ABIs ($kr_checks comparisons)" "$kr_bad"

# Layer 3: host-CPU execution. lccc owns the object; gcc 16.2 links it.
# `*p(0)` type-checks ONLY when the K&R declarator yields a function
# returning `int *`; the wrong `int (*const)(int)` type fails to compile.
kr_runtime=(
  'static int cell = 42; int (* const (p)(int x)) { (void)x; return &cell; } int main(void) { return *p(0) != 42; }'
  'static int cell = 42; static int *addr(int x) { (void)x; return &cell; } int kr(int (* (p)(int)), int v) { return *p(v); } int main(void) { return kr(addr, 0) != 42; }'
  'static int cell = 42; static int *addr(int x) { (void)x; return &cell; } int kr(int (* const (p)(int)), int v) { int *(*q)(int) = (int *(*)(int))p; return *q(v); } int main(void) { return kr((int *const (*)(int))addr, 0) != 42; }'
)
rt_bad=0
rt_checks=0
for src in "${kr_runtime[@]}"; do
    printf '%s\n' "$src" >"$tmp/kr.c"
    for opt in 0 1 2 3; do
        out=$("$CCC" -std=gnu89 -O"$opt" -c "$tmp/kr.c" -o "$tmp/kr.o" 2>&1) && rc=0 || rc=$?
        rt_checks=$((rt_checks + 1))
        if [ "$rc" -ne 0 ] || grep -q 'error:\|warning:' <<<"$out"; then
            printf 'FAIL: K&R runtime fixture -O%s emitted a diagnostic: %s\n' "$opt" "$out"
            rt_bad=$((rt_bad + 1)); continue
        fi
        if ! "$GCC" "$tmp/kr.o" -o "$tmp/kr.bin" >"$tmp/kr.link" 2>&1; then
            printf 'FAIL: K&R runtime fixture -O%s failed to link: %s\n' "$opt" "$(cat "$tmp/kr.link")"
            rt_bad=$((rt_bad + 1)); continue
        fi
        if ! "$tmp/kr.bin"; then
            printf 'FAIL: K&R runtime fixture -O%s returned a wrong value\n' "$opt"
            rt_bad=$((rt_bad + 1)); continue
        fi
    done
    # Oracle control: the same source must also compile/link/run under each
    # available oracle, proving the fixture itself is valid C.
    for i in "${!kr_oracles[@]}"; do
        rt_checks=$((rt_checks + 1))
        if ! "${kr_oracles[$i]}" -std=gnu89 -O2 -c "$tmp/kr.c" -o "$tmp/kr.o" >"$tmp/kr.occ" 2>&1 \
           || ! "$GCC" "$tmp/kr.o" -o "$tmp/kr.bin" >>"$tmp/kr.occ" 2>&1 \
           || ! "$tmp/kr.bin"; then
            printf 'FAIL: K&R runtime fixture rejected by oracle %s: %s\n' "${kr_names[$i]}" "$(cat "$tmp/kr.occ")"
            rt_bad=$((rt_bad + 1))
        fi
    done
done
check "K&R host-CPU execution + oracle control ($rt_checks checks, -O0..-O3, zero diagnostics)" "$rt_bad"


# ---------------------------------------------------------------------------
# D. Array-derivation constraints -- C11 6.7.6.3p7 / 6.7.6.2p4 / 6.7.6.3p1.
#
# Red-teaming Section C's array-qualifier check found FOUR classes of real
# divergence against GCC 16.2 and Clang 23.1.4 (75-shape differential probe; 86 shapes pinned in this gate):
#
#  1. The check keyed on "is this a function pointer" instead of "is this
#     the parameter's OUTERMOST array derivation". Quals in a LATER dimension
#     (`int a[4][static 4]`), in a pointer-to-array (`int (*a)[static 4]`)
#     and after an array-of-pointers group (`int (*p[3])[static 4]`) were
#     all accepted, while the oracles reject them.
#  2. Four copies of the `[...]` loop drifted apart: `[*]` was parsed only
#     in one, `static [*]` was accepted, and the first dimension's
#     qualifiers were not propagated through a parenthesised group, so
#     `int (a)[const 4]` wrongly allowed `a = 0`.
#  3. `int (a[4])` was a SYNTAX ERROR (dims inside parens were skipped after
#     the closing `)`); `int ((a[4]))` left an unbalanced `)`.
#  4. Fixing (3) exposed the array-of-functions case `int (a[4])(void)`,
#     which must be refused rather than parsed as a function pointer.
#
# Layer 1: verdict parity over all shapes x 2 lccc ABIs x every installed
#          oracle, under -std=gnu11 (C99 spellings are dialect keywords).
# Layer 2: exact diagnostic identity for the constraint violations, so a
#          future "accept everything" regression cannot pass on rc alone.
# Layer 3: host-CPU execution of parenthesised array parameters at
#          -O0..-O3 with zero lccc diagnostics, linked by GCC 16.2, with
#          an oracle control proving each fixture is valid C.
# ---------------------------------------------------------------------------
d_oracles=("$GCC")
d_names=("gcc 16.2")
if command -v "$CLANG" >/dev/null 2>&1; then
    d_oracles+=("$CLANG"); d_names+=("clang 23.1.4")
else
    printf 'D oracle note: Clang not installed; no Clang result claimed.\n'
fi
if command -v "$ICX" >/dev/null 2>&1; then
    d_oracles+=("$ICX"); d_names+=("icx 2026.2")
else
    printf 'D oracle note: ICX 2026.2 not installed; no ICX result claimed.\n'
fi

d_shapes=(
  'void f(int a[static 4]);'
  'void f(int a[const 4]);'
  'void f(int a[restrict 4]);'
  'void f(int a[volatile 4]);'
  'void f(int a[4][static 4]);'
  'void f(int a[4][const 4]);'
  'void f(int a[const 4][4]);'
  'void f(int a[static 4][const 4]);'
  'void f(int *a[static 4]);'
  'void f(int *a[4][static 4]);'
  'void f(int (a)[static 4]);'
  'void f(int (a)[const 4]);'
  'void f(int (a)[4][static 4]);'
  'void f(int (a)[static 4][4]);'
  'void f(int (a[static 4]));'
  'void f(int (a[4]));'
  'void f(int (a[]));'
  'void f(int (a[4][4]));'
  'void f(int (a[const 4]));'
  'void f(int (a[restrict 4]));'
  'void f(int (a[static 4][4]));'
  'void f(int ([4]));'
  'void f(int ([static 4]));'
  'void f(int ([4][static 4]));'
  'void f(int ((a[4])));'
  'void f(int ((a)[4]));'
  'void f(int (*a)[static 4]);'
  'void f(int (*a)[const 4]);'
  'void f(int (*a)[4][static 4]);'
  'void f(int (*a)[static 4][4]);'
  'void f(int (*a)[4][4]);'
  'void f(int (*a)[*]);'
  'void f(int (*a)[static *]);'
  'void f(int (* const p)[4]);'
  'void f(int (* const (p))[4]);'
  'void f(int (*p[3])[static 4]);'
  'void f(int (*p[3])[const 4]);'
  'void f(int (*p[3])[4]);'
  'void f(int (*p[static 3])[4]);'
  'void f(int (*p[static 3])[static 4]);'
  'void f(int (*p[3])[*]);'
  'void f(int (*p[static 3])[*]);'
  'void f(int (*p[3])(void));'
  'void f(int (*p[static 3])(void));'
  'void f(int (*p)(int)[static 4]);'
  'void f(int (*p)(int)[const 4]);'
  'void f(int (*p)(int)[4]);'
  'void f(int (*p)(void)[4]);'
  'void f(int (*p)(int)[4][static 4]);'
  'void f(int (* const p)(int)[4]);'
  'void f(int (* const (p)(int))[static 4]);'
  'void f(int (* const (p)(int))[4]);'
  'void f(int (* const (p)(int))[const 4]);'
  'void f(int (* const (p)(int))[4][static 4]);'
  'void f(int (* const (p)(int))[static 4][4]);'
  'void f(int a[*]);'
  'void f(int a[static *]);'
  'void f(int (a[*]));'
  'void f(int a[4](void));'
  'void f(int (a[4])(void));'
  'void f(int (a[static 4])(void));'
  'void f(int (a)(void)[4]);'
  'int f(int (a)[4]) { return 0; }'
  'int f(int (*a)[4]) { return 0; }'
  'int f(int (a)[static 4]) { return 0; }'
  'int f(int (a[static 4])) { return 0; }'
  'int f(int (a[4])) { return 0; }'
  'int f(int (a[const 4])) { a = 0; return 0; }'
  'int f(int (a[4])) { a = 0; return 0; }'
  'int f(int (a[static 4])) { a = 0; return 0; }'
  'int f(int a[const 4]) { a = 0; return 0; }'
  'int f(int a[4]) { a = 0; return 0; }'
  'int f(int (a)[const 4]) { a = 0; return 0; }'
  'int f(int (a)[static 4]) { a = 0; return 0; }'
  # C11 6.7.6.3p7: static needs a SIZE. This constraint outranks the
  # derivation-position check, including in nested/parenthesised groups.
  'void f(int a[static]);'
  'void f(int (a[static]));'
  'void f(int (*a)[static]);'
  'void f(int a[static 4][static]);'
  # Qualifiers alone are legal with an empty or VLA-star size.
  'void f(int a[const]);'
  'void f(int a[const *]);'
  'void f(int a[restrict]);'
  'void f(int a[*][static 4]);'
  'void f(int a[static 4][*]);'
  'void f(int a[volatile 4][const 4]);'
  'void f(int a[4][*]);'
  'void f(int (*a)[*][*]);'
)
d_msgs=(
  'void f(int a[4][static 4]);|static or type qualifiers in non-parameter array declarator'
  'void f(int (*a)[static 4]);|static or type qualifiers in non-parameter array declarator'
  'void f(int (*p[3])[const 4]);|static or type qualifiers in non-parameter array declarator'
  'void f(int (*p)(int)[const 4]);|static or type qualifiers in non-parameter array declarator'
  'void f(int a[static *]);|'\''static'\'' may not be used with an unspecified variable length array size'
  'void f(int a[4](void));|array of functions'
  'void f(int (a[4])(void));|array of functions'
  'void f(int (a)(void)[4]);|function returning an array'
  'void f(int (*p)(int)[4]);|function returning an array'
  'void f(int a[static]);|'\''static'\'' may not be used without an array size'
  'void f(int (a[static]));|'\''static'\'' may not be used without an array size'
  'void f(int (*a)[static]);|'\''static'\'' may not be used without an array size'
  'void f(int a[static 4][static]);|'\''static'\'' may not be used without an array size'
)
d_runtime=(
  'int f(int (a[static 4]), int i) { return a[i]; } int main(void) { int v[4] = {1,2,3,4}; return f(v, 2) != 3; }'
  'int f(int (a[4]), int i) { a = 0; return i; } int main(void) { int v[4] = {1,2,3,4}; return f(v, 7) != 7; }'
  'int f(int (*a)[4], int i) { return (*a)[i]; } int main(void) { int v[4] = {5,6,7,8}; return f(&v, 3) != 8; }'
)

d_bad=0
d_checks=0
for abi in x86_64 i686; do
    if [ "$abi" = i686 ]; then lcc=$CCC_I686; lf=(-m32); gf=(-m32); else lcc=$CCC; lf=(); gf=(); fi
    if [ ! -x "$lcc" ]; then
        printf 'D ABI note: %s binary %s not built; ABI skipped.\n' "$abi" "$lcc"
        continue
    fi
    for src in "${d_shapes[@]}"; do
        printf '%s\n' "$src" >"$tmp/kr.c"
        lv=$(verdict "$lcc" gnu11 "${lf[@]}")
        for i in "${!d_oracles[@]}"; do
            ov=$(verdict "${d_oracles[$i]}" gnu11 "${gf[@]}")
            d_checks=$((d_checks + 1))
            if [ "$lv" != "$ov" ]; then
                printf 'FAIL: D %s %s %q lccc=%s oracle=%s\n' \
                    "$abi" "${d_names[$i]}" "$src" "$lv" "$ov"
                d_bad=$((d_bad + 1))
            fi
        done
    done
done
printf 'D layer 1: %d declarator shapes x 2 ABIs x %d oracles = %d verdict comparisons\n' \
    "${#d_shapes[@]}" "${#d_oracles[@]}" "$d_checks"
check "D array-derivation verdict parity: ${#d_shapes[@]} shapes x 2 ABIs ($d_checks comparisons)" "$d_bad"

# Layer 2: lccc must not merely reject -- it must name the violated
# constraint. Exact wording is pinned to GCC 16.2 / Clang 23.1.4.
d_msg_bad=0
for pair in "${d_msgs[@]}"; do
    src=${pair%%|*}; needle=${pair#*|}
    printf '%s\n' "$src" >"$tmp/kr.c"
    d_out=$("$CCC" -std=gnu11 -O2 -c "$tmp/kr.c" -o "$tmp/kr.o" 2>&1) && d_rc=0 || d_rc=$?
    if [ "$d_rc" -ne 0 ] && grep -qF "$needle" <<<"$d_out"; then
        check "D diagnostic $needle :: $src" 0
    else
        printf 'FAIL: D diagnostic rc=%s expected "%s": %s\n' "$d_rc" "$needle" "$d_out"
        check "D diagnostic $needle :: $src" 1
    fi
done

printf 'D layer 2: %d exact diagnostic-identity pins\n' "${#d_msgs[@]}"

# Layer 3: host-CPU execution. lccc owns the object; GCC 16.2 links it.
d_rt_bad=0
d_rt_checks=0
for src in "${d_runtime[@]}"; do
    printf '%s\n' "$src" >"$tmp/kr.c"
    for opt in 0 1 2 3; do
        out=$("$CCC" -std=gnu11 -O"$opt" -c "$tmp/kr.c" -o "$tmp/kr.o" 2>&1) && rc=0 || rc=$?
        d_rt_checks=$((d_rt_checks + 1))
        if [ "$rc" -ne 0 ] || grep -q 'error:\|warning:' <<<"$out"; then
            printf 'FAIL: D runtime -O%s emitted a diagnostic: %s\n' "$opt" "$out"
            d_rt_bad=$((d_rt_bad + 1)); continue
        fi
        if ! "$GCC" "$tmp/kr.o" -o "$tmp/kr.bin" >"$tmp/kr.link" 2>&1; then
            printf 'FAIL: D runtime -O%s failed to link: %s\n' "$opt" "$(cat "$tmp/kr.link")"
            d_rt_bad=$((d_rt_bad + 1)); continue
        fi
        if ! "$tmp/kr.bin"; then
            printf 'FAIL: D runtime -O%s returned a wrong value\n' "$opt"
            d_rt_bad=$((d_rt_bad + 1))
        fi
    done
    for i in "${!d_oracles[@]}"; do
        d_rt_checks=$((d_rt_checks + 1))
        if ! "${d_oracles[$i]}" -std=gnu11 -O2 -c "$tmp/kr.c" -o "$tmp/kr.o" >"$tmp/kr.occ" 2>&1 \
           || ! "$GCC" "$tmp/kr.o" -o "$tmp/kr.bin" >>"$tmp/kr.occ" 2>&1 \
           || ! "$tmp/kr.bin"; then
            printf 'FAIL: D runtime fixture rejected by oracle %s: %s\n' "${d_names[$i]}" "$(cat "$tmp/kr.occ")"
            d_rt_bad=$((d_rt_bad + 1))
        fi
    done
done
printf 'D layer 3: %d runtime fixtures x (-O0..-O3 + %d oracle controls) = %d checks\n' \
    "${#d_runtime[@]}" "${#d_oracles[@]}" "$d_rt_checks"
check "D host-CPU execution + oracle control ($d_rt_checks checks, -O0..-O3)" "$d_rt_bad"

printf 'check_gnu_declaration_corners: %d passed, %d failed\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
