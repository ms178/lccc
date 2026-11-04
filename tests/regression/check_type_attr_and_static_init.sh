#!/usr/bin/env bash
# GCC 17 C-torture regression gate: pending TYPE attributes and static
# address-constant differences, on x86_64 and i686 at -O0 and -O2.
#
# Two independent classes of miscompile were found in the four-leg GCC 17
# torture matrix and are pinned here:
#
#  1. PENDING VECTOR ATTRIBUTE LEAK. `ParsedDeclAttrs::save_flags()` packs
#     only BOOLEAN flags, so `parsing_vector_size`, `parsing_vector_size_expr`
#     and `parsing_ext_vector_nelem` survived into the next declaration that
#     happened to be parsed. `gcc.c-torture/compile/vector-3.c`
#       __attribute__((vector_size(16))) float g(void) { float t = 1.0f; ... }
#     silently made the LOCAL `t` a 16-byte vector and dropped the attribute
#     from g's return type. A second path consumed a function RETURN TYPE's
#     attribute in the parameter list, because declarators (and therefore
#     parameter lists) are parsed before parse_function_def builds the return
#     type. Both directions are covered below, including prototypes and GNU
#     nested functions.
#
#  2. STATIC ADDRESS-CONSTANT DIFFERENCES. GCC 16.2 folds
#       int z = &"Foobar"[1] - &"Foobar"[0];
#     to 1, but rejects a difference between DIFFERENT literals with
#     "initializer element is not computable at load time". Clang 23.1.4
#     rejects even the same-text form; LCCC deliberately follows GCC's
#     documented GNU behaviour, which the kernel and gcc.c-torture rely on.
#     The lowering must fold the value, not merely stop rejecting it: reading
#     the static data at runtime proves that. Pointer-width integer casts keep
#     the BYTE difference and pointer casts scale by the pointee size; on i686
#     the pointer width is 4, not the host's 8, so both ABIs are exercised.
#
# Every expectation was measured against GCC 16.2 and Clang 23.1.4 where
# those compilers implement the construct (Clang has no nested functions and
# rejects same-literal differences).
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
CCC_I686=${CCC_I686:-./target/fastbuild/lccc-i686}
GCC=${GCC:-gcc-16}
command -v "$GCC" >/dev/null 2>&1 || GCC=gcc
[ -x "$CCC" ] || { echo "check_type_attr_and_static_init: SKIP ($CCC not built)"; exit 0; }

tmp=${TMPDIR:-/tmp}/lccc-type-attr.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"

# ── Runtime program: vector ABI, attribute scope, folded static data ───────
cat >"$tmp/runtime.c" <<'C'
#include <stdio.h>

typedef int v4si __attribute__((vector_size(16)));

static int fails;
#define CHECK(e) do { if (!(e)) { printf("FAIL %d: %s\n", __LINE__, #e); ++fails; } } while (0)

/* A typedef'd vector parameter and return must keep their 16-byte layout. */
static v4si add(v4si a, v4si b) { return a + b; }

/* Direct (non-typedef) vector_size in BOTH the return specifier and the
   parameter specifier. Before the fix the parameter list consumed the return
   attribute: `return x` was diagnosed as "have vector but expected int". */
__attribute__((vector_size(16))) int ident(__attribute__((vector_size(16))) int x)
{
    float scalar_local = 1.0f;   /* must NOT inherit the vector attribute */
    if (sizeof(scalar_local) != 4) return (__attribute__((vector_size(16))) int){9,9,9,9};
    return x;
}

/* A vector RETURN type with a scalar local, returning a valid vector. This
   is vector-3.c's shape with a well-typed return. */
__attribute__((vector_size(16))) float vec_return(void)
{
    float t = 1.0f;
    return (__attribute__((vector_size(16))) float){t, t, t, t};
}

