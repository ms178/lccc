/* A copy of part of an aggregate writes that part and reads that part.
 *
 * Two forwards of aggregate_copy_forward took a partial `memcpy` for a whole
 * one (-O2, -O3, -Os; found through `_mm_storel_pi`, which stored 16 bytes
 * where it stores 8):
 *
 *  1. Construction forwarded into the destination. An inlined function that
 *     takes an aggregate by value and copies part of it out,
 *
 *         static inline void lo(char *p, struct S v) { memcpy(p, &v, 8); }
 *
 *     is `memcpy(tmp, arg, 16); memcpy(p, tmp, 8)`. The 16-byte fill of the
 *     temporary was redirected into `p`: the 8 bytes behind what the program
 *     wrote were overwritten.
 *
 *  2. Reads forwarded to the source. After `memcpy(&v, &s, 8)` a read of
 *     `v` behind the eight bytes was read from `s`: for a by-value parameter
 *     that is the caller's argument replaced by another object's field.
 *
 * Every function is noinline and every object is filled by a call, so that
 * nothing here is folded before the pass sees it. GCC is the oracle. */
#include <stdio.h>
#include <string.h>

struct S { long a, b; };
struct Q { int a, b, c, d; };
struct B { long a, b, c, d; };

__attribute__((noinline)) static void fill_s(struct S *s) { s->a = 0x1111111111111111L; s->b = 0x2222222222222222L; }
__attribute__((noinline)) static void fill_q(struct Q *q) { q->a = 1; q->b = 2; q->c = 3; q->d = 4; }
__attribute__((noinline)) static void fill_b(struct B *b) { b->a = 0x11; b->b = 0x22; b->c = 0x33; b->d = 0x44; }

/* --- 1: a partial copy out of a by-value parameter ---------------------- */

static char bytes[24];
static inline void store_low(char *p, struct S v) { memcpy(p, &v, 8); }
static inline void store_high(char *p, struct S v) { memcpy(p, (const char *)&v + 8, 8); }
static inline void store_middle(char *p, struct B v) { memcpy(p, (const char *)&v + 8, 16); }

__attribute__((noinline)) static void low_half(void) { struct S s; fill_s(&s); store_low(bytes, s); }
__attribute__((noinline)) static void high_half(void) { struct S s; fill_s(&s); store_high(bytes + 8, s); }
__attribute__((noinline)) static void middle(void) { struct B b; fill_b(&b); store_middle(bytes, b); }

static struct { int first; int keep[3]; } record = { 0, { 7, 8, 9 } };
static inline void store_first(int *p, struct Q v) { memcpy(p, &v, sizeof(int)); }
__attribute__((noinline)) static void first_field(void) { struct Q q; fill_q(&q); store_first(&record.first, q); }

/* The same through a pointer the function is given, and into a local. */
__attribute__((noinline)) static void low_half_to(char *p) { struct S s; fill_s(&s); store_low(p, s); }
__attribute__((noinline)) static long low_half_local(void)
{
    long pair[2] = { 5, 6 };
    struct S s;
    fill_s(&s);
    store_low((char *)pair, s);
    return pair[0] ^ pair[1];
}

static void show_bytes(const char *what)
{
    printf("%s:", what);
    for (unsigned i = 0; i < sizeof bytes; i++)
        printf("%s%02x", i % 8 ? "" : " ", (unsigned char)bytes[i]);
    puts("");
}

/* --- 2: a read behind a partial copy ------------------------------------ */

/* The parameter's home is written by the prologue: no instruction says so. */
__attribute__((noinline)) static long parameter_behind(struct B v)
{
    struct B s;
    fill_b(&s);
    memcpy(&v, &s, 8);
    return v.a * 1000 + v.b;
}
__attribute__((noinline)) static long parameter_far_behind(struct B v)
{
    struct B s;
    fill_b(&s);
    memcpy(&v, &s, 16);
    return v.b * 1000 + v.d;
}
/* A read that begins inside the copy and ends behind it. */
__attribute__((noinline)) static long parameter_straddling(struct B v)
{
    struct B s;
    long straddle;
    fill_b(&s);
    memcpy(&v, &s, 12);
    memcpy(&straddle, (const char *)&v + 8, 8);
    return straddle;
}
/* Inside the copy the read is the source's: this one may be forwarded. */
__attribute__((noinline)) static long parameter_inside(struct B v)
{
    struct B s;
    fill_b(&s);
    memcpy(&v, &s, 16);
    return v.a * 1000 + v.b;
}
/* A local with earlier contents, partly overwritten. */
__attribute__((noinline)) static long local_behind(void)
{
    struct B s, t;
    fill_b(&s);
    t.a = 5; t.b = 6; t.c = 7; t.d = 8;
    memcpy(&t, &s, 8);
    return t.a * 1000 + t.b;
}
static inline long tail_of(struct B v, const struct B *s) { memcpy(&v, s, 8); return v.a * 1000 + v.d; }
__attribute__((noinline)) static long inlined_behind(void)
{
    struct B s, t;
    fill_b(&s);
    t.a = 5; t.b = 6; t.c = 7; t.d = 8;
    return tail_of(t, &s);
}
/* A small object copied into the front of a larger one that is read whole. */
static inline void widen(struct B *out, struct S v) { struct B t = { 0, 0, 9, 9 }; memcpy(&t, &v, sizeof v); *out = t; }
__attribute__((noinline)) static long widened(void)
{
    struct S s;
    struct B o;
    fill_s(&s);
    widen(&o, s);
    return (o.a & 0xff) + (o.b & 0xff) * 100 + o.c * 10000 + o.d * 1000000;
}

int main(void)
{
    char pointed[16];
    struct B v = { 1, 2, 3, 4 };

    memset(bytes, 0x7f, sizeof bytes);
    low_half();
    show_bytes("low");
    memset(bytes, 0x7f, sizeof bytes);
    high_half();
    show_bytes("high");
    memset(bytes, 0x7f, sizeof bytes);
    middle();
    show_bytes("middle");
    first_field();
    printf("record: %d %d %d %d\n", record.first, record.keep[0], record.keep[1], record.keep[2]);
    memset(pointed, 0x3c, sizeof pointed);
    low_half_to(pointed);
    printf("pointed: %02x %02x\n", (unsigned char)pointed[7], (unsigned char)pointed[8]);
    printf("local: %lx\n", (unsigned long)low_half_local());

    printf("behind: %ld %ld %lx %ld\n", parameter_behind(v), parameter_far_behind(v),
           (unsigned long)parameter_straddling(v), parameter_inside(v));
    printf("locals: %ld %ld %ld\n", local_behind(), inlined_behind(), widened());
    return 0;
}
