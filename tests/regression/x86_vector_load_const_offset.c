/* A vector load at `pointer + constant` inside a packed operation reads at the
 * address C gives the expression.
 *
 * `_mm256_add_epi32(acc, _mm256_loadu_si256(p + 1))` is one instruction with a
 * memory operand: the lowering hands the operation the load's own address and
 * moves a constant offset into the displacement. It took that constant as
 * bytes whatever the pointer pointed to, so for a `const __m256i *p` the
 * instruction read at `1(%rdi)`, where `32(%rdi)` is the second object, and
 * for a `float *x` the load at `x + 8` read at byte 8. Every pointee but a
 * byte was wrong code at -O1 and above, with nothing said.
 *
 * Each function below is one way of writing the address: the pointee sizes
 * (1, 2, 4, 8, 16, 32 and a 24-byte struct), the constant on either side, a
 * negative one, sums of constants around a variable index, casts between
 * pointer types on either side of the sum (the type the sum is written in
 * decides, not the one the load takes), `void *` (GNU C: one byte), an array
 * that decays, and the sums that are not addresses yet (an integer converted
 * afterwards). The 128-bit forms and the SHA-style mixed-width path share the
 * function that peels the offset; the first are run here. GCC's output is the
 * reference.
 */
#include <immintrin.h>
#include <stdint.h>
#include <stdio.h>

#define NOINLINE __attribute__((noinline)) static

struct triple { long long a, b, c; };

/* ---- 256-bit integer add: the pointee decides the scale ---- */
NOINLINE __m256i at_vec256(const __m256i *p, __m256i acc)
{ return _mm256_add_epi32(acc, _mm256_loadu_si256(p + 1)); }
NOINLINE __m256i at_vec128(const __m128i *p, __m256i acc)
{ return _mm256_add_epi32(acc, _mm256_loadu_si256((const __m256i *)(p + 3))); }
NOINLINE __m256i at_bytes(const unsigned char *p, __m256i acc)
{ return _mm256_add_epi32(acc, _mm256_loadu_si256((const __m256i *)(p + 8))); }
NOINLINE __m256i at_shorts(const short *p, __m256i acc)
{ return _mm256_add_epi32(acc, _mm256_loadu_si256((const __m256i *)(p + 5))); }
NOINLINE __m256i at_ints(const int *p, __m256i acc)
{ return _mm256_add_epi32(acc, _mm256_loadu_si256((const __m256i *)(p + 3))); }
NOINLINE __m256i at_longs(const long long *p, __m256i acc)
{ return _mm256_add_epi32(acc, _mm256_loadu_si256((const __m256i *)(p + 2))); }
NOINLINE __m256i at_structs(const struct triple *p, __m256i acc)
{ return _mm256_add_epi32(acc, _mm256_loadu_si256((const __m256i *)(p + 1))); }
NOINLINE __m256i at_void(const void *p, __m256i acc)
{ return _mm256_add_epi32(acc, _mm256_loadu_si256((const __m256i *)(p + 12))); }

/* ---- where the constant stands ---- */
NOINLINE __m256i constant_first(const __m256i *p, __m256i acc)
{ return _mm256_add_epi32(acc, _mm256_loadu_si256(1 + p)); }
NOINLINE __m256i negative(const __m256i *p, __m256i acc)
{ return _mm256_add_epi32(acc, _mm256_loadu_si256(p - 1)); }
NOINLINE __m256i negative_constant(const int *p, __m256i acc)
{ return _mm256_add_epi32(acc, _mm256_loadu_si256((const __m256i *)(p + -4))); }
NOINLINE __m256i two_constants(const int *p, __m256i acc)
{ return _mm256_add_epi32(acc, _mm256_loadu_si256((const __m256i *)(p + 9 - 4))); }
NOINLINE __m256i around_an_index(const int *p, long i, __m256i acc)
{ return _mm256_add_epi32(acc, _mm256_loadu_si256((const __m256i *)(p + i + 2))); }
NOINLINE __m256i index_last(const int *p, long i, __m256i acc)
{ return _mm256_add_epi32(acc, _mm256_loadu_si256((const __m256i *)(p + 2 + i))); }
NOINLINE __m256i unsigned_count(const int *p, __m256i acc)
{ return _mm256_add_epi32(acc, _mm256_loadu_si256((const __m256i *)(p + 3u))); }
NOINLINE __m256i char_count(const int *p, __m256i acc)
{ return _mm256_add_epi32(acc, _mm256_loadu_si256((const __m256i *)(p + (signed char)-3))); }

