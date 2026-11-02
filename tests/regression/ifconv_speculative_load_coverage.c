/*
 * R1 red-team regression: if-conversion must NOT speculate arm loads whose
 * address the scalar program does not dereference on every path.
 *
 * Bug (competing revision, since fixed): the "IV-addressed inside a loop"
 * shape gate treated every induction-variable-addressed arm load as
 * speculatable.  For `d[i] = a[i] > t ? 1.0f : c[i]` the stream `c` is
 * dereferenced ONLY on the false path; hoisting the load into the branch
 * predecessor (and into the vector body after late map vectorization) reads
 * c[i] for iterations the source program never reads — a fault the C
 * program cannot produce.  Below, `c` is an mmap'd page followed by a
 * PROT_NONE guard page and the loop is arranged so the false arm is taken
 * only for i < 4: the scalar never reads beyond c[0..3], while the
 * speculated form walks the full trip count and crosses the guard page.
 *
 * Reference behavior (GCC 14, -O3, x86-64-v3, measured): GCC keeps this
 * load behind `vmaskmovps` — masked, not speculated.  The sound
 * path-coverage gate refuses the diamond here and the loop stays branchy.
 *
 * The second half locks the SOUND speculation shapes in: when every path
 * dereferences the arm address (sign_apply: `a[i]` is read by both arms),
 * the diamond must still convert — the output stays byte-identical to GCC.
 * Compile with -O3 -march=x86-64-v3 so if-conversion and the late map
 * vectorizer both run.
 */
#include <stdio.h>
#include <stdint.h>
#include <sys/mman.h>
#include <string.h>

#define N 64

__attribute__((noinline))
void cond_load(float *restrict d, const float *restrict a,
               const float *restrict c, float t, int n) {
    for (int i = 0; i < n; i++)
        d[i] = a[i] > t ? 1.0f : c[i];
}

__attribute__((noinline))
void sign_apply(float *restrict d, const float *restrict a,
                const float *restrict s, int n) {
    for (int i = 0; i < n; i++)
        d[i] = s[i] < 0.0f ? -a[i] : a[i];
}

int main(void) {
    /* Guard page setup: one readable page whose LAST 16 bytes back `c`,
     * then a PROT_NONE page.  The scalar reads c[0..3] (exactly 16 bytes);
     * any speculated read of c[4..] or any 8-lane vector load near the trip
     * end crosses into the guard page and faults deterministically. */
    char *base = mmap(NULL, 2 * 4096, PROT_READ | PROT_WRITE,
                      MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (base == MAP_FAILED) {
        printf("setup mmap-fail\n");
        return 0; /* cannot exercise the guard here; still compare stdout */
    }
    if (mprotect(base + 4096, 4096, PROT_NONE) != 0) {
        printf("setup mprotect-fail\n");
        return 0;
    }
    float *c = (float *)(base + 4096 - 16);

    float a[N], s[N], d[N];
    for (int i = 0; i < N; i++) {
        /* i < 4 -> small (false arm taken, reads c[i]); i >= 4 -> large
         * (true arm taken; the scalar NEVER reads c[i] again). */
        a[i] = (i < 4) ? -1.0f * (float)(4 - i) : 100.0f + (float)i;
        s[i] = (i % 2) ? -2.5f : 3.5f;
    }
    c[0] = 10.0f; c[1] = 20.0f; c[2] = 30.0f; c[3] = 40.0f;

    memset(d, 0, sizeof d);
    cond_load(d, a, c, 0.0f, N);
    /* Expected: i<4 -> c[i]; i>=4 -> 1.0f.  A speculated (unsound) build
     * faults on the guard page before this line prints. */
    for (int i = 0; i < N; i++)
        printf("cl %d %.1f\n", i, d[i]);

    memset(d, 0, sizeof d);
    sign_apply(d, a, s, N);
    for (int i = 0; i < N; i++)
        printf("sa %d %.1f\n", i, d[i]);

    printf("done\n");
    return 0;
}
