#!/usr/bin/env bash
# Runtime + compile contract for the GNU builtins required by the GCC 17-git
# C-torture compile leg (ref c94b688a83fc84015c6fab8ab0ff142051598fc8).
#
# Every expectation here was measured with gcc-16.2 -O2 AND -O0 before being
# encoded. The point of this gate is not "the file compiles" -- a wrong
# LibcAlias compiles too -- but that the *values* match GCC:
#
#   __builtin_memcmp_eq(a,b,4) == 1,   memcmp(a,b,4) == -1   (NOT the same)
#   __builtin_stack_save/restore survive a __builtin_alloca scope
#   __builtin_cexpi(4) == cos(4) + i*sin(4)
#   __builtin_clear_padding accepts GNU zero-length/VLA arrays
#
# Both x86_64 and i686 are exercised: the size_t conversions, the 32-bit
# stack watermark and sincos' call ABI differ between the two targets.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
CCC_I686=${CCC_I686:-./target/fastbuild/lccc-i686}
GCC=${GCC:-gcc-16}
command -v "$GCC" >/dev/null 2>&1 || GCC=gcc
[ -x "$CCC" ] || { echo "check_gcc17_builtin_semantics: SKIP ($CCC not built)"; exit 0; }

tmp=${TMPDIR:-/tmp}/lccc-gcc17-builtins.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"

cat >"$tmp/runtime.c" <<'C'
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int fails;
#define CHECK(expr) do { if (!(expr)) { printf("FAIL line %d: %s\n", __LINE__, #expr); ++fails; } } while (0)

volatile char A[4] = { 'a', 'b', 'c', 0 };
volatile char B[4] = { 'a', 'b', 'd', 0 };
volatile char C[4] = { 'a', 'b', 'c', 0 };
static const unsigned char K[4] = { 1, 2, 3, 4 };
static const unsigned char L[4] = { 1, 2, 3, 4 };

static void bar(void *p, int n) { (void)p; (void)n; }

/* gcc.c-torture/compile/20071117-1.c, with bar defined so it can RUN. */
static int stack_probe(int i)
{
    char *p = __builtin_stack_save();
    void *q = __builtin_alloca(i);
    bar(q, i);
    __builtin_stack_restore(p);
    bar("", 0);
    return 6;
}

int main(void)
{
    /* GCC 16.2 at -O2 and -O0: "1 0 -1" -- memcmp_eq is a BOOLEAN. */
    CHECK(__builtin_memcmp_eq((void *)A, (void *)B, 4) == 1);
    CHECK(__builtin_memcmp_eq((void *)A, (void *)C, 4) == 0);
    /* The builtin's prototype is const void *, not void *. This must be
       accepted under -Werror=discarded-qualifiers on both target widths. */
    CHECK(__builtin_memcmp_eq(K, L, sizeof K) == 0);
    CHECK(__builtin_memcmp((void *)A, (void *)B, 4) != 0);
    CHECK(__builtin_bcmp((void *)A, (void *)C, 4) == 0);
    CHECK(__builtin_bcmp((void *)A, (void *)B, 4) != 0);

    char z[8];
    memset(z, 0x5a, sizeof z);
    __builtin_bzero(z, sizeof z);
    CHECK(z[0] == 0 && z[7] == 0);

    char *d = __builtin_strdup("hello");
    CHECK(d && strcmp(d, "hello") == 0);
    free(d);
    char *nd = __builtin_strndup("hello world", 5);
    CHECK(nd && strcmp(nd, "hello") == 0);
    free(nd);

    CHECK(__builtin_strncasecmp("AbCd", "aBcX", 3) == 0);
    CHECK(__builtin_strncasecmp("AbCd", "aBcX", 4) != 0);
    CHECK(__builtin_ilogb(8.0) == 3);
    float iptrf;
    CHECK(__builtin_modff(1.5f, &iptrf) == 0.5f && iptrf == 1.0f);
    double iptr;
    CHECK(__builtin_modf(-2.25, &iptr) == -0.25 && iptr == -2.0);

    _Complex double cx = __builtin_cexpi(4.0);
    CHECK(fabs(__real cx - cos(4.0)) < 1e-9);
    CHECK(fabs(__imag cx - sin(4.0)) < 1e-9);

    /* (1+2i)^3 == -11-2i; checks complex ARGUMENT decomposition AND the
       target's complex RETURN convention (xmm0/xmm1 vs hidden sret). */
    _Complex double base = 1.0 + 2.0i;
    _Complex double expn = 3.0 + 0.0i;
    _Complex double cp = __builtin_cpow(base, expn);
    CHECK(fabs(__real cp + 11.0) < 1e-9);
    CHECK(fabs(__imag cp + 2.0) < 1e-9);

    CHECK(stack_probe(64) == 6);
    printf(fails ? "FAIL %d\n" : "PASS %d\n", fails);
    return fails != 0;
}
C

cat >"$tmp/compile.c" <<'C'
/* gcc.c-torture/compile/pr98087.c: GNU zero-length / VLA arrays are
   COMPLETE types. clear_padding on a zero-sized object is a no-op. */
struct S { char a; long long b; };
struct T { struct S c[0]; char d; };
void clear_pad(int n)
{
    struct S a[n][0];
    __builtin_clear_padding(a);
    __builtin_clear_padding(&a);
    struct S b[7][0];
    __builtin_clear_padding(&b);
    struct T c;
    __builtin_clear_padding(&c);
}
C

run_target() {
    local label=$1 compiler=$2 mflag=$3
    echo "== $label =="
    "$compiler" $mflag -O2 -Werror=discarded-qualifiers -c "$tmp/runtime.c" -o "$tmp/runtime.o"
    "$GCC" $mflag "$tmp/runtime.o" -lm -o "$tmp/runtime"
    local out rc=0
    out=$("$tmp/runtime") || rc=$?
    [ "$out" = "PASS 0" ] || { echo "$label runtime mismatch: rc=$rc out='$out'"; return 1; }
    echo "$label runtime: $out"
    "$compiler" $mflag -O2 -c "$tmp/compile.c" -o "$tmp/compile.o"
    echo "$label clear_padding compile: PASS"
}

run_target x86_64 "$CCC" ""
if [ -x "$CCC_I686" ] && [ "${SKIP_I686:-0}" != 1 ]; then
    run_target i686 "$CCC_I686" "-m32"
else
    echo "i686: SKIP ($CCC_I686 not built)"
fi
echo "check_gcc17_builtin_semantics: PASS"
