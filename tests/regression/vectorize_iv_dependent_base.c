/* Reduction vectorizer must NOT treat a multi-level GEP whose BASE depends
 * on the induction variable as a contiguous stride-elem_size stream.
 *
 * s += C[i][i] lowers to
 *     row  = C + (i << 11)     <- base depends on i!
 *     addr = row + (i << 3)    <- offset alone looks like stride-8
 *
 * The transform rewrites the IV to step in BYTES (0,32,64,..,2047) and keeps
 * every IV use, so the row computation becomes C + (byte_iv << 11): up to
 * 2047<<11 = 4 MB past C -- out-of-bounds reads, SIGSEGV on a 256x256
 * static matrix (found via matmul diagonal-sum; present upstream).
 *
 * The fix requires the GEP base to be loop-invariant (defined outside the
 * loop, or GlobalAddr/Alloca). Contiguous reductions (s += a[i]) must still
 * vectorize -- covered by vectorize_f32_sum / vectorize_dot_product.
 */
#include <stdio.h>

#define N 256
static double C[N][N];

int main(void) {
    for (int i = 0; i < N; i++)
        C[i][i] = (double)i;

    double s = 0.0;
    for (int i = 0; i < N; i++)
        s += C[i][i];          /* diagonal walk: stride N+1 elements */

    /* sum 0..255 = 32640 */
    if (s != 32640.0) { printf("FAIL diag sum %.1f\n", s); return 1; }

    /* variant: IV-dependent base via row pointer arithmetic */
    double t = 0.0;
    for (int i = 0; i < N; i += 2)
        t += C[i][i];
    if (t != 16256.0) { printf("FAIL strided diag %.1f\n", t); return 2; }

    printf("32640.0\n");
    return 0;
}
