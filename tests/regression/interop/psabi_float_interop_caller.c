/* Caller half of the floating-point interop fixture; see psabi_float_interop.h.
 * Self-checking: exit status 0 is success; abort() otherwise. */
#include "psabi_float_interop.h"

extern void abort(void);

static volatile int zero;

/* Built at run time so every call below stays an indirect call. */
static struct ops table;

int main(void)
{
    const struct P p = {1.5 + zero, 0.25};
    const struct P q = {0.5, 3.0 + zero};
    const struct F f = {2.0f, 0.5f + zero};
    const struct DI di = {0.5 + zero, 7};
    double (*elems[2])(struct P) = {p_sum, p_sum};
    double (*local)(struct P) = p_sum;

    table.p = p_sum;
    table.f = f_sum;
    table.di = di_sum;
    table.v = v_doubles;
    table.vp = v_points;
    struct ops *volatile ops = &table;

    if (nine_doubles(1, 2, 3, 4, 5, 6, 7, 8, 9 + zero, 10) !=
        1 + 4 + 12 + 32 + 80 + 192 + 448 + 1024 + 2304 + 5120.0)
        abort();
    if (ten_floats(11, 1, 2, 3, 4, 5, 6, 7, 8, 9 + zero, 10) !=
        1 + 4 + 12 + 32 + 80 + 192 + 448 + 1024 + 2304 + 5120 + 11264.0f)
        abort();
    if (nine_then_variadic(1, 2, 3, 4, 5, 6, 7, 8, 9 + zero, 10.0, 11ll) !=
        1 + 4 + 12 + 32 + 80 + 192 + 448 + 1024 + 2304 + 5120 + 11264.0)
        abort();
    if (di_sum(di) != 8.0 || ops->di(di) != 8.0)
        abort();
    if (ops->v(3, 1.5, 2.5, 3.0) != 1.5 + 5.0 + 12.0 || (*ops->v)(1, 8.0) != 8.0)
        abort();
    if (p_sum(p) != 3.25 || f_sum(3, f) != 11.5)
        abort();
    if (ops->p(p) != 3.25 || ops->f(3, f) != 11.5)
        abort();
    if (elems[zero](q) != 4.0 || local(p) != 3.25 || through_param(p_sum, q) != 8.0)
        abort();
    if (ops->vp(2, p, q) != 3.25 + 16.0 || v_points(1, q) != 4.0)
        abort();
    return 0;
}
