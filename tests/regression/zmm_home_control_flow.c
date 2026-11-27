/* 512-bit vector objects that live in registers: control flow and the
 * uses that keep an object in memory.
 *
 * zmm_home_objects.c asks what a home must hold; this file asks where. A
 * register is given to an object from the liveness of the function, so
 * each function here has flow that a liveness can get wrong: a computed
 * goto whose targets are a table, a switch that falls through, a loop
 * with two entries, a value that is written on one path only, an early
 * return out of two nested loops, a declaration a `goto` jumps back over.
 * The second half mixes homes with objects that cannot have one: a member
 * of a structure, an element of an array, a pointer that is chosen at run
 * time, a lane that is read by subscript.
 *
 * Every function has a model beside it that computes the same lanes one
 * `unsigned` at a time.
 */
#include <immintrin.h>
#include <stdio.h>
#include <string.h>

#define NOINLINE __attribute__((noinline))

typedef unsigned u32;

static int failures;

static void expect(const char *what, const void *got, const void *want, size_t bytes)
{
    if (memcmp(got, want, bytes) != 0) {
        printf("FAIL %s\n", what);
        failures++;
    }
}

static __m512i load(const int *p) { return _mm512_loadu_si512((const void *)p); }
static void store(int *p, __m512i v) { _mm512_storeu_si512((void *)p, v); }

/* A computed goto: every label of the table follows the jump. */
NOINLINE static void computed(int *out, const int *p, int n, int which)
{
    static void *const table[] = {&&add, &&sub, &&mul, &&done};
    __m512i acc = _mm512_set1_epi32(1);
    __m512i three = _mm512_set1_epi32(3);
    __m512i v;
    int i = 0;
next:
    if (i + 16 > n)
        goto *table[3];
    v = load(p + i);
    i += 16;
    goto *table[(which + i / 16) % 3];
add:
    acc = _mm512_add_epi32(acc, v);
    goto next;
sub:
    acc = _mm512_sub_epi32(acc, _mm512_xor_si512(v, three));
    goto next;
mul:
    acc = _mm512_mullo_epi32(acc, _mm512_or_si512(v, three));
    goto next;
done:
    store(out, acc);
}

static void computed_model(int *out, const int *p, int n, int which)
{
    for (int lane = 0; lane < 16; lane++) {
        u32 acc = 1;
        for (int i = 0; i + 16 <= n;) {
            u32 v = (u32)p[i + lane];
            i += 16;
            switch ((which + i / 16) % 3) {
            case 0: acc += v; break;
            case 1: acc -= v ^ 3u; break;
            default: acc *= v | 3u; break;
            }
        }
        out[lane] = (int)acc;
    }
}

/* A switch that falls through, and an arm that rotates three names. */
NOINLINE static void switched(int *out, const int *p, int n)
{
    __m512i a = _mm512_setzero_si512(), b = _mm512_set1_epi32(7), c = _mm512_set1_epi32(-2);
    for (int i = 0; i + 16 <= n; i += 16) {
        __m512i v = load(p + i);
        switch ((i / 16) & 3) {
        case 0:
            a = _mm512_add_epi32(a, v);
            break;
        case 1:
            b = _mm512_xor_si512(b, v);
            /* fall through */
        case 2:
            c = _mm512_sub_epi32(c, b);
            break;
        default: {
            __m512i t = a;
            a = b;
            b = c;
            c = t;
        }
        }
    }
    store(out, a);
    store(out + 16, b);
    store(out + 32, c);
}

static void switched_model(int *out, const int *p, int n)
{
    for (int lane = 0; lane < 16; lane++) {
        u32 a = 0, b = 7, c = (u32)-2;
        for (int i = 0; i + 16 <= n; i += 16) {
            u32 v = (u32)p[i + lane];
            switch ((i / 16) & 3) {
            case 0:
                a += v;
                break;
            case 1:
                b ^= v;
                /* fall through */
            case 2:
                c -= b;
                break;
            default: {
                u32 t = a;
                a = b;
                b = c;
                c = t;
            }
            }
        }
        out[lane] = (int)a;
        out[16 + lane] = (int)b;
        out[32 + lane] = (int)c;
    }
}

