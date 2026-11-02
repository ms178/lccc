/*
 * iv_widen: an int->float conversion of a widened induction variable is a
 * value CONVERSION, never a bit-level extension.
 *
 * Historic defect (found by gcc.c-torture/execute/20060420-1.c, -O2/-O3):
 * `iv_widen.rs` admitted `Cast { from_ty: I32, to_ty: F32 }` as a
 * `MemberKind::WidenCast` because the admission test was
 *
 *     to_ty.size() >= 4 && to_ty != Ptr && is_unsigned_ty(to) == is_unsigned_ty(from)
 *
 * and `F32` satisfies all three (size 4, not Ptr, and `is_unsigned_ty` is
 * `false` for both `F32` and `I32`). The cast was therefore DROPPED and its
 * consumers re-classified as integer closure members, so
 *
 *     float e = 11 * (float) i;      ->   Mul(i:I64, widen_const(F32(11.0)))
 *                                    ->   Mul(i:I64, I64(0))          == 0
 *
 * i.e. every `K * (float) i` in a counted loop with an addressing use of `i`
 * silently evaluated to ZERO. The same hole let float-typed `BinOp`s of width
 * 8 (`F64`, `D64`) pass the `ty.size() >= 8` "wide chain op" shortcut.
 *
 * This test pins the whole family: signed/unsigned IVs, f32/f64/long double,
 * both operand orders, multiply/add/subtract/divide, a float select, a
 * float compare, and the induction variable escaping the loop into a float.
 * Every loop keeps a GEP use of `i` so the widening candidacy (which
 * requires an addressing anchor) actually fires.
 *
 * Reference values are computed with a `volatile` step so no optimizer can
 * constant-fold the expected side into the same wrong closed form.
 */
#include <stdio.h>

#define N 16

static float src_f[N];
static double src_d[N];
static unsigned char bytes[N];

/* --- 1. the exact 20060420-1.c shape: K * (float) i ------------------- */
static void mul_f32(float *dst, const float *a, int n) {
    int i;
    for (i = 0; i < n; ++i)
        dst[i] = a[i] + 11 * (float) i;
}

/* --- 2. constant on the right, and a second scale in the same body ---- */
static void mul_f32_rhs(float *dst, const float *a, int n) {
    int i;
    for (i = 0; i < n; ++i)
        dst[i] = a[i] + (float) i * 11 + 12 * (float) i;
}

/* --- 3. double, and a subtraction (Sub is also a "const scale" arm) --- */
static void mul_f64(double *dst, const double *a, int n) {
    int i;
    for (i = 0; i < n; ++i)
        dst[i] = a[i] + 7.5 * (double) i - 2.5 * (double) i;
}

/* --- 4. unsigned IV (the zext path) with a float divide ---------------- */
static void div_u32(float *dst, const float *a, unsigned n) {
    unsigned i;
    for (i = 0; i < n; ++i)
        dst[i] = a[i] + (float) i / 4.0f;
}

/* --- 5. narrow (u8) loads feeding a float scale, IV also indexes ------- */
static void byte_scale(float *dst, const unsigned char *b, int n) {
    int i;
    for (i = 0; i < n; ++i)
        dst[i] = (float) b[i] * 3 + 5 * (float) i;
}

/* --- 6. float SELECT whose data operands derive from the IV ----------- */
static void select_f32(float *dst, const float *a, int n) {
    int i;
    for (i = 0; i < n; ++i) {
        float lo = 2 * (float) i;
        float hi = 3 * (float) i;
        dst[i] = a[i] + ((i & 1) ? hi : lo);
    }
}

/* --- 7. float COMPARE against an IV-derived value --------------------- */
static int cmp_f32(const float *a, int n) {
    int i, hits = 0;
    for (i = 0; i < n; ++i)
        if (a[i] > 9 * (float) i)
            hits += i;
    return hits;
}

/* --- 8. long double (F128 carrier) scale ------------------------------ */
static void mul_ld(double *dst, const double *a, int n) {
    int i;
    for (i = 0; i < n; ++i)
        dst[i] = (double) ((long double) a[i] + (long double) 13 * (long double) i);
}

/* --- 9. the IV escapes the loop as a float (escape-truncation path) ---- */
static float escape_f32(const float *a, int n) {
    int i;
    float last = 0.0f;
    for (i = 0; i < n; ++i)
        last = a[i];
    /* `i` is live out and is consumed by a conversion, not an extension. */
    return last + 6 * (float) i;
}

/*
 * --- 10. the *exact* 20060420-1.c shape ------------------------------
 *
 * A cold `noreturn` exit inside the loop keeps the vectorizer away, so the
 * residual scalar counted loop reaches iv_widen and the in-loop
 * `Cast { I32 -> F32 }` is presented to the closure classifier. This is the
 * arm that produced `Mul(i:I64, I64(0))`; without the cold call the loop is
 * vectorized and the defect hides.
 */
