/* _mm_prefetch() and _m_prefetchw(): hints, and nothing but hints.
 *
 * _mm_prefetch was `((void)(P), (void)(I))` in this compiler's <xmmintrin.h>
 * and compiled to nothing; it is the instruction now (which one, for which
 * target, is scripts/check_x86_prefetch.py's question: a prefetch has no
 * value for a test like this one to compare). What this case holds is what
 * a program can observe around the hint:
 *
 *   - the address expression is evaluated exactly once, and before the
 *     statement that follows;
 *   - an address that is never dereferenced (null, past the end, unmapped)
 *     does not fault: a prefetch is not a load;
 *   - the selectors are GCC's `enum _mm_hint`, value for value (that each
 *     is visible to `#ifdef` as well, as in the headers where they are
 *     macros, is not GCC's and is held by the script);
 *   - no value of the program changes.
 */
#include <immintrin.h>
#include <stdio.h>
#include <stdlib.h>

static int calls;

static const char *next_line(const char *p)
{
    calls++;
    return p + 64;
}

int main(void)
{
    int n = 1 << 14;
    unsigned char *a = malloc((size_t)n);
    if (!a)
        return 2;
    for (int i = 0; i < n; i++)
        a[i] = (unsigned char)(i * 131 + 7);

    printf("T0=%d T1=%d T2=%d NTA=%d ET0=%d RST2=%d IT0=%d IT1=%d\n", (int)_MM_HINT_T0,
           (int)_MM_HINT_T1, (int)_MM_HINT_T2, (int)_MM_HINT_NTA, (int)_MM_HINT_ET0,
           (int)_MM_HINT_RST2, (int)_MM_HINT_IT0, (int)_MM_HINT_IT1);
    enum _mm_hint chosen = _MM_HINT_T1;
    printf("enum=%d size_matches_int=%d\n", (int)chosen, sizeof chosen == sizeof(int));

    /* The shape of a prefetching loop: every selector, every iteration. */
    unsigned long sum = 0;
    for (int i = 0; i < n; i += 64) {
        _mm_prefetch((const char *)a + i + 256, _MM_HINT_T0);
        _mm_prefetch((const char *)a + i + 320, _MM_HINT_T1);
        _mm_prefetch((const char *)a + i + 384, _MM_HINT_T2);
        _mm_prefetch((const char *)a + i + 448, _MM_HINT_NTA);
        _mm_prefetch((const char *)a + i + 512, _MM_HINT_ET0);
        _mm_prefetch((const char *)a + i + 576, _MM_HINT_RST2);
        _m_prefetchw(a + i + 640);
        for (int j = 0; j < 64; j++)
            sum += a[i + j];
    }
    printf("sum=%lu\n", sum);

    /* The address is evaluated once, and the selector is a constant
     * expression, not only a literal or an enumerator. */
    const char *p = (const char *)a;
    _mm_prefetch(p++, _MM_HINT_T0);
    printf("advanced=%d\n", (int)(p - (const char *)a));
    _mm_prefetch(next_line(p), _MM_HINT_T0 | 4);
    _mm_prefetch(next_line(p), 1 + 1);
    _m_prefetchw(next_line(p));
    printf("calls=%d\n", calls);

    /* Addresses nothing may read. */
    _mm_prefetch((const char *)0, _MM_HINT_T0);
    _mm_prefetch((const char *)a + n + (1 << 20), _MM_HINT_NTA);
    _mm_prefetch((const char *)(unsigned long)-4096, _MM_HINT_ET0);
    _m_prefetchw((void *)0);
    puts("survived");

    /* A hint written to the line it names leaves the line as it was. */
    unsigned check = 0;
    for (int i = 0; i < n; i++)
        check = check * 31 + a[i];
    printf("check=%u\n", check);
    free(a);
    return 0;
}
