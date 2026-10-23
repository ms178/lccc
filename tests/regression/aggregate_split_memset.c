/*
 * aggregate_sroa form 4, rule 5b (whole-object memset expansion), and the
 * inliner's SROA-unlock admission of plain-static helpers.
 *
 * The bitstream case is zstd's HUF_CStream_t shape: a local struct zeroed by
 * `memset(&s, 0, sizeof s)` inside a plain `static` init helper, driven in a
 * loop, then finished by a plain `static` close helper. Promoting it needs
 * both halves of the change: the inliner must clone the helpers that take
 * `&s` (they are not declared inline), and the splitter must turn the memset
 * into per-field stores instead of treating it as an escape.
 *
 * Every other case computes a result twice, once through an aggregate and
 * once through plain scalars, and both must agree. The "must refuse" shapes
 * (partial memset, interior memset, used result, nonzero bytes into a double)
 * are exercised so the refusals are decided by the analysis, not by luck. A
 * memset followed by a whole-struct copy-out checks that the gap carriers
 * receive the memset bytes.
 *
 * exit 0 = pass (the runner's convention for the C tests in tests/regression).
 */
#include <stdio.h>
#include <string.h>

typedef unsigned char u8;
typedef unsigned short u16;
typedef unsigned int u32;
typedef unsigned long long u64;

static int failures;

static void check(const char *name, u64 got, u64 want)
{
    if (got != want) {
        printf("FAIL %s: got %llu want %llu\n", name, got, want);
        failures++;
    }
}

/* ---- HUF_CStream_t shape ------------------------------------------------ */

typedef struct {
    u64 container[2];
    u64 pos[2];
    u8 *start;
    u8 *ptr;
    u8 *end;
} stream_t;

static int stream_init(stream_t *s, u8 *dst, u64 cap)
{
    memset(s, 0, sizeof(*s));
    s->start = dst;
    s->ptr = dst;
    s->end = dst + cap - sizeof(s->container[0]);
    return cap <= sizeof(s->container[0]);
}

static inline __attribute__((always_inline)) void stream_add(stream_t *s, u32 bits, u32 n, int idx)
{
    s->container[idx] = (s->container[idx] << n) | (bits & ((1u << n) - 1));
    s->pos[idx] += n;
}

static inline __attribute__((always_inline)) void stream_flush(stream_t *s)
{
    u64 bytes = s->pos[0] >> 3;
    u64 v = s->container[0] << (64 - s->pos[0]);
    memcpy(s->ptr, &v, sizeof(v));
    s->ptr += bytes;
    if (s->ptr > s->end)
        s->ptr = s->end;
    s->pos[0] &= 7;
}

static u64 stream_close(stream_t *s)
{
    stream_add(s, 1, 1, 0);
    stream_flush(s);
    if (s->ptr >= s->end)
        return 0;
    return (u64)(s->ptr - s->start) + (s->pos[0] > 0);
}

static u64 encode(const u8 *src, u32 n, u8 *dst, u64 cap)
{
    stream_t s;
    if (stream_init(&s, dst, cap))
        return 0;
    for (u32 i = 0; i < n; i++) {
        stream_add(&s, src[i], 5 + (src[i] & 3), 0);
        stream_add(&s, src[i] >> 2, 3, 1);
        s.container[0] ^= s.container[1] & 0xff;
        s.pos[1] = 0;
        if ((i & 3) == 3)
            stream_flush(&s);
    }
    return stream_close(&s);
}

/* The same computation on plain scalars (no aggregate to promote). */
static u64 encode_scalar(const u8 *src, u32 n, u8 *dst, u64 cap)
{
    u64 c0 = 0, c1 = 0, p0 = 0;
    u8 *start = dst, *ptr = dst, *end = dst + cap - 8;
    if (cap <= 8)
        return 0;
    for (u32 i = 0; i < n; i++) {
        u32 n0 = 5 + (src[i] & 3);
        c0 = (c0 << n0) | (src[i] & ((1u << n0) - 1));
        p0 += n0;
        c1 = (c1 << 3) | ((u32)(src[i] >> 2) & 7);
        c0 ^= c1 & 0xff;
        if ((i & 3) == 3) {
            u64 v = c0 << (64 - p0);
            memcpy(ptr, &v, 8);
            ptr += p0 >> 3;
            if (ptr > end)
                ptr = end;
            p0 &= 7;
        }
    }
    c0 = (c0 << 1) | 1;
    p0 += 1;
    {
        u64 v = c0 << (64 - p0);
        memcpy(ptr, &v, 8);
        ptr += p0 >> 3;
        if (ptr > end)
            ptr = end;
        p0 &= 7;
    }
    if (ptr >= end)
        return 0;
    return (u64)(ptr - start) + (p0 > 0);
}

