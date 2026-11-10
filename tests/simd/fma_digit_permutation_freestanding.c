/* Freestanding (libc-free) FMA3 digit-permutation probe.
 *
 * Same differential as `fma_digit_permutation.c`, but with no C library: the
 * rows go straight to fd 1 through write(2) and the numbers are formatted
 * here.  That is what lets the gate EXECUTE the result on a backend whose
 * libc the host does not have -- a static 32-bit ELF runs natively on an
 * x86-64 kernel, so the i686 backend's operand placement is checked by the
 * values it produces, not by reading its assembly.
 *
 * The output grammar is deliberately identical to the hosted fixture, so the
 * driver parses both with one set of rules:
 *
 *     ref_ps lane<N> <value>        scalar a*b + c, per lane
 *     ps_132 lane<N> <value>        the 256-bit spelling under test
 *     ...
 *
 * Values are printed as scaled integers (value * 10).  Every payload is
 * exactly representable and every product is an integer well below 2^24, so
 * `scale` is exact in binary32 and the scaled integer is exact in int -- no
 * floating-point formatting is needed and no rounding can mask a defect.
 *
 * The pre-fix backend emitted `vfmadd213ps %ymm2,%ymm0,%ymm1` followed by
 * `vmovdqu %ymm0,(%rax)`, i.e. it stored `a`; measured here that prints
 * 30,50,70,... where 220,460,800,... is owed.  See the gate's header.
 */

typedef unsigned int u32;

/* The 256-bit builtins return `void *` = the ADDRESS of the result vector.
 * Declared here rather than via <lcccsimd.h> so the file needs no headers at
 * all (a reference compiler cannot parse lccc's intrinsic header). */
void *__lccc_simd256_ps_vfmadd132ps256(long, void *, void *, void *);
void *__lccc_simd256_ps_vfmadd213ps256(long, void *, void *, void *);
void *__lccc_simd256_ps_vfmadd231ps256(long, void *, void *, void *);
void *__lccc_simd256_ps_vfmadd132pd256(long, void *, void *, void *);
void *__lccc_simd256_ps_vfmadd213pd256(long, void *, void *, void *);
void *__lccc_simd256_ps_vfmadd231pd256(long, void *, void *, void *);

static int sys_write(const char *buf, u32 len) {
    int ret;
    __asm__ volatile("int $0x80" : "=a"(ret) : "a"(4), "b"(1), "c"(buf), "d"(len) : "memory");
    return ret;
}

static char *put_int(char *p, int v) {
    if (v < 0) {
        *p++ = '-';
        v = -v;
    }
    char tmp[12];
    int n = 0;
    do {
        tmp[n++] = (char)('0' + v % 10);
        v /= 10;
    } while (v);
    while (n) {
        *p++ = tmp[--n];
    }
    return p;
}

/* 64 rows of at most 24 bytes plus the exit path: 2 KiB is ample, and the
 * bound is static so no allocation and no bounds arithmetic is involved. */
static char out_buf[4096];
static char *out_pos = out_buf;

static void emit_row(const char *tag, int lane, int scaled) {
    while (*tag) {
        *out_pos++ = *tag++;
    }
    *out_pos++ = ' ';
    out_pos = put_int(out_pos, lane);
    *out_pos++ = ' ';
    out_pos = put_int(out_pos, scaled);
    *out_pos++ = '\n';
}

/* `volatile` forces a real load at runtime, so the optimiser cannot fold the
 * payloads into the result and make the comparison vacuous. */
static volatile float fa[8], fb[8], fc[8];
static volatile double da[4], db[4], dc[4];

/* Full width, all-distinct a*b+c per lane (see the hosted fixture): a 4-wide
 * table would leave lanes 4..7 zero and let a wrong permutation coincide. */
static const float FA[8] = {3.0f, 5.0f, 7.0f, 11.0f, 13.0f, 17.0f, 19.0f, 23.0f};
static const float FB[8] = {5.0f, 7.0f, 11.0f, 3.0f, 17.0f, 19.0f, 23.0f, 13.0f};
static const float FC[8] = {7.0f, 11.0f, 3.0f, 5.0f, 19.0f, 23.0f, 13.0f, 17.0f};
static const double DA[4] = {2.0, 6.0, 9.0, 14.0};
static const double DB[4] = {4.0, 8.0, 13.0, 5.0};
static const double DC[4] = {6.0, 10.0, 2.0, 7.0};

void _start(void) {
    for (int i = 0; i < 8; i++) {
        fa[i] = FA[i];
        fb[i] = FB[i];
        fc[i] = FC[i];
    }
    for (int i = 0; i < 4; i++) {
        da[i] = DA[i];
        db[i] = DB[i];
        dc[i] = DC[i];
    }

    for (int lane = 0; lane < 8; lane++) {
        emit_row("ref_ps", lane, (int)((fa[lane] * fb[lane] + fc[lane]) * 10.0f));
    }
    for (int lane = 0; lane < 4; lane++) {
        emit_row("ref_pd", lane, (int)((da[lane] * db[lane] + dc[lane]) * 10.0));
    }

    void *a = (void *)fa, *b = (void *)fb, *c = (void *)fc;
    {
        float *r = (float *)__lccc_simd256_ps_vfmadd132ps256(0, a, b, c);
        for (int i = 0; i < 8; i++) emit_row("ps_132", i, (int)(r[i] * 10.0f));
    }
    {
        float *r = (float *)__lccc_simd256_ps_vfmadd213ps256(0, a, b, c);
        for (int i = 0; i < 8; i++) emit_row("ps_213", i, (int)(r[i] * 10.0f));
    }
    {
        float *r = (float *)__lccc_simd256_ps_vfmadd231ps256(0, a, b, c);
        for (int i = 0; i < 8; i++) emit_row("ps_231", i, (int)(r[i] * 10.0f));
    }

    void *ad = (void *)da, *bd = (void *)db, *cd = (void *)dc;
    {
        double *r = (double *)__lccc_simd256_ps_vfmadd132pd256(0, ad, bd, cd);
        for (int i = 0; i < 4; i++) emit_row("pd_132", i, (int)(r[i] * 10.0));
    }
    {
        double *r = (double *)__lccc_simd256_ps_vfmadd213pd256(0, ad, bd, cd);
        for (int i = 0; i < 4; i++) emit_row("pd_213", i, (int)(r[i] * 10.0));
    }
    {
        double *r = (double *)__lccc_simd256_ps_vfmadd231pd256(0, ad, bd, cd);
        for (int i = 0; i < 4; i++) emit_row("pd_231", i, (int)(r[i] * 10.0));
    }

    sys_write(out_buf, (u32)(out_pos - out_buf));
    for (;;) {
        /* exit_group(0) -- the process is done and there is no caller. */
        __asm__ volatile("int $0x80" : : "a"(252), "b"(0) : "memory");
    }
}
