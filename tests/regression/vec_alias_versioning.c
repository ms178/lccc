/* Elementwise (map/stencil) vectorization: dependence legality and runtime
 * alias versioning.
 *
 * Every kernel below is written WITHOUT `restrict`, so the compiler cannot
 * prove the streams disjoint.  Each is exercised with disjoint buffers, with
 * exact in-place aliasing, and with the destination running one/two/seven
 * elements (and 2 bytes) ahead of and behind the source.  The scalar
 * semantics (a recurrence when the destination is 1..W-1 elements ahead of
 * the source) must be preserved either by the runtime dependence-distance
 * guard entering the scalar loop, or by not vectorizing at all.
 *
 * Root cause pinned (upstream 5649e279, -O2): the stencil path accepted
 * `y[i] = a * x[i]` with distinct unproven pointers and then failed to build
 * its scalar remainder (the invariant leaf `a` is defined outside the loop),
 * leaving the vector header's exit edge pointing at a label that was never
 * created -- the function fell through into the next one and `scale()`
 * wrote nothing at all.  Trip counts here are all non-multiples of 8 so the
 * remainder loop is exercised on every call. */
#include <stdio.h>
#include <string.h>

#define N 61

__attribute__((noinline)) void scale_f(float *y, const float *x, float a, int n)
{ for (int i = 0; i < n; i++) y[i] = a * x[i]; }
__attribute__((noinline)) void scale_d(double *y, const double *x, double a, int n)
{ for (int i = 0; i < n; i++) y[i] = x[i] * a; }
__attribute__((noinline)) void add3_f(float *y, const float *x, const float *z, int n)
{ for (int i = 0; i < n; i++) y[i] = x[i] + z[i]; }
__attribute__((noinline)) void add3_i(int *y, const int *x, const int *z, int n)
{ for (int i = 0; i < n; i++) y[i] = x[i] + z[i]; }
__attribute__((noinline)) void axpy_f(float *y, const float *x, float a, int n)
{ for (int i = 0; i < n; i++) y[i] = a * x[i] + y[i]; }
__attribute__((noinline)) void addto_i(int *y, const int *x, int n)
{ for (int i = 0; i < n; i++) y[i] += x[i]; }
__attribute__((noinline)) void sub_f(float *y, const float *x, const float *z, int n)
{ for (int i = 0; i < n; i++) y[i] = x[i] - z[i] * 0.5f; }
__attribute__((noinline)) void stencil3(float *y, const float *x, int n)
{ for (int i = 1; i < n - 1; i++) y[i] = 0.25f * x[i - 1] + 0.5f * x[i] + 0.25f * x[i + 1]; }
/* True recurrence on one pointer: must never be vectorized. */
__attribute__((noinline)) void prefix_f(float *x, int n)
{ for (int i = 1; i < n; i++) x[i] = x[i - 1] + x[i]; }
/* Reads ahead of the write cursor on one pointer: legal to vectorize. */
__attribute__((noinline)) void shift_f(float *x, int n)
{ for (int i = 0; i < n - 1; i++) x[i] = x[i + 1] * 2.0f; }
/* Non-zero start: the map path used to assume element 0. */
__attribute__((noinline)) void from_one(float *y, const float *x, int n)
{ for (int i = 1; i < n; i++) y[i] = 2.0f * x[i]; }

static unsigned long long h;
static void mix(double v) { unsigned long long b; memcpy(&b, &v, 8); h = (h ^ b) * 0x100000001b3ULL; }
static void dump_f(const char *tag, const float *p, int n)
{ double s = 0; for (int i = 0; i < n; i++) { s += p[i] * (i + 1); mix(p[i]); } printf("%s %.9g\n", tag, s); }
static void dump_i(const char *tag, const int *p, int n)
{ long long s = 0; for (int i = 0; i < n; i++) { s += (long long)p[i] * (i + 1); mix(p[i]); } printf("%s %lld\n", tag, s); }
static void dump_d(const char *tag, const double *p, int n)
{ double s = 0; for (int i = 0; i < n; i++) { s += p[i] * (i + 1); mix(p[i]); } printf("%s %.17g\n", tag, s); }