extern void iv_widen_report_mismatch(int i, float got, float want);

static int cold_exit_scale(const float *a, int n) {
    int i;
    for (i = 0; i < n; ++i) {
        float e = 11 * (float) i;
        if (a[i] != e)
            iv_widen_report_mismatch(i, a[i], e);
    }
    return i;
}

static int mismatches;
static int first_bad_i = -1;
static float first_bad_got, first_bad_want;

void iv_widen_report_mismatch(int i, float got, float want) {
    if (first_bad_i < 0) {
        first_bad_i = i;
        first_bad_got = got;
        first_bad_want = want;
    }
    ++mismatches;
}

int main(void) {
    volatile int one = 1;
    int i, fails = 0;
    float out[N];
    double outd[N];

    for (i = 0; i < N; ++i) {
        src_f[i] = (float) (i * one) * 0.5f;
        src_d[i] = (double) (i * one) * 0.25;
        bytes[i] = (unsigned char) (i * one + 3);
    }

    mul_f32(out, src_f, N);
    for (i = 0; i < N; ++i) {
        float want = src_f[i] + 11.0f * (float) (i * one);
        if (out[i] != want) {
            printf("mul_f32[%d] = %g want %g\n", i, out[i], want);
            ++fails;
        }
    }

    mul_f32_rhs(out, src_f, N);
    for (i = 0; i < N; ++i) {
        float want = src_f[i] + (float) (i * one) * 11.0f + 12.0f * (float) (i * one);
        if (out[i] != want) {
            printf("mul_f32_rhs[%d] = %g want %g\n", i, out[i], want);
            ++fails;
        }
    }

    mul_f64(outd, src_d, N);
    for (i = 0; i < N; ++i) {
        double want = src_d[i] + 7.5 * (double) (i * one) - 2.5 * (double) (i * one);
        if (outd[i] != want) {
            printf("mul_f64[%d] = %g want %g\n", i, outd[i], want);
            ++fails;
        }
    }

    div_u32(out, src_f, (unsigned) N);
    for (i = 0; i < N; ++i) {
        float want = src_f[i] + (float) (unsigned) (i * one) / 4.0f;
        if (out[i] != want) {
            printf("div_u32[%d] = %g want %g\n", i, out[i], want);
            ++fails;
        }
    }

    byte_scale(out, bytes, N);
    for (i = 0; i < N; ++i) {
        float want = (float) bytes[i] * 3.0f + 5.0f * (float) (i * one);
        if (out[i] != want) {
            printf("byte_scale[%d] = %g want %g\n", i, out[i], want);
            ++fails;
        }
    }

    select_f32(out, src_f, N);
    for (i = 0; i < N; ++i) {
        float want = src_f[i] + ((i & 1) ? 3.0f * (float) (i * one) : 2.0f * (float) (i * one));
        if (out[i] != want) {
            printf("select_f32[%d] = %g want %g\n", i, out[i], want);
            ++fails;
        }
    }

    {
        int hits = cmp_f32(src_f, N), want = 0;
        for (i = 0; i < N; ++i)
            if (src_f[i] > 9.0f * (float) (i * one))
                want += i;
        if (hits != want) {
            printf("cmp_f32 = %d want %d\n", hits, want);
            ++fails;
        }
    }

    mul_ld(outd, src_d, N);
    for (i = 0; i < N; ++i) {
        double want =
            (double) ((long double) src_d[i] + (long double) 13 * (long double) (i * one));
        if (outd[i] != want) {
            printf("mul_ld[%d] = %g want %g\n", i, outd[i], want);
            ++fails;
        }
    }

    {
        float got = escape_f32(src_f, N);
        float want = src_f[N - 1] + 6.0f * (float) (N * one);
        if (got != want) {
            printf("escape_f32 = %g want %g\n", got, want);
            ++fails;
        }
    }

    /*
     * Feed `cold_exit_scale` exactly the values it expects, so a correct
     * compiler reports zero mismatches. A compiler that folds
     * `11 * (float) i` to zero reports N-1 of them (i == 0 agrees by luck).
     */
    {
        float ref[N];
        for (i = 0; i < N; ++i)
            ref[i] = 11.0f * (float) (i * one);
        if (cold_exit_scale(ref, N) != N) {
            printf("cold_exit_scale returned the wrong trip count\n");
            ++fails;
        }
        if (mismatches != 0) {
            printf("cold_exit_scale: %d mismatch(es), first i=%d got %g want %g\n", mismatches,
                   first_bad_i, first_bad_got, first_bad_want);
            ++fails;
        }
    }

    printf("iv_widen_float_conversion_closure: %s\n", fails ? "FAIL" : "OK");
    return fails != 0;
}
