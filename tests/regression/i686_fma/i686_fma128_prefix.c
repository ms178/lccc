/* The PRE-FIX operand placement, executed to prove the probe is not vacuous.
 *
 * The 128-bit i686 arm did not exist, so there was never an emitted sequence to
 * copy; this is the sequence the backend WOULD have produced had the arm been
 * added with the same fixed placement the 256-bit arm had before SA-11:
 * 213 selected (`vfmadd213ps`), the 132 roles (`rm=xmm1, vvvv=xmm2,
 * dst=xmm0`), and `%xmm0` stored regardless.
 *
 * `check_i686_fma128.sh` requires this to report a non-zero number of wrong
 * lanes.  A probe that cannot see the defect it was written for is decoration,
 * and the whole reason the 128-bit arm could go missing unnoticed is that no
 * gate was measuring it.
 */
typedef unsigned int u32;
static int sys_write(const char *p, u32 n) {
    int r; __asm__ volatile ("int $0x80" : "=a"(r) : "a"(4), "b"(1), "c"(p), "d"(n) : "memory");
    return r;
}
static char *put_i(char *p, int v) {
    if (v < 0) { *p++ = '-'; v = -v; }
    char t[12]; int n = 0;
    do { t[n++] = (char)('0' + v % 10); v /= 10; } while (v);
    while (n) *p++ = t[--n];
    return p;
}
static char buf[1024]; static char *o = buf;
static volatile float fa[4], fb[4], fc[4], out[4];

void _start(void) {
    for (int i = 0; i < 4; i++) { fa[i] = (float)(i+1); fb[i] = (float)(i+2); fc[i] = (float)(i+3); }
    __asm__ volatile (
        "vmovdqu (%0), %%xmm0\n\t"
        "vmovdqu (%1), %%xmm1\n\t"
        "vmovdqu (%2), %%xmm2\n\t"
        "vfmadd213ps %%xmm2, %%xmm0, %%xmm1\n\t"   /* result in xmm1 */
        "vmovdqu %%xmm0, (%3)\n\t"                 /* pre-fix: stores a */
        : : "r"(fa), "r"(fb), "r"(fc), "r"((void*)out)
        : "xmm0","xmm1","xmm2","memory");
    int bad = 0;
    for (int i = 0; i < 4; i++) if (out[i] != fa[i]*fb[i]+fc[i]) bad++;
    o = put_i(o, bad); 
    const char *m = " of 4 lanes wrong (pre-fix sequence)\n";
    while (*m) *o++ = *m++;
    sys_write(buf, (u32)(o - buf));
    for (;;) __asm__ volatile ("int $0x80" : : "a"(252), "b"(0) : "memory");
}