/* A loop with two entries. */
NOINLINE static void two_entries(int *out, const int *p, int n, int enter)
{
    __m512i a = _mm512_set1_epi32(5), b = _mm512_set1_epi32(9);
    __m512i one = _mm512_set1_epi32(1);
    int i = 0;
    if (enter)
        goto second;
first:
    a = _mm512_add_epi32(a, load(p + (i & 16)));
    if (++i >= n)
        goto out;
second:
    b = _mm512_mullo_epi32(b, _mm512_or_si512(a, one));
    if (++i < n)
        goto first;
out:
    store(out, a);
    store(out + 16, b);
}

static void two_entries_model(int *out, const int *p, int n, int enter)
{
    for (int lane = 0; lane < 16; lane++) {
        u32 a = 5, b = 9;
        int i = 0;
        if (enter)
            goto second;
    first:
        a += (u32)p[(i & 16) + lane];
        if (++i >= n)
            goto out;
    second:
        b *= a | 1u;
        if (++i < n)
            goto first;
    out:
        out[lane] = (int)a;
        out[16 + lane] = (int)b;
    }
}

/* Written on one path only, and read on that path only. */
NOINLINE static void one_path(int *out, const int *p, int flag)
{
    __m512i v = _mm512_setzero_si512();
    __m512i w = _mm512_set1_epi32(11);
    if (flag)
        v = load(p);
    w = _mm512_add_epi32(w, w);
    if (flag)
        w = _mm512_sub_epi32(w, v);
    store(out, w);
}

static void one_path_model(int *out, const int *p, int flag)
{
    for (int lane = 0; lane < 16; lane++)
        out[lane] = (int)(22u - (flag ? (u32)p[lane] : 0u));
}

/* Two names that trade places through a third, round after round. */
NOINLINE static void trade(int *out, const int *p, int n)
{
    __m512i a = load(p), b = load(p + 16);
    for (int i = 0; i < n; i++) {
        __m512i t = a;
        a = b;
        b = _mm512_add_epi32(t, _mm512_set1_epi32(i));
    }
    store(out, a);
    store(out + 16, b);
}

static void trade_model(int *out, const int *p, int n)
{
    for (int lane = 0; lane < 16; lane++) {
        u32 a = (u32)p[lane], b = (u32)p[16 + lane];
        for (int i = 0; i < n; i++) {
            u32 t = a;
            a = b;
            b = t + (u32)i;
        }
        out[lane] = (int)a;
        out[16 + lane] = (int)b;
    }
}

/* One object as every operand of an operation, and as both ends of a copy. */
NOINLINE static void itself(int *out, const int *p)
{
    __m512i a = load(p);
    __m512i *same = &a;
    a = *same;
    a = _mm512_add_epi32(a, a);
    a = _mm512_ternarylogic_epi32(a, a, a, 0x1e);
    __m512i b = a;
    b = _mm512_mullo_epi32(b, b);
    a = _mm512_sub_epi32(b, a);
    store(out, a);
}

static void itself_model(int *out, const int *p)
{
    for (int lane = 0; lane < 16; lane++) {
        u32 a = (u32)p[lane];
        a += a;
        /* 0x1e is a ^ (b | c); with all three the same that is 0. */
        a = a ^ (a | a);
        u32 b = a * a;
        out[lane] = (int)(b - a);
    }
}

/* An early return out of two loops, with different values alive at each. */
NOINLINE static int early(int *out, const int *p, int rows, int stop)
{
    __m512i total = _mm512_setzero_si512();
    for (int r = 0; r < rows; r++) {
        __m512i row = _mm512_set1_epi32(r);
        for (int k = 0; k < 4; k++) {
            row = _mm512_add_epi32(row, load(p + 16 * k));
            if (r * 4 + k == stop) {
                store(out, _mm512_xor_si512(row, total));
                return 1;
            }
        }
        total = _mm512_sub_epi32(total, row);
    }
    store(out, total);
    return 0;
}

