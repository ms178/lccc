/* Memory and calls: struct copies, memcpy/memset shapes, recursion that
 * forces a real frame, and varargs.  These exercise the calling-convention
 * and stack-slot paths, which a pure-arithmetic program never reaches. */
#include <stdio.h>
#include <stdint.h>
#include <stdarg.h>
#include <string.h>

typedef struct { int32_t a, b; double c; } Pair;
typedef struct { char pad[61]; int32_t v; } Odd;      /* forces alignment */
typedef struct { uint64_t w[8]; } Wide;

static Pair mk(int32_t a, int32_t b) { Pair p; p.a = a; p.b = b; p.c = (double)a * b; return p; }
static int32_t use(Pair p) { return p.a ^ p.b ^ (int32_t)p.c; }
static Wide wide_id(Wide w) { return w; }              /* passed in memory */

static int fib(int n) { return n < 2 ? n : fib(n - 1) + fib(n - 2); }
static int ack(int m, int n) {
    if (m == 0) return n + 1;
    if (n == 0) return ack(m - 1, 1);
    return ack(m - 1, ack(m, n - 1));
}
static uint32_t vsum(int n, ...) {                    /* varargs */
    va_list ap; uint32_t s = 0;
    va_start(ap, n);
    for (int i = 0; i < n; i++) s += va_arg(ap, uint32_t);
    va_end(ap);
    return s;
}

int main(void) {
    uint64_t h = 0;
    for (int i = 0; i < 512; i++) {
        Pair p = mk(i, i * 3);
        h += (uint32_t)use(p);
        h ^= p.c > 0 ? 1u : 2u;
    }

    unsigned char buf[256];
    for (int i = 0; i < 256; i++) buf[i] = (unsigned char)(i * 7 + 1);
    unsigned char cp[256];
    memcpy(cp, buf, sizeof cp);
    for (int i = 0; i < 256; i += 17) memset(cp + i, (unsigned char)i, 9);
    for (int i = 0; i < 256; i++) h = h * 131u + cp[i];

    Wide w;
    for (int i = 0; i < 8; i++) w.w[i] = (uint64_t)i * 0x0123456789ABCDEFull;
    Wide v = wide_id(w);
    for (int i = 0; i < 8; i++) h ^= v.w[i] * (uint64_t)(i + 1);

    Odd o; memset(&o, 0, sizeof o); o.v = 0x5A5A5A5A;
    h += (uint32_t)(o.v ^ (uint32_t)((unsigned char *)&o.v - (unsigned char *)&o));

    printf("memory %016llx\n", (unsigned long long)h);
    printf("memory fib %d ack %d\n", fib(20), ack(2, 3));
    printf("memory va %u\n", vsum(8, 1u, 2u, 3u, 4u, 5u, 6u, 7u, 8u));
    return 0;
}
