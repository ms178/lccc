/* ARM NEON reduction/map lowering: the Vec* load intrinsics carry a
 * (base, byte-offset) argument pair; the byte-IV rewrite passes the loop's
 * marching offset as args[1]. The original lowering read only args[0], so
 * every vector iteration loaded the SAME lanes: sum of 1024 i32 returned
 * 512*(a[0]+a[1]) instead of the true sum (8704 vs 1578496 on this input).
 * F64 reductions and the NEON map transform hit the same hole.
 * On x86 this test also guards the SSE2/AVX2 offset paths. */
extern int printf(const char *, ...);

static int a32[1024];
static double f64[512];
static int dst[1024], src[1024];

int main(void) {
    /* I32 -> I64 widening reduction */
    for (int i = 0; i < 1024; i++) a32[i] = i * 3 + 7;
    long s = 0;
    for (int i = 0; i < 1024; i++) s += a32[i];
    if (s != 1578496) { printf("FAIL i32 sum %ld\n", s); return 1; }

    /* F64 reduction */
    for (int i = 0; i < 512; i++) f64[i] = i * 0.5;
    double d = 0;
    for (int i = 0; i < 512; i++) d += f64[i];
    if (d != 65408.0) { printf("FAIL f64 sum %.1f\n", d); return 2; }

    /* map loop (NEON 4-wide on AArch64) */
    for (int i = 0; i < 1024; i++) src[i] = i;
    for (int r = 0; r < 3; r++)
        for (int i = 0; i < 1024; i++)
            dst[i] = src[i] * 3 + 7;
    long m = 0;
    for (int i = 0; i < 1024; i++) m += dst[i];
    if (m != 1578496) { printf("FAIL map %ld\n", m); return 3; }

    printf("1578496\n");
    return 0;
}