/* File-scope address constants: narrow literal differences. */
int diff_a = &"Foobar"[1] - &"Foobar"[0];
int diff_d = &"Foobar"[3] - &"Foobar"[1];
int diff_scaled = (int *)&"Foobar"[4] - (int *)&"Foobar"[0];
long diff_long = (long)&"Foobar"[1] - (long)&"Foobar"[0];
/* Commutative and chained pointer arithmetic (GCC 16.2 folds all of them). */
int diff_commutative = 1 + "Foobar" - "Foobar";
int diff_chain = ("Foobar" + 3 - 1) - ("Foobar" + 1);
/* Wide literals: pointer difference is in ELEMENTS, a cast changes the scale. */
int diff_wide = &L"Foobar"[1] - &L"Foobar"[0];
int diff_wide2 = &L"Foobar"[3] - &L"Foobar"[1];
int diff_wide_cast = (int *)&L"Foobar"[4] - (int *)&L"Foobar"[0];

int main(void)
{
    v4si x = {1, 2, 3, 4}, y = {10, 20, 30, 40};
    v4si z = add(x, y);
    int *p = (int *)&z;
    CHECK(p[0] == 11 && p[1] == 22 && p[2] == 33 && p[3] == 44);

    v4si w = ident(x);
    p = (int *)&w;
    CHECK(p[0] == 1 && p[1] == 2 && p[2] == 3 && p[3] == 4);

    CHECK(sizeof(v4si) == 16);
    CHECK(sizeof(ident(x)) == 16);
    CHECK(sizeof(vec_return()) == 16);

    CHECK(diff_a == 1);
    CHECK(diff_d == 2);
    CHECK(diff_scaled == 1);
    CHECK(diff_long == 1);
    CHECK(diff_commutative == 1);
    CHECK(diff_chain == 1);
    CHECK(diff_wide == 1);
    CHECK(diff_wide2 == 2);
    CHECK(diff_wide_cast == 4);

    /* The same fold in a block-scope `static` (a different sema path). */
    static int block_static = &"Foobar"[2] - &"Foobar"[0];
    CHECK(block_static == 2);

    printf(fails ? "FAIL %d\n" : "PASS %d\n", fails);
    return fails != 0;
}
C

# ── Attribute-scope compile-only shapes (must all be accepted) ─────────────
cat >"$tmp/scope_typedef.c" <<'C'
typedef float v4sf __attribute__((vector_size(16)));
v4sf a;
float t = 1.0f;                 /* no leak after a vector typedef */
C
cat >"$tmp/scope_struct.c" <<'C'
struct S { __attribute__((vector_size(16))) float m; };
float q = 2.0f;                 /* no leak after a vector member */
C
cat >"$tmp/scope_globals.c" <<'C'
float a = 1.0f;
__attribute__((vector_size(16))) float b;
float c = 3.0f;                 /* no leak after a vector global */
C
cat >"$tmp/scope_param.c" <<'C'
#include <stddef.h>
int f(__attribute__((vector_size(16))) int t) { return (int)sizeof(t); }
int g(void) { return f((__attribute__((vector_size(16))) int){1,2,3,4}) == 16; }
C
cat >"$tmp/scope_proto.c" <<'C'
typedef int v4si __attribute__((vector_size(16)));
v4si f(v4si x);
v4si f(v4si x) { return x; }
__attribute__((vector_size(16))) int g(__attribute__((vector_size(16))) int x);
__attribute__((vector_size(16))) int g(__attribute__((vector_size(16))) int x) { return x; }
C
cat >"$tmp/scope_nested.c" <<'C'
/* GNU nested function: the return type is built AFTER the body, so the
   pending attribute must survive parse_compound_stmt's save/restore. */
typedef int v4si __attribute__((vector_size(16)));
int main(void)
{
    v4si a = {1,2,3,4};
    __attribute__((vector_size(16))) int inner(__attribute__((vector_size(16))) int x)
    { return x; }
    v4si b = inner(a);
    int *p = (int *)&b;
    return p[3] - 4;
}
C

# ── Negatives ──────────────────────────────────────────────────────────────
cat >"$tmp/bad_return.c" <<'C'
__attribute__((vector_size(16))) float g(void) { float t = 1.0f; return t; }
C
cat >"$tmp/bad_diff.c" <<'C'
int b = &"Foobar"[1] - &"Baz"[0];
int c = &"Foobar"[1] - &"Fooba"[0];
C
cat >"$tmp/bad_kind.c" <<'C'
int a = &"Foobar"[1] - &L"Foobar"[0];
C

