/* Freestanding i686 128-bit VEX FMA3 probe.
 *
 * Executes all six digit permutations (ps and pd) and reports any lane that is
 * not `a*b + c`.  Driven by `tests/regression/check_i686_fma128.sh`, which
 * builds it with `-m32 ... -ffreestanding -nostdlib -static` and RUNS it: this
 * host has no 32-bit libc headers, and a static i386 ELF needs none when it
 * reaches the kernel through `int $0x80` directly.
 *
 * Payloads are small integers, so `float`/`double` arithmetic is exact and the
 * comparison cannot be defeated by rounding: 1*2+3 = 5, 2*3+4 = 10, and so on,
 * with every lane's expected value distinct so a wrong permutation cannot
 * coincide with the right one.
 *
 * Measured before the backend arm existed: this translation unit did not
 * compile at all -- the compiler PANICKED in `reject_value_form`.
 */
typedef unsigned int u32;
void *__lccc_simd128_ps_fmadd132ps128(long, void *, void *, void *);
void *__lccc_simd128_ps_fmadd213ps128(long, void *, void *, void *);
void *__lccc_simd128_ps_fmadd231ps128(long, void *, void *, void *);
void *__lccc_simd128_ps_fmadd132pd128(long, void *, void *, void *);
void *__lccc_simd128_ps_fmadd213pd128(long, void *, void *, void *);
void *__lccc_simd128_ps_fmadd231pd128(long, void *, void *, void *);
/* 256-bit: the same fixed-placement store bug lived here (that is what SA-11
 * was), so one executable probe covers both widths on this backend. */
void *__lccc_simd256_ps_vfmadd132ps256(long, void *, void *, void *);
void *__lccc_simd256_ps_vfmadd213ps256(long, void *, void *, void *);
void *__lccc_simd256_ps_vfmadd231ps256(long, void *, void *, void *);

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
static char buf[2048]; static char *o = buf;
static char *put_s(char *p, const char *s) { while (*s) *p++ = *s++; return p; }

static volatile float fa[4], fb[4], fc[4];
static volatile double da[2], db[2], dc[2];
static volatile float ga[8], gb[8], gc[8];

void _start(void) {
    for (int i = 0; i < 4; i++) { fa[i] = (float)(i+1); fb[i] = (float)(i+2); fc[i] = (float)(i+3); }
    for (int i = 0; i < 2; i++) { da[i] = (double)(i+2); db[i] = (double)(i+4); dc[i] = (double)(i+6); }
    for (int i = 0; i < 8; i++) { ga[i] = (float)(i+1); gb[i] = (float)(i+2); gc[i] = (float)(i+3); }

    int bad = 0;
    void *A=(void*)fa, *B=(void*)fb, *C=(void*)fc;
    for (int k = 0; k < 3; k++) {
        float *r = k==0 ? (float*)__lccc_simd128_ps_fmadd132ps128(0,A,B,C)
                 : k==1 ? (float*)__lccc_simd128_ps_fmadd213ps128(0,A,B,C)
                        : (float*)__lccc_simd128_ps_fmadd231ps128(0,A,B,C);
        for (int i = 0; i < 4; i++) {
            float want = fa[i]*fb[i] + fc[i];
            if (r[i] != want) { o = put_s(o, "ps lane"); o = put_i(o, i); *o++=' '; o = put_i(o,(int)r[i]); *o++=' '; o = put_i(o,(int)want); *o++='\n'; bad++; }
        }
    }
    void *DA_=(void*)da, *DB_=(void*)db, *DC_=(void*)dc;
    for (int k = 0; k < 3; k++) {
        double *r = k==0 ? (double*)__lccc_simd128_ps_fmadd132pd128(0,DA_,DB_,DC_)
                  : k==1 ? (double*)__lccc_simd128_ps_fmadd213pd128(0,DA_,DB_,DC_)
                         : (double*)__lccc_simd128_ps_fmadd231pd128(0,DA_,DB_,DC_);
        for (int i = 0; i < 2; i++) {
            double want = da[i]*db[i] + dc[i];
            if (r[i] != want) { o = put_s(o, "pd lane"); o = put_i(o, i); *o++=' '; o = put_i(o,(int)r[i]); *o++=' '; o = put_i(o,(int)want); *o++='\n'; bad++; }
        }
    }
    void *GA=(void*)ga, *GB=(void*)gb, *GC=(void*)gc;
    for (int k = 0; k < 3; k++) {
        float *r = k==0 ? (float*)__lccc_simd256_ps_vfmadd132ps256(0,GA,GB,GC)
                 : k==1 ? (float*)__lccc_simd256_ps_vfmadd213ps256(0,GA,GB,GC)
                        : (float*)__lccc_simd256_ps_vfmadd231ps256(0,GA,GB,GC);
        for (int i = 0; i < 8; i++) {
            float want = ga[i]*gb[i] + gc[i];
            if (r[i] != want) { o = put_s(o, "ps256 lane"); o = put_i(o, i); *o++=' '; o = put_i(o,(int)r[i]); *o++=' '; o = put_i(o,(int)want); *o++='\n'; bad++; }
        }
    }
    o = put_s(o, bad ? "FAIL: " : "PASS: all 42 lanes (18 x 128-bit + 24 x 256-bit) = a*b+c (bad=");
    o = put_i(o, bad); o = put_s(o, ")\n");
    sys_write(buf, (u32)(o - buf));
    for (;;) __asm__ volatile ("int $0x80" : : "a"(252), "b"(0) : "memory");
}
