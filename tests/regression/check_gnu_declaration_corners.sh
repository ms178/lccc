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

printf 'check_gnu_declaration_corners: %d passed, %d failed\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