static int early_model(int *out, const int *p, int rows, int stop)
{
    int left = 0;
    for (int lane = 0; lane < 16; lane++) {
        u32 total = 0;
        left = 0;
        for (int r = 0; r < rows && !left; r++) {
            u32 row = (u32)r;
            for (int k = 0; k < 4; k++) {
                row += (u32)p[16 * k + lane];
                if (r * 4 + k == stop) {
                    out[lane] = (int)(row ^ total);
                    left = 1;
                    break;
                }
            }
            if (!left)
                total -= row;
        }
        if (!left)
            out[lane] = (int)total;
    }
    return left;
}

/* A `goto` back over a declaration: the scope is entered again. */
NOINLINE static void again(int *out, const int *p, int rounds)
{
    __m512i acc = load(p);
    int round = 0;
back:;
    __m512i fresh = _mm512_set1_epi32(round + 1);
    fresh = _mm512_mullo_epi32(fresh, acc);
    acc = _mm512_add_epi32(acc, fresh);
    if (++round < rounds)
        goto back;
    store(out, acc);
}

static void again_model(int *out, const int *p, int rounds)
{
    for (int lane = 0; lane < 16; lane++) {
        u32 acc = (u32)p[lane];
        int round = 0;
        do {
            acc += (u32)(round + 1) * acc;
        } while (++round < rounds);
        out[lane] = (int)acc;
    }
}

