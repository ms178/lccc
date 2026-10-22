// FLAGS: -mavx2
#include <immintrin.h>
#include <stdio.h>
#include <stdint.h>
#define WIDTH(t, n) _Static_assert(sizeof(t) == n, "wrong GNU vector width")
WIDTH(__v2df,16); WIDTH(__v2di,16); WIDTH(__v8hi,16); WIDTH(__v16qi,16);
WIDTH(__v4df,32); WIDTH(__v4di,32); WIDTH(__v16hi,32); WIDTH(__v32qi,32);
WIDTH(__v8df,64); WIDTH(__v8di,64); WIDTH(__v32hi,64); WIDTH(__v64qi,64);
static void dump(__m256d v) {
    uint64_t a[4]; _mm256_storeu_pd((double *)a,v);
    for (int i=0;i<4;++i) printf("%016llx ", (unsigned long long)a[i]);
    puts("");
}
int main(void) {
    /* Distinct high and low dwords expose accidental VPERMILPS emission. */
    double a[4] = {1.25,-17.5,4097.125,-65537.75};
    __m256d v = _mm256_loadu_pd(a);
    dump(_mm256_permute_pd(v,0)); dump(_mm256_permute_pd(v,1));
    dump(_mm256_permute_pd(v,2)); dump(_mm256_permute_pd(v,3));
    dump(_mm256_permute_pd(v,4)); dump(_mm256_permute_pd(v,5));
    dump(_mm256_permute_pd(v,6)); dump(_mm256_permute_pd(v,7));
    dump(_mm256_permute_pd(v,8)); dump(_mm256_permute_pd(v,9));
    dump(_mm256_permute_pd(v,10)); dump(_mm256_permute_pd(v,11));
    dump(_mm256_permute_pd(v,12)); dump(_mm256_permute_pd(v,13));
    dump(_mm256_permute_pd(v,14)); dump(_mm256_permute_pd(v,15));
    return 0;
}