/* ---- casts: each sum counts in the type it is written in ---- */
NOINLINE __m256i cast_then_sum(const int *p, __m256i acc)
{ return _mm256_add_epi32(acc, _mm256_loadu_si256((const __m256i *)p + 1)); }
NOINLINE __m256i sum_cast_sum(const int *p, __m256i acc)
{ return _mm256_add_epi32(acc, _mm256_loadu_si256((const __m256i *)((const short *)(p + 1) + 3) + 1)); }
NOINLINE __m256i through_void(const int *p, __m256i acc)
{ return _mm256_add_epi32(acc, _mm256_loadu_si256((const __m256i *)(const void *)(p + 4))); }
NOINLINE __m256i through_bytes(const __m256i *p, __m256i acc)
{ return _mm256_add_epi32(acc, _mm256_loadu_si256((const __m256i *)((const char *)p + 20))); }

/* ---- sums that are integers when they are made ---- */
NOINLINE __m256i integer_sum(uintptr_t address, __m256i acc)
{ return _mm256_add_epi32(acc, _mm256_loadu_si256((const __m256i *)(address + 16))); }
NOINLINE __m256i narrow_integer_sum(const int *base, unsigned offset, __m256i acc)
{ return _mm256_add_epi32(acc, _mm256_loadu_si256((const __m256i *)((uintptr_t)base + (offset + 8)))); }

/* ---- an array that decays, and a member ---- */
static __m256i table[4];
NOINLINE __m256i from_array(__m256i acc)
{ return _mm256_add_epi32(acc, _mm256_loadu_si256(table + 2)); }
struct holder { int skip; int values[24]; };
NOINLINE __m256i from_member(const struct holder *h, __m256i acc)
{ return _mm256_add_epi32(acc, _mm256_loadu_si256((const __m256i *)(h->values + 5))); }

/* ---- other operations and widths that take the same path ---- */
NOINLINE __m256 float_add(const float *x, __m256 acc)
{ return _mm256_add_ps(acc, _mm256_loadu_ps(x + 8)); }
NOINLINE __m256d double_mul(const double *x, __m256d acc)
{ return _mm256_mul_pd(acc, _mm256_loadu_pd(x + 4)); }
NOINLINE __m256i both_operands(const __m256i *p)
{ return _mm256_sub_epi32(_mm256_loadu_si256(p + 2), _mm256_loadu_si256(p + 1)); }
NOINLINE __m256i bitwise(const __m256i *p, __m256i acc)
{ return _mm256_xor_si256(acc, _mm256_loadu_si256(p + 3)); }
NOINLINE __m256i compare(const int *p, __m256i acc)
{ return _mm256_cmpgt_epi32(acc, _mm256_loadu_si256((const __m256i *)(p + 16))); }
NOINLINE __m128i narrow_add(const __m128i *p, __m128i acc)
{ return _mm_add_epi32(acc, _mm_loadu_si128(p + 1)); }
NOINLINE __m128i narrow_ints(const int *p, __m128i acc)
{ return _mm_add_epi32(acc, _mm_loadu_si128((const __m128i *)(p + 7))); }
NOINLINE __m128 narrow_float(const float *x, __m128 acc)
{ return _mm_add_ps(acc, _mm_loadu_ps(x + 4)); }
NOINLINE __m128d narrow_double(const double *x, __m128d acc)
{ return _mm_add_pd(acc, _mm_loadu_pd(x + 2)); }

/* ---- the shape real code has: an unrolled loop ---- */
NOINLINE int dot(const int *x, const int *y, long n)
{
    __m256i a = _mm256_setzero_si256(), b = _mm256_setzero_si256();
    for (long i = 0; i + 16 <= n; i += 16) {
        a = _mm256_add_epi32(a, _mm256_mullo_epi32(_mm256_loadu_si256((const __m256i *)(x + i)),
                                                    _mm256_loadu_si256((const __m256i *)(y + i))));
        b = _mm256_add_epi32(b, _mm256_mullo_epi32(_mm256_loadu_si256((const __m256i *)(x + i + 8)),
                                                    _mm256_loadu_si256((const __m256i *)(y + i + 8))));
    }
    a = _mm256_add_epi32(a, b);
    __m128i s = _mm_add_epi32(_mm256_castsi256_si128(a), _mm256_extracti128_si256(a, 1));
    s = _mm_add_epi32(s, _mm_shuffle_epi32(s, 0x4e));
    s = _mm_add_epi32(s, _mm_shuffle_epi32(s, 0xb1));
    return _mm_cvtsi128_si32(s);
}
NOINLINE float sum_unrolled(const float *x, long n)
{
    __m256 a = _mm256_setzero_ps(), b = _mm256_setzero_ps();
    for (long i = 0; i + 16 <= n; i += 16) {
        a = _mm256_add_ps(a, _mm256_loadu_ps(x + i));
        b = _mm256_add_ps(b, _mm256_loadu_ps(x + i + 8));
    }
    float out[8];
    _mm256_storeu_ps(out, _mm256_add_ps(a, b));
    return out[0] + out[1] + out[2] + out[3] + out[4] + out[5] + out[6] + out[7];
}