static float fb[3][N + 32];
static double db[2][N + 32];
static int ib[3][N + 32];
static void init(void)
{
    for (int i = 0; i < N + 32; i++) {
        fb[0][i] = 1.0f + i * 0.125f; fb[1][i] = 2.0f - i * 0.0625f; fb[2][i] = (i % 7) - 3.0f;
        db[0][i] = 1.0 + i * 0.125; db[1][i] = 3.0 - i * 0.25;
        ib[0][i] = i * 3 - 40; ib[1][i] = 100 - i * 5; ib[2][i] = i * i;
    }
}
/* Offsets in ELEMENTS between dst and src (dst = src + off). */
static const int offs[] = { 0, 1, 2, 7, 8, 9, -1, -2, -7, -8, 15 };

int main(void)
{
    for (unsigned k = 0; k < sizeof offs / sizeof offs[0]; k++) {
        int off = offs[k];
        float *fbase = fb[0] + 16; double *dbase = db[0] + 16; int *ibase = ib[0] + 16;
        char tag[64];

        init(); scale_f(fbase + off, fbase, 1.5f, N); snprintf(tag, sizeof tag, "scale_f[%d]", off); dump_f(tag, fb[0], N + 32);
        init(); scale_d(dbase + off, dbase, 1.5, N); snprintf(tag, sizeof tag, "scale_d[%d]", off); dump_d(tag, db[0], N + 32);
        init(); add3_f(fbase + off, fbase, fb[1], N); snprintf(tag, sizeof tag, "add3_f[%d]", off); dump_f(tag, fb[0], N + 32);
        init(); add3_f(fbase, fb[1], fbase + off, N); snprintf(tag, sizeof tag, "add3_f_z[%d]", off); dump_f(tag, fb[0], N + 32);
        init(); add3_i(ibase + off, ibase, ib[1], N); snprintf(tag, sizeof tag, "add3_i[%d]", off); dump_i(tag, ib[0], N + 32);
        init(); axpy_f(fbase + off, fbase, 0.75f, N); snprintf(tag, sizeof tag, "axpy_f[%d]", off); dump_f(tag, fb[0], N + 32);
        init(); addto_i(ibase + off, ibase, N); snprintf(tag, sizeof tag, "addto_i[%d]", off); dump_i(tag, ib[0], N + 32);
        init(); sub_f(fbase + off, fbase, fb[2], N); snprintf(tag, sizeof tag, "sub_f[%d]", off); dump_f(tag, fb[0], N + 32);
        init(); stencil3(fbase + off, fbase, N); snprintf(tag, sizeof tag, "stencil3[%d]", off); dump_f(tag, fb[0], N + 32);
        init(); from_one(fbase + off, fbase, N); snprintf(tag, sizeof tag, "from_one[%d]", off); dump_f(tag, fb[0], N + 32);
    }
    /* Partial (2-byte) overlap: dst = (char*)src + 2.  The cast forms a
     * MISALIGNED float pointer, so dereferencing it is undefined behaviour
     * and this case cannot take part in the GCC-oracle output comparison:
     * GCC exploits the UB and vectorizes without a guard, while lccc keeps
     * its defined-behaviour fallback and enters the scalar remainder —
     * both are valid under the standard and their bytes differ.  The case
     * is still executed so the runtime guard-fallback path (the exact
     * reason the test exists) is exercised on every run: the kernel must
     * neither crash nor fall through, and the process must exit 0.  Its
     * bytes are deliberately not printed and not mixed into the hash. */
    init(); scale_f((float *)((char *)(fb[0] + 16) + 2), fb[0] + 16, 1.5f, N);
    init(); prefix_f(fb[0] + 1, N); dump_f("prefix_f", fb[0], N + 32);
    init(); shift_f(fb[0] + 1, N); dump_f("shift_f", fb[0], N + 32);
    printf("hash %016llx\n", h);
    return 0;
}
