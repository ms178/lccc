/* GLA source-less rematerialization: critical-edge trampoline coverage.
 *
 * The global-location allocator rematerializes a source-less value
 * (GlobalAddr here; the i686 -O1 trace that first exercised the emitter
 * used a Copy-of-const) that feeds a φ incoming on ONE edge of a fan-out
 * predecessor under register pressure:
 *
 *   B (over budget): p = &gbuf; 6+ independent producers; cond -> M : E
 *   E (false arm):  in-body use of p (keeps p's live interval a single
 *                   continuous segment), q = p + K
 *   M (merge):      q = φ(p from B, q' from E); sink consumes producers
 *
 * The remat cannot be placed at B's exit (it would execute on the false
 * edge too), so the materializer isolates B->M with a fresh trampoline
 * block that rebuilds p and falls through to M, and rewires the φ
 * incoming onto it.  Before this test the emitter had only ever fired in
 * a translation unit that fails to assemble for i686 (x86-64-only inline
 * asm), so this is the first runnable coverage of the path.  Output must
 * be identical with CCC_RA_GLOBAL_LOCATION on/off and match the native
 * compiler. */
#include <stdint.h>
#include <stdio.h>

static char gbuf[64] = "ABCDEFGH-trampoline-path-coverage";
static volatile int sink;

/* x86-64 variant: nine params + eight long-lived producers hold the
 * fan-out block over the 12-GPR budget at every optimization level. The
 * same pressure is grossly over i686's 8-register ceiling, where the
 * planner leaves it alone. */
__attribute__((noinline))
static int select_ptr_64(int c, uint32_t a, uint32_t b, uint32_t d,
                         uint32_t e, uint32_t f, uint32_t g,
                         uint32_t h, uint32_t i, uint32_t j) {
    char *p = gbuf;
    /* Independent long-lived producers, each consuming params so every
     * parameter stays resident across the fan-out block. Nine params plus
     * the eight producers carry x86-64 to 13+/12 even after the -O2/O3
     * simplifiers; the same 17-name peak is grossly over i686's 8-register
     * ceiling, where the planner (correctly) leaves the function alone. */
    uint32_t x0 = a * b + 1u;
    uint32_t x1 = d * e + 2u;
    uint32_t x2 = f * g + 3u;
    uint32_t x3 = h * i + 4u;
    uint32_t x4 = j * a + 5u;
    uint32_t x5 = b ^ d ^ f ^ h;
    uint32_t x6 = e + g + i + j;
    uint32_t x7 = a * g + b * h;
    /* q is initialized to p BEFORE the branch and the true path has no
     * body: the merge φ's true-edge incoming (p) flows straight from the
     * fan-out predecessor B, while the false arm recomputes q. That is the
     * exact edge the trampoline isolates. */
    char *q = p;
    if (!(c & 1)) {
        char *alt = p + 9;        /* single in-body p-use cluster */
        sink += alt[0];
        q = alt;
    }
    sink += (int)(x0 ^ x1 ^ x2 ^ x3 ^ x4 ^ x5 ^ x6 ^ x7);
    sink += (int)(x0 + x1 + x2 + x3 + x6);
    sink += (int)(x4 * 3u + x5 * 5u + x7 * 9u);
    return (int)(q[0]) + (int)(q[1]) * 7 + (int)(q[2]) * 49;
}

/* i686 variant: the fan-out block itself must be the candidate's cover
 * block at 7/6 (reach band 2), and the gross-producer pressure lives in
 * the merge *after* the φ. Five params + p + q peak at exactly budget+1;
 * p is served on the false edge by an in-body clone and on the true φ
 * edge by the trampoline. On x86-64 this stays under the 12-GPR budget
 * and (correctly) does not fire. */
__attribute__((noinline))
static int select_ptr_32(int c, uint32_t a, uint32_t b, uint32_t d,
                         uint32_t e, uint32_t f) {
    char *p = gbuf;
    char *q = p;
    if (!(c & 1)) {
        char *alt = p + 9;
        sink += alt[0];
        q = alt;
    }
    /* Producer pressure belongs to the merge block, post-φ. */
    sink += (int)(a ^ b ^ d ^ e ^ f);
    sink += (int)(a * 3u + b * 5u + d * 7u + e * 11u + f * 13u);
    return (int)(q[0]) + (int)(q[1]) * 7 + (int)(q[2]) * 49;
}

int main(void) {
    int r = 0;
    for (int k = 0; k < 6; k++) {
        r += select_ptr_64(k, 11u + k, 13u + 2 * k, 17u, 19u - k, 23u,
                           29u + k, 31u, 37u + 3 * k, 41u + 5 * k);
        r += select_ptr_32(k, 11u + k, 13u + 2 * k, 17u, 19u - k, 23u);
    }
    printf("%d\n", r);
    return 0;
}
