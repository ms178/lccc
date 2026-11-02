/* GNU89 extern-inline (gnu_inline) bodies must be inlined at -O2.
 *
 * `extern __inline __attribute__((__gnu_inline__))` provides an
 * inline-ONLY definition: no out-of-line copy is emitted in this TU, and a
 * call left behind resolves to an external symbol. glibc's rtld links with
 * -nostdlib, so the leftover call to `__bsearch` (bits/stdlib-bsearch.h,
 * used by intel_check_word via dl-cacheinfo.h) was a hard undefined-symbol
 * error at the ld.so link.
 *
 * The inliner used to reject the body on block count (bsearch: ~10 blocks
 * with a loop > MAX_INLINE_BLOCKS): is_gnu_inline_def now has a dedicated
 * eligibility class. This test mirrors the bsearch shape 1:1 and fails to
 * LINK if the class regresses (nm would show `U my_bsearch`).
 */
#include <stdio.h>

typedef int (*cmp_t)(const void *, const void *);

extern __inline __attribute__((__gnu_inline__)) void *
my_bsearch(const void *key, const void *base, unsigned long nmemb,
           unsigned long size, cmp_t cmp)
{
    const void *p;
    int c;
    while (nmemb) {
        p = (const void *)(((const char *)base) + ((nmemb >> 1) * size));
        c = (*cmp)(key, p);
        if (c == 0)
            return (void *)p;
        if (c > 0) {
            base = ((const char *)p) + size;
            nmemb -= (nmemb >> 1) + 1;
        } else
            nmemb >>= 1;
    }
    return (void *)0;
}

static int icmp(const void *a, const void *b)
{
    return *(const int *)a - *(const int *)b;
}

static int find(const int *arr, unsigned long n, int key)
{
    void *r = my_bsearch(&key, arr, n, sizeof(int), icmp);
    return r ? *(int *)r : -1;
}

int main(void)
{
    int a[7] = { 2, 3, 5, 7, 11, 13, 17 };
    int ok = find(a, 7, 7) == 7 && find(a, 7, 2) == 2 && find(a, 7, 17) == 17
        && find(a, 7, 4) == -1 && find(a, 7, 1) == -1 && find(a, 0, 2) == -1;
    printf("bsearch:%s\n", ok ? "ok" : "MISMATCH");
    return ok ? 0 : 1;
}