# ── Runtime program: GCC 17 torture compile legs fixed in the same audit ───
# vector-2.c (vector STRUCT MEMBER), 20010113-1.c / 20050516-1.c (address of
# a member through a nested/absolute pointer chain) and pr48517.c (an
# UNSIZED static array initialized from a whole-array compound literal).
# Every value is additionally checked against the GCC 16.2 oracle.
cat >"$tmp/runtime2.c" <<'C'
#include <stdio.h>
#define vector __attribute__((vector_size(16)))
struct ss { vector float mVec; };
vector float getCapsule(vector int t)
{
  vector float t1 = (vector float)t;
  struct ss y = {t1};
  *((float*)&y.mVec) = 1.0;
  float local = 2.0f;          /* must NOT inherit the member's attribute */
  (void)local;
  return y.mVec;              /* must be a vector, not a plain float */
}

struct { struct { int x; int y; } p; } v;
int *z = &((&(v.p))->y);
struct S { char s; int pad; };
struct T { struct S t; };
struct S *const p = &((struct T * const)(0x4000))->t;

static void arr(void)
{
  static const unsigned short a[] = (const unsigned short []) { 0x0D2B, 0x1234 };
  printf("%zu %u %u\n", sizeof(a), a[0], a[1]);
}

int main(void)
{
  vector int t = {1,2,3,4};
  vector float r = getCapsule(t);
  float *f = (float *)&r;
  printf("%zu %g %d %p\n", sizeof(r), (double)f[0],
         (int)((char *)z - (char *)&v), (void *)p);
  arr();
  return 0;
}
C

expect_error() {
    local file=$1 want=$2 out rc=0
    out=$("$compiler" ${mflag:-} -O2 -c "$tmp/$file" -o "$tmp/neg.o" 2>&1) || rc=$?
    [ "$rc" -ne 0 ] || { echo "$label/$file: accepted an invalid program"; return 1; }
    case "$out" in
        *"$want"*) ;;
        *) echo "$label/$file: expected '$want', got: $out"; return 1 ;;
    esac
    echo "$label/$file: rejected ($want)"
}

