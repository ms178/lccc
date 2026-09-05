/*
 * Call arguments are a conversion boundary: the recorded argument type must
 * describe the value that was actually materialised.
 *
 * Two historic defects, one root cause.
 *
 * `get_expr_type` answers a *storage-level* question and reports `I64` on LP64
 * for an `IntLiteral` operand, so an arithmetic `BinaryOp` such as `i + 1` was
 * typed `I64` even though the emitted `BinOp` correctly carries `ty: I32`.
 * That disagreement was recorded straight into `CallInfo::arg_types`:
 *
 *   1. `__builtin_memcmp(p, q, i + 1)` produced
 *      `args: [.., v:I32], arg_types: [.., I64]` with no conversion at all,
 *      because the LibcAlias builtin path had no libc signature to convert
 *      against. At -O0 the 32-bit value lives in a 4-byte stack slot and the
 *      8-byte argument load pulled the ADJACENT slot in as its high half:
 *      `%rdx` became `0x1_00000007` and memcmp ran off both buffers
 *      (gcc.c-torture/execute/pr59229.c, pr109938.c, pr109986.c).
 *
 *   2. With a prototype in scope the same lie produced
 *      `Cast { from_ty: I64, to_ty: U64 }` — a same-width no-op — where C
 *      requires `int -> size_t` to SIGN-extend first (C11 6.3.1.3), so a
 *      negative length was zero-extended into a merely-large value instead of
 *      SIZE_MAX-relative.
 *
 * The test pins both: a runtime-length compare through the builtin and through
 * a real prototype, the `sizeof(x) != 0` comparison-operand shape from the PR
 * tests (a `Cmp` materialises an I32 while the storage query says I64), and
 * the negative-int-to-size_t conversion, checked against values the compiler
 * cannot constant-fold.
 */
#include <stdio.h>
#include <string.h>

extern int printf(const char *, ...);

int g_len;                 /* global so the length is not a known constant */
volatile int v_neg = -1;

/* Observes the size_t its caller actually passed. */
__attribute__((noinline)) unsigned long observe_size(unsigned long n) { return n; }

__attribute__((noinline)) int builtin_cmp(const char *p, const char *q) {
    /* size_t parameter fed by an `int` expression. */
    return __builtin_memcmp(p, q, g_len + 1);
}

__attribute__((noinline)) int proto_cmp(const char *p, const char *q) {
    return memcmp(p, q, (size_t) (g_len + 1));
}

__attribute__((noinline)) int builtin_cmp_cmpexpr(const char *p, const char *q) {
    /* The pr109938/pr109986 shape: the length operand is a COMPARISON, which
     * materialises a 4-byte value while the storage-level query says I64. */
    return __builtin_memcmp(p, q, sizeof(int) != 0);
}

__attribute__((noinline)) void builtin_set(char *p, int fill) {
    __builtin_memset(p, fill, g_len + 1);
}

__attribute__((noinline)) void builtin_cpy(char *d, const char *s) {
    __builtin_memcpy(d, s, g_len + 1);
}

int main(void) {
    const char *ref = "abcdefg";
    char buf[8] = {'a', 'b', 'c', 'd', 'e', 'f', 'g', 0};
    char dst[8];
    int fails = 0;

    for (g_len = 0; g_len < 7; ++g_len) {
        if (builtin_cmp(buf, ref) != 0) {
            printf("builtin_memcmp(len=%d) != 0\n", g_len + 1);
            ++fails;
        }
        if (proto_cmp(buf, ref) != 0) {
            printf("proto memcmp(len=%d) != 0\n", g_len + 1);
            ++fails;
        }
        if (builtin_cmp_cmpexpr(buf, ref) != 0) {
            printf("builtin_memcmp(cmpexpr len) != 0 at g_len=%d\n", g_len);
            ++fails;
        }
        /* A real difference must still be reported. */
        buf[g_len] ^= 0x20;
        if (builtin_cmp(buf, ref) == 0) {
            printf("builtin_memcmp missed a difference at %d\n", g_len);
            ++fails;
        }
        buf[g_len] ^= 0x20;

        memset(dst, 0x7f, sizeof dst);
        builtin_cpy(dst, ref);
        if (memcmp(dst, ref, (size_t) g_len + 1) != 0 ||
            (unsigned char) dst[g_len + 1] != 0x7fu) {
            printf("builtin_memcpy wrote the wrong extent at g_len=%d\n", g_len);
            ++fails;
        }

        memset(dst, 0x7f, sizeof dst);
        builtin_set(dst, 'Z');
        {
            int k, ok = 1;
            for (k = 0; k <= g_len; ++k)
                if (dst[k] != 'Z')
                    ok = 0;
            if (!ok || (unsigned char) dst[g_len + 1] != 0x7fu) {
                printf("builtin_memset wrote the wrong extent at g_len=%d\n", g_len);
                ++fails;
            }
        }
    }

    /*
     * int -> size_t must sign-extend, so (size_t)(-1) is SIZE_MAX and
     * (size_t)(-1 + 1) is 0. A zero-extending (or truncating) conversion would
     * yield 0xFFFFFFFF and 0.
     */
    {
        unsigned long want_max = (unsigned long) (long) v_neg;
        unsigned long got = observe_size(v_neg);
        if (got != want_max) {
            printf("int->size_t: got %lu want %lu\n", got, want_max);
            ++fails;
        }
        if (observe_size(v_neg - 1) != (unsigned long) (long) (v_neg - 1)) {
            printf("int->size_t on an expression is wrong\n");
            ++fails;
        }
        /* And through the builtin's own signature. */
        if (__builtin_memcmp(buf, ref, (unsigned long) (v_neg + 1)) != 0) {
            printf("__builtin_memcmp with a zero-length (v_neg+1) is wrong\n");
            ++fails;
        }
    }

    printf("builtin_size_t_arg_width: %s\n", fails ? "FAIL" : "OK");
    return fails != 0;
}
