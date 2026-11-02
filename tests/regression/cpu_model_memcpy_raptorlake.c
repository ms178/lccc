/* CPU tuning model — block-copy lowering must follow the -march contract and
 * the -mtune strategy (docs/CPU_MODEL_AUDIT.md §2.1, §4).
 *
 * Before the fix `emit_memcpy_impl_impl` emitted `vmovdqu %ymm` for every
 * copy > 64 bytes regardless of -march (SIGILL on pre-AVX hosts, and a
 * violation of the -march=x86-64 contract), and the YMM loop never armed the
 * epilogue vzeroupper.  With -mtune=raptorlake the 4096-byte copy is lowered
 * to `rep movsb` (glibc's FSRM threshold is 2112); the count register set-up
 * in front of it used to be deleted by the liveness peephole.
 *
 * The suite compiles this file at the default -march (x86-64 baseline) and
 * runs it; the flags sidecar selects -mtune=raptorlake so both the rep movsb
 * path (4096 B) and the SSE2 loop path (200 B, 1000 B) are exercised. */
#include <stdio.h>
#include <string.h>

struct big { unsigned char b[4096]; };
struct mid { unsigned char b[1000]; };
struct small_odd { unsigned char b[200]; };

__attribute__((noinline)) void cp_big(struct big *d, const struct big *s) { *d = *s; }
__attribute__((noinline)) void cp_mid(struct mid *d, const struct mid *s) { *d = *s; }
__attribute__((noinline)) void cp_odd(struct small_odd *d, const struct small_odd *s) { *d = *s; }

/* Scalar FP after the copy: any AVX/SSE transition or clobbered state shows
 * up as a wrong sum. */
__attribute__((noinline)) double mix(const unsigned char *p, int n) {
    double acc = 0.0;
    for (int i = 0; i < n; i++) acc = acc * 1.0000001 + p[i];
    return acc;
}

static struct big A, B;
static struct mid M1, M2;
static struct small_odd O1, O2;

int main(void) {
    unsigned long h = 1469598103934665603ULL;
    for (int i = 0; i < 4096; i++) A.b[i] = (unsigned char)(i * 131 + 7);
    for (int i = 0; i < 1000; i++) M1.b[i] = (unsigned char)(i * 29 + 3);
    for (int i = 0; i < 200; i++) O1.b[i] = (unsigned char)(i * 17 + 1);
    for (int rep = 0; rep < 64; rep++) {
        memset(&B, 0, sizeof B);
        memset(&M2, 0xAA, sizeof M2);
        memset(&O2, 0x55, sizeof O2);
        cp_big(&B, &A);
        cp_mid(&M2, &M1);
        cp_odd(&O2, &O1);
        if (memcmp(&A, &B, sizeof A) != 0) { puts("FAIL big"); return 1; }
        if (memcmp(&M1, &M2, sizeof M1) != 0) { puts("FAIL mid"); return 1; }
        if (memcmp(&O1, &O2, sizeof O1) != 0) { puts("FAIL odd"); return 1; }
        double s = mix(B.b, 4096) + mix(M2.b, 1000) + mix(O2.b, 200);
        h = (h ^ (unsigned long)s) * 1099511628211ULL;
        A.b[rep] ^= (unsigned char)rep;
    }
    printf("ok %lu\n", h);
    return 0;
}