run_target() {
    local label=$1 compiler=$2 mflag=${3:-}
    local opt
    for opt in -O0 -O2; do
        "$compiler" $mflag "$opt" -c "$tmp/runtime.c" -o "$tmp/runtime.o"
        "$GCC" $mflag "$tmp/runtime.o" -o "$tmp/runtime"
        local out rc=0
        out=$("$tmp/runtime") || rc=$?
        [ "$out" = "PASS 0" ] || {
            echo "$label $opt runtime mismatch: rc=$rc out='$out'"; return 1; }
        echo "$label $opt runtime: $out"

        # GCC 16.2 is the oracle for the very same source: equal values prove
        # the folds are not LCCC-specific inventions.
        "$GCC" $mflag "$opt" "$tmp/runtime.c" -o "$tmp/runtime_gcc"
        local gout grc=0
        gout=$("$tmp/runtime_gcc") || grc=$?
        [ "$gout" = "$out" ] || {
            echo "$label $opt GCC16.2 oracle mismatch: out='$gout' rc=$grc"; return 1; }
        echo "$label $opt GCC16.2 oracle: $gout"
    done

    # A vector return type must be applied to the FUNCTION, never to its
    # first local. The old bug produced a second "have vector but expected
    # float" diagnostic at `float t`; assert exactly one error, at `return t`.
    local bout brc=0
    bout=$("$compiler" ${mflag:-} -O2 -c "$tmp/bad_return.c" -o "$tmp/bad.o" 2>&1) || brc=$?
    [ "$brc" -ne 0 ] || { echo "$label/bad_return.c: accepted an invalid return"; return 1; }
    # Count DIAGNOSTIC lines only. `ccc: error: 1 error(s) during semantic
    # analysis` is a summary, not a second diagnostic; requiring exactly one
    # .c:line:col diagnostic is what proves the local `float t` was NOT also
    # mis-typed (the old bug emitted two conflicting errors).
    local nerr
    nerr=$(printf '%s\n' "$bout" | grep -Ec '\.c:[0-9]+:[0-9]+: error:' || true)
    [ "$nerr" -eq 1 ] || {
        echo "$label/bad_return.c: expected exactly 1 error, got $nerr: $bout"; return 1; }
    case "$bout" in
        *"have 'float' but expected '__attribute__((vector_size(16))) float'"*) ;;
        *) echo "$label/bad_return.c: wrong diagnostic: $bout"; return 1 ;;
    esac
    echo "$label/bad_return.c: exactly one diagnostic at the return"
    # GCC 16.2 must reject the same program (Clang 23.1.4 does too).
    if "$GCC" ${mflag:-} -O2 -c "$tmp/bad_return.c" -o "$tmp/bad_gcc.o" >/dev/null 2>&1; then
        echo "$label/bad_return.c: GCC 16.2 unexpectedly accepted it"; return 1
    fi
    echo "$label/bad_return.c: GCC 16.2 rejects it too"

    expect_error bad_diff.c "initializer element is not constant"
    # Identical TEXT but different literal kinds are different objects and
    # incompatible pointer types; GCC 16.2 and LCCC both reject this.
    expect_error bad_kind.c "invalid operands to binary -"

    local f
    for f in scope_typedef.c scope_struct.c scope_globals.c scope_param.c \
             scope_proto.c scope_nested.c; do
        "$compiler" ${mflag:-} -O2 -c "$tmp/$f" -o "$tmp/pos.o" || {
            echo "$label/$f: valid program rejected"; return 1; }
        echo "$label/$f: accepted"
    done

    # A GNU nested function returning a 16-byte vector must return the
    # OBJECT, not the address bits. On i686 the old `Cast { Ptr -> I128 }`
    # panicked in the scalar cast emitter; on both ABIs it would have
    # returned a stack address. This is a runtime proof at -O0 and -O2,
    # with GCC 16.2 as the value oracle.
    for opt in -O0 -O2; do
        "$compiler" ${mflag:-} "$opt" "$tmp/scope_nested.c" -o "$tmp/nested_lccc" || {
            echo "$label $opt: nested vector return failed to link"; return 1; }
        local nout nrc=0
        nout=$("$tmp/nested_lccc") || nrc=$?
        [ "$nrc" -eq 0 ] || {
            echo "$label $opt nested vector return: rc=$nrc"; return 1; }
        "$GCC" ${mflag:-} "$opt" "$tmp/scope_nested.c" -o "$tmp/nested_gcc"
        local g_rc=0
        "$tmp/nested_gcc" || g_rc=$?
        [ "$g_rc" -eq "$nrc" ] || {
            echo "$label $opt nested vector return: GCC16.2 rc=$g_rc vs LCCC rc=$nrc"; return 1; }
        echo "$label $opt nested vector return: rc=0 (GCC16.2 parity)"
    done

    # The four additional GCC 17 compile-leg families, at -O0 and -O2, with
    # GCC 16.2 compiling the SAME source as the value oracle. This proves
    # sizeof(a) is not left at the incomplete-array placeholder, that the
    # struct member stayed a vector, and that every member offset is right
    # on BOTH pointer widths.
    for opt in -O0 -O2; do
        "$compiler" ${mflag:-} "$opt" -msse "$tmp/runtime2.c" -o "$tmp/r2_lccc" || {
            echo "$label $opt: GCC17 fixed-family program failed to link"; return 1; }
        "$GCC" ${mflag:-} "$opt" -msse "$tmp/runtime2.c" -o "$tmp/r2_gcc" || {
            echo "$label $opt: GCC16.2 oracle failed to link"; return 1; }
        local lout gout lrc=0 grc=0
        lout=$("$tmp/r2_lccc") || lrc=$?
        gout=$("$tmp/r2_gcc") || grc=$?
        [ "$lrc" -eq 0 ] || { echo "$label $opt runtime2: rc=$lrc"; return 1; }
        [ "$grc" -eq 0 ] || { echo "$label $opt runtime2: GCC rc=$grc"; return 1; }
        [ "$lout" = "$gout" ] || {
            echo "$label $opt runtime2: LCCC='$lout' GCC16.2='$gout'"; return 1; }
        echo "$label $opt GCC17 fixed-family runtime: PASS ($lout)"
    done
}

run_target x86_64 "$CCC" ""
if [ -x "$CCC_I686" ] && [ "${SKIP_I686:-0}" != 1 ]; then
    run_target i686 "$CCC_I686" "-m32"
else
    echo "i686: SKIP ($CCC_I686 not built)"
fi
echo "type-attribute scope and static-init address-constant contract: PASS"