static int data[96];
static float fdata[64];
static double ddata[32];

static void show(const char *name, __m256i r)
{
    int o[8];
    _mm256_storeu_si256((__m256i *)o, r);
    printf("%-18s", name);
    for (int i = 0; i < 8; i++)
        printf(" %d", o[i]);
    printf("\n");
}

int main(void)
{
    for (int i = 0; i < 96; i++)
        data[i] = 1000 + 37 * i + (i * i) % 11;
    for (int i = 0; i < 64; i++)
        fdata[i] = (float)(i * 3 + 1) * 0.5f;
    for (int i = 0; i < 32; i++)
        ddata[i] = (double)(i * 7 + 2) * 0.25;
    for (int i = 0; i < 4; i++)
        table[i] = _mm256_set1_epi32(50000 + i);

    const int *mid = data + 32;
    __m256i acc = _mm256_set_epi32(8, 7, 6, 5, 4, 3, 2, 1);

    show("at_vec256", at_vec256((const __m256i *)mid, acc));
    show("at_vec128", at_vec128((const __m128i *)mid, acc));
    show("at_bytes", at_bytes((const unsigned char *)mid, acc));
    show("at_shorts", at_shorts((const short *)mid, acc));
    show("at_ints", at_ints(mid, acc));
    show("at_longs", at_longs((const long long *)mid, acc));
    show("at_structs", at_structs((const struct triple *)mid, acc));
    show("at_void", at_void(mid, acc));
    show("constant_first", constant_first((const __m256i *)mid, acc));
    show("negative", negative((const __m256i *)mid, acc));
    show("negative_constant", negative_constant(mid, acc));
    show("two_constants", two_constants(mid, acc));
    show("around_an_index", around_an_index(mid, 3, acc));
    show("around_an_index", around_an_index(mid, -9, acc));
    show("index_last", index_last(mid, 5, acc));
    show("unsigned_count", unsigned_count(mid, acc));
    show("char_count", char_count(mid, acc));
    show("cast_then_sum", cast_then_sum(mid, acc));
    show("sum_cast_sum", sum_cast_sum(mid, acc));
    show("through_void", through_void(mid, acc));
    show("through_bytes", through_bytes((const __m256i *)mid, acc));
    show("integer_sum", integer_sum((uintptr_t)mid, acc));
    show("narrow_integer_sum", narrow_integer_sum(mid, 4, acc));
    show("from_array", from_array(acc));
    show("from_member", from_member((const struct holder *)mid, acc));
    show("both_operands", both_operands((const __m256i *)mid));
    show("bitwise", bitwise((const __m256i *)mid, acc));
    show("compare", compare(mid, _mm256_set1_epi32(2800)));

    float f[8];
    _mm256_storeu_ps(f, float_add(fdata + 16, _mm256_set1_ps(0.25f)));
    printf("%-18s %g %g %g %g %g %g %g %g\n", "float_add", f[0], f[1], f[2], f[3], f[4], f[5], f[6], f[7]);
    double d[4];
    _mm256_storeu_pd(d, double_mul(ddata + 8, _mm256_set1_pd(2.0)));
    printf("%-18s %g %g %g %g\n", "double_mul", d[0], d[1], d[2], d[3]);

    int n[4];
    _mm_storeu_si128((__m128i *)n, narrow_add((const __m128i *)mid, _mm_set1_epi32(1)));
    printf("%-18s %d %d %d %d\n", "narrow_add", n[0], n[1], n[2], n[3]);
    _mm_storeu_si128((__m128i *)n, narrow_ints(mid, _mm_set1_epi32(1)));
    printf("%-18s %d %d %d %d\n", "narrow_ints", n[0], n[1], n[2], n[3]);
    _mm_storeu_ps(f, narrow_float(fdata + 16, _mm_set1_ps(0.25f)));
    printf("%-18s %g %g %g %g\n", "narrow_float", f[0], f[1], f[2], f[3]);
    _mm_storeu_pd(d, narrow_double(ddata + 8, _mm_set1_pd(0.5)));
    printf("%-18s %g %g\n", "narrow_double", d[0], d[1]);

    printf("%-18s %d\n", "dot", dot(data, data + 48, 48));
    printf("%-18s %g\n", "sum_unrolled", sum_unrolled(fdata, 64));
    return 0;
}