/* Forty objects one after another, never more than three alive. */
NOINLINE static void parade(int *out, const int *p)
{
    __m512i acc = load(p);
#define STEP(n)                                                   \
    {                                                             \
        __m512i t##n = _mm512_add_epi32(acc, _mm512_set1_epi32(n)); \
        __m512i u##n = _mm512_xor_si512(t##n, acc);               \
        acc = _mm512_sub_epi32(u##n, t##n);                       \
    }
    STEP(1) STEP(2) STEP(3) STEP(4) STEP(5) STEP(6) STEP(7) STEP(8) STEP(9) STEP(10)
    STEP(11) STEP(12) STEP(13) STEP(14) STEP(15) STEP(16) STEP(17) STEP(18) STEP(19) STEP(20)
#undef STEP
    store(out, acc);
}

static void parade_model(int *out, const int *p)
{
    for (int lane = 0; lane < 16; lane++) {
        u32 acc = (u32)p[lane];
        for (u32 n = 1; n <= 20; n++) {
            u32 t = acc + n;
            u32 u = t ^ acc;
            acc = u - t;
        }
        out[lane] = (int)acc;
    }
}

/* A mask compare decides a branch between two vector operations. */
NOINLINE static void decide(int *out, const int *p, const int *q)
{
    __m512i a = load(p), b = load(q);
    __m512i r = _mm512_add_epi32(a, b);
    if (_mm512_cmpeq_epi32_mask(a, b) != 0 && _mm512_cmplt_epi32_mask(a, r) != 0xffff)
        r = _mm512_sub_epi32(r, a);
    else
        r = _mm512_xor_si512(r, b);
    store(out, r);
}

static void decide_model(int *out, const int *p, const int *q)
{
    int any_equal = 0, all_less = 1;
    for (int lane = 0; lane < 16; lane++) {
        any_equal |= p[lane] == q[lane];
        all_less &= p[lane] < (int)((u32)p[lane] + (u32)q[lane]);
    }
    for (int lane = 0; lane < 16; lane++) {
        u32 a = (u32)p[lane], b = (u32)q[lane], r = a + b;
        out[lane] = (int)(any_equal && !all_less ? r - a : r ^ b);
    }
}

/* The object is chosen at run time: both are named by a pointer. */
NOINLINE static void chosen(int *out, const int *p, const int *q, int k)
{
    __m512i a = load(p), b = load(q);
    a = _mm512_add_epi32(a, b);
    __m512i r = k ? a : b;
    __m512i *which = k & 2 ? &a : &b;
    *which = _mm512_sub_epi32(*which, r);
    store(out, a);
    store(out + 16, b);
}

static void chosen_model(int *out, const int *p, const int *q, int k)
{
    for (int lane = 0; lane < 16; lane++) {
        u32 a = (u32)p[lane], b = (u32)q[lane];
        a += b;
        u32 r = k ? a : b;
        if (k & 2)
            a -= r;
        else
            b -= r;
        out[lane] = (int)a;
        out[16 + lane] = (int)b;
    }
}

/* A member of a structure and an element of an array are memory; what is
 * copied into and out of them may be a register. */
struct holder {
    int before;
    __m512i value;
    int after;
};

NOINLINE static void members(int *out, const int *p, const int *q, int index)
{
    struct holder h = {.before = 1, .after = 2};
    __m512i table[4];
    __m512i a = load(p), b = load(q);
    for (int i = 0; i < 4; i++)
        table[i] = _mm512_add_epi32(a, _mm512_set1_epi32(i));
    h.value = _mm512_xor_si512(a, b);
    a = table[index & 3];
    b = _mm512_sub_epi32(h.value, a);
    h.value = b;
    table[(index + 1) & 3] = h.value;
    a = _mm512_add_epi32(table[(index + 1) & 3], table[(index + 2) & 3]);
    store(out, a);
    store(out + 16, h.value);
    out[32] = h.before + h.after;
}

static void members_model(int *out, const int *p, const int *q, int index)
{
    for (int lane = 0; lane < 16; lane++) {
        u32 a = (u32)p[lane], b = (u32)q[lane], table[4];
        for (u32 i = 0; i < 4; i++)
            table[i] = a + i;
        u32 value = a ^ b;
        a = table[index & 3];
        b = value - a;
        value = b;
        table[(index + 1) & 3] = value;
        a = table[(index + 1) & 3] + table[(index + 2) & 3];
        out[lane] = (int)a;
        out[16 + lane] = (int)value;
    }
    out[32] = 3;
}

/* The result is stored over an operand's memory. */
NOINLINE static void overlap(int *inout, const int *q)
{
    __m512i a = load(inout);
    __m512i b = load(q);
    store(inout, _mm512_add_epi32(a, b));
    __m512i c = load(inout);
    store(inout + 8, _mm512_sub_epi32(c, a));
    __m512i d = load(inout);
    store(inout + 24, _mm512_xor_si512(d, c));
}

static void overlap_model(int *inout, const int *q)
{
    u32 a[16], c[16], d[16];
    for (int lane = 0; lane < 16; lane++) {
        a[lane] = (u32)inout[lane];
        inout[lane] = (int)(a[lane] + (u32)q[lane]);
    }
    for (int lane = 0; lane < 16; lane++)
        c[lane] = (u32)inout[lane];
    for (int lane = 0; lane < 16; lane++)
        inout[8 + lane] = (int)(c[lane] - a[lane]);
    for (int lane = 0; lane < 16; lane++)
        d[lane] = (u32)inout[lane];
    for (int lane = 0; lane < 16; lane++)
        inout[24 + lane] = (int)(d[lane] ^ c[lane]);
}

/* The vector extensions: operators and a lane read by subscript.  The
 * lanes are added as unsigned ones: a sum of `long long` may not wrap. */
typedef unsigned long long lanes __attribute__((vector_size(64)));

NOINLINE static long long extensions(long long *out, const long long *p, const long long *q, int lane)
{
    __m512i a = _mm512_loadu_si512((const void *)p), b = _mm512_loadu_si512((const void *)q);
    __m512i sum = (__m512i)((lanes)a + (lanes)b);
    __m512i mixed = (__m512i)((lanes)_mm512_xor_si512(sum, a) - (lanes)b);
    mixed[1] = sum[2];
    _mm512_storeu_si512((void *)out, _mm512_add_epi64(mixed, sum));
    return (long long)((unsigned long long)mixed[lane & 7] + (unsigned long long)sum[(lane + 3) & 7]);
}

static long long extensions_model(long long *out, const long long *p, const long long *q, int lane)
{
    unsigned long long sum[8], mixed[8];
    for (int i = 0; i < 8; i++) {
        sum[i] = (unsigned long long)p[i] + (unsigned long long)q[i];
        mixed[i] = (sum[i] ^ (unsigned long long)p[i]) - (unsigned long long)q[i];
    }
    mixed[1] = sum[2];
    for (int i = 0; i < 8; i++)
        out[i] = (long long)(mixed[i] + sum[i]);
    return (long long)(mixed[lane & 7] + sum[(lane + 3) & 7]);
}

int main(void)
{
    if (!__builtin_cpu_supports("avx512f") || !__builtin_cpu_supports("avx512bw")
        || !__builtin_cpu_supports("avx512dq")) {
        puts("zmm homes, control flow: no AVX-512 F, BW and DQ on this machine");
        return 0;
    }

    int x[80], y[80], got[64], want[64];
    for (int i = 0; i < 80; i++) {
        x[i] = 1000003 * i + 17;
        y[i] = i % 5 == 0 ? x[i] : 77 - 40503 * i * i;
    }

    for (int n = 0; n <= 80; n += 8)
        for (int which = 0; which < 3; which++) {
            computed(got, x, n, which);
            computed_model(want, x, n, which);
            expect("computed", got, want, 64);
        }

    for (int n = 0; n <= 80; n += 16) {
        switched(got, y, n);
        switched_model(want, y, n);
        expect("switched", got, want, 192);
    }

    for (int n = 1; n < 9; n++)
        for (int enter = 0; enter < 2; enter++) {
            two_entries(got, x, n, enter);
            two_entries_model(want, x, n, enter);
            expect("two_entries", got, want, 128);
        }

    for (int flag = 0; flag < 2; flag++) {
        one_path(got, y, flag);
        one_path_model(want, y, flag);
        expect("one_path", got, want, 64);
    }

    for (int n = 0; n < 6; n++) {
        trade(got, x, n);
        trade_model(want, x, n);
        expect("trade", got, want, 128);
    }

    itself(got, x);
    itself_model(want, x);
    expect("itself", got, want, 64);

    for (int stop = -1; stop < 14; stop++) {
        int left = early(got, y, 3, stop);
        int left_model = early_model(want, y, 3, stop);
        expect("early", got, want, 64);
        if (left != left_model) {
            printf("FAIL early: returned %d\n", left);
            failures++;
        }
    }

    for (int rounds = 1; rounds < 5; rounds++) {
        again(got, x, rounds);
        again_model(want, x, rounds);
        expect("again", got, want, 64);
    }

    parade(got, y);
    parade_model(want, y);
    expect("parade", got, want, 64);

    decide(got, x, y);
    decide_model(want, x, y);
    expect("decide: some lanes equal", got, want, 64);
    decide(got, x + 1, y + 2);
    decide_model(want, x + 1, y + 2);
    expect("decide: no lane equal", got, want, 64);

    for (int k = 0; k < 4; k++) {
        chosen(got, x, y, k);
        chosen_model(want, x, y, k);
        expect("chosen", got, want, 128);
    }

    for (int index = 0; index < 4; index++) {
        memset(got, 0, sizeof got);
        memset(want, 0, sizeof want);
        members(got, x, y, index);
        members_model(want, x, y, index);
        expect("members", got, want, 33 * sizeof(int));
    }

    {
        int a[40], b[40];
        memcpy(a, x, sizeof a);
        memcpy(b, x, sizeof b);
        overlap(a, y);
        overlap_model(b, y);
        expect("overlap", a, b, sizeof a);
    }

    {
        long long p[8], q[8], vgot[8], vwant[8];
        for (int i = 0; i < 8; i++) {
            p[i] = 0x0123456789abcdefLL * (i + 1);
            q[i] = (long long)(0xeeeeeeeeddddddddULL * (unsigned long long)(i + 3));
        }
        for (int lane = 0; lane < 8; lane++) {
            long long r = extensions(vgot, p, q, lane);
            long long r_model = extensions_model(vwant, p, q, lane);
            expect("extensions", vgot, vwant, 64);
            if (r != r_model) {
                printf("FAIL extensions: lane %d\n", lane);
                failures++;
            }
        }
    }

    puts(failures ? "zmm homes, control flow: FAILED" : "zmm homes, control flow: ok");
    return failures != 0;
}
