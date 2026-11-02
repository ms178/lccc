/* A by-value aggregate argument is materialized at its call site, not before
 * the branch that guards the call.
 *
 *   seq_t const s = decode(i);            // scalar stores into a stack object
 *   n = exec(op, oend, s);                // struct passed by value
 *
 * `exec` is inlined, so its parameter is a private object written by a 24-byte
 * `Memcpy` from `s` and then read field by field.  The copy sits on the hot
 * path even though the call it exists for happens only on the last iteration.
 * `aggregate_sroa`'s form 5 proves `s` is dead at the copy, renames the copy's
 * destination onto `s`, and rebuilds a contiguous image only where the callee
 * actually needs one — so the fields stay in registers in the loop.
 *
 * The second loop forces the cold path on every iteration, which pins the
 * image's contents against the memcpy it replaced (a stale or partial image
 * changes `cold_sum`). */
#include <stdio.h>

typedef struct {
    long long a, b, c;
} seq_t;

static seq_t decode(long long i)
{
    seq_t s;
    s.a = i * 3;
    s.b = i * 5;
    s.c = i * 7;
    if (i & 1) s.a += 1000;
    if (i & 2) s.b -= 7;
    if (i & 4) s.c ^= 0xff;
    return s;
}

static long long __attribute__((noinline)) exec_cold(seq_t s, long long *calls)
{
    *calls += 1;
    return s.a + s.b * 3 + s.c * 5;
}

static long long exec(long long op, long long oend, seq_t s, long long *calls)
{
    if (op + s.a > oend)
        return exec_cold(s, calls);
    *calls += s.b;
    return s.c - s.a;
}

static long long loop(long long n, long long oend, long long *calls)
{
    long long acc = 0;
    for (long long i = 0; i < n; i++) {
        seq_t const s = decode(i);
        acc += exec(acc, oend, s, calls);
    }
    return acc;
}

int main(void)
{
    long long calls = 0;
    long long r1 = loop(200, 1000000, &calls);
    long long c1 = calls;
    printf("r1=%lld c1=%lld\n", r1, c1);

    /* Small budget: every iteration takes the call path. */
    long long r2 = loop(200, 0, &calls);
    long long c2 = calls - c1;
    printf("r2=%lld c2=%lld\n", r2, c2);

    return (r1 == -19240 && c1 == 98800 && r2 == 1158400 && c2 == 199) ? 0 : 1;
}
