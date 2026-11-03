/* A call through any callee expression whose type is a pointer to a variadic
 * prototype -- a struct member, an array element, a cast, a call result, a
 * dereferenced pointer-to-pointer -- is a variadic call.  LCCC classified
 * every such call as fixed: on x86-64 no `mov $n, %al` preceded the
 * `call *reg`, so a variadic callee that tests %al skipped its XMM save area
 * and va_arg read stale vector registers (`tbl[0](2, 4.0, 0.25)` returned 4
 * at -O0); on RV64 the unnamed doubles went to fa-registers that the
 * callee's va_arg never reads.  Self-checking: exit status 0 is success. */
#include <stdarg.h>

extern void abort(void);

typedef double (*vsum_fn)(int, ...);

__attribute__((noinline)) static double vsum(int n, ...)
{
    va_list ap;
    double s = 0, w = 1;
    va_start(ap, n);
    for (int i = 0; i < n; i++, w *= 2)
        s += va_arg(ap, double) * w;
    va_end(ap);
    return s;
}

__attribute__((noinline)) static long long vmix(int n, ...)
{
    va_list ap;
    long long s = 0;
    va_start(ap, n);
    for (int i = 0; i < n; i++) {
        double d = va_arg(ap, double);
        int k = va_arg(ap, int);
        s = s * 100 + (long long)(d * 4) + k;
    }
    va_end(ap);
    return s;
}

struct ops {
    int tag;
    double (*sum)(int, ...);
    long long (*mix)(int, ...);
};

static volatile int zero;
static struct ops table;

/* Returns its argument in the return register: a zero %al right before the
 * indirect call below is then likely, not left to chance. */
__attribute__((noinline)) static int pick(int i) { return i + zero; }

__attribute__((noinline)) static vsum_fn get_sum(void) { return table.sum; }

int main(void)
{
    vsum_fn tbl[2];
    void *erased;
    vsum_fn *indirect = &tbl[1];

    table.sum = vsum;
    table.mix = vmix;
    tbl[0] = vsum;
    tbl[1] = vsum;
    erased = (void *)vsum;
    struct ops *volatile o = &table;

    if (o->sum(3, 1.5, 2.5, 3.0) != 1.5 + 5.0 + 12.0)
        abort();
    if (tbl[pick(0)](2, 4.0, 0.25) != 4.5)
        abort();
    if ((*o->sum)(1, 8.0) != 8.0)
        abort();
    if (((double (*)(int, ...))erased)(2, 0.5, 0.75) != 2.0)
        abort();
    if (get_sum()(4, 1.0, 1.0, 1.0, 1.0) != 15.0)
        abort();
    if ((*indirect)(1, 6.25) != 6.25)
        abort();
    if (o->mix(3, 0.25, 7, 1.5, 8, 2.0, 9) != ((1 + 7) * 100 + 6 + 8) * 100 + 8 + 9)
        abort();
    return 0;
}