static u64 fnv(const u8 *p, u64 n)
{
    u64 h = 1469598103934665603ull;
    for (u64 i = 0; i < n; i++)
        h = (h ^ p[i]) * 1099511628211ull;
    return h;
}

/* ---- nonzero splat into mixed field widths ------------------------------ */

typedef struct {
    u8 c;
    u16 h;
    u32 w;
    const void *p;
} mixed_t;

static __attribute__((noinline)) u64 splat_case(int byte, u32 k)
{
    mixed_t m;
    memset(&m, byte, sizeof(m));
    m.w += k;
    return (u64)m.c + ((u64)m.h << 8) + ((u64)m.w << 24) + (u64)(unsigned long)m.p;
}

static u64 splat_scalar(int byte, u32 k)
{
    u8 b = (u8)byte;
    u64 c = b;
    u64 h = (u16)(b * 0x0101u);
    u64 w = (u32)(b * 0x01010101u) + k;
    w &= 0xffffffffu;
    unsigned long p = (unsigned long)(b * 0x0101010101010101ull);
    return c + (h << 8) + (w << 24) + (u64)p;
}

/* ---- shapes the rule must refuse ---------------------------------------- */

typedef struct {
    u64 a, b;
} pair_t;

static __attribute__((noinline)) u64 partial_memset(u64 x)
{
    pair_t q;
    q.a = x;
    q.b = x * 3;
    memset(&q, 0, sizeof(q.a)); /* zeroes a only */
    return q.a * 7 + q.b;
}

static __attribute__((noinline)) u64 interior_memset(u64 x)
{
    pair_t q;
    q.a = x;
    q.b = x * 3;
    memset(&q.b, 0xff, sizeof(q.b)); /* starts inside the object */
    return q.a + (q.b >> 60);
}

static __attribute__((noinline)) u64 used_result(u64 x)
{
    pair_t q;
    pair_t *r = memset(&q, 0, sizeof(q));
    r->b = x;
    return q.a + q.b * 5;
}

typedef struct {
    double d;
    u64 n;
} dn_t;

static __attribute__((noinline)) u64 float_nonzero(void)
{
    dn_t v;
    memset(&v, 0x40, sizeof(v)); /* d = bits 0x4040404040404040 = 32.50196... */
    v.n += 1;
    return (u64)(v.d * 1e6) ^ v.n; /* a double-typed field: rule 5b refuses */
}

static u64 float_nonzero_scalar(void)
{
    u64 bits = 0x4040404040404040ull;
    double d;
    memcpy(&d, &bits, sizeof(d));
    return (u64)(d * 1e6) ^ (bits + 1);
}

static __attribute__((noinline)) double float_zero(double k)
{
    dn_t v;
    memset(&v, 0, sizeof(v));
    v.d += k;
    v.n += 2;
    return v.d * (double)v.n;
}

/* ---- memset then whole-struct copy-out (gap carriers) ------------------- */

typedef struct {
    u32 x;  /* the only field the body touches */
    u32 gap;
    u64 tail;
} out_t;

static __attribute__((noinline)) void copy_out(out_t *dst, u32 k)
{
    out_t t;
    memset(&t, 0, sizeof(t));
    t.x = t.x + k;
    *dst = t;
}

int main(void)
{
    u8 src[257];
    for (int i = 0; i < 257; i++)
        src[i] = (u8)(i * 73 + 11);

    for (u32 n = 0; n < 257; n += 37) {
        u8 a[512], b[512];
        memset(a, 0xcc, sizeof(a));
        memset(b, 0xcc, sizeof(b));
        u64 ra = encode(src, n, a, sizeof(a));
        u64 rb = encode_scalar(src, n, b, sizeof(b));
        check("encode.size", ra, rb);
        check("encode.bytes", fnv(a, sizeof(a)), fnv(b, sizeof(b)));
    }
    check("encode.tiny", encode(src, 3, (u8[8]){0}, 8), 0);

    for (int byte = -1; byte <= 0x1ff; byte += 0x55)
        check("splat", splat_case(byte, 1000u + (u32)byte), splat_scalar(byte, 1000u + (u32)byte));

    check("partial", partial_memset(5), 15);
    check("interior", interior_memset(9), 9 + 15);
    check("used_result", used_result(4), 20);
    check("float_nonzero", float_nonzero(), float_nonzero_scalar());
    check("float_zero", (u64)float_zero(2.5), 5);

    out_t o;
    memset(&o, 0x5a, sizeof(o));
    copy_out(&o, 77);
    check("copy_out.x", o.x, 77);
    check("copy_out.gap", o.gap, 0);
    check("copy_out.tail", o.tail, 0);

    printf("%s\n", failures ? "FAILED" : "ok");
    return failures != 0;
}
