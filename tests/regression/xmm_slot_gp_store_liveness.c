/* Regression: scalar-FP frame-slot moves must participate in the peephole
 * slot model (x86 `StoreXmmRbp` / `LoadXmmRbp`).
 *
 * Upstream b5c634ed introduced dedicated line kinds for movss/movsd slot
 * traffic but constructed them with empty memory-operand caches, so:
 *   (1) windowed dead-store elimination could not see `movss slot, %xmmN`
 *       and deleted the GP store feeding it (`movq %rax, slot`) — the -O0
 *       int<->float bit-transfer idiom broke: gcc.c-torture 20000605-1.c,
 *       pr47538.c, pr59229.c, pr109938.c, pr109986.c, 20021120-1.c;
 *   (2) store forwarding kept a GP slot mapping alive across an XMM store to
 *       the same slot, forwarding a stale GP value into a later reload.
 *
 * Kept at -O0 (see .flags) because the idiom is emitted by the unoptimised
 * lowering; every optimisation level must still pass. */
void abort(void);

typedef struct { int y; float scaley; int src_y; } RenderInfo;

static void bar(void) {}

/* Shape (1): float error flows through a GP register and a frame slot. */
static int render(RenderInfo *info)
{
    int y = info->y, ye = 256;
    float step = 1.0 / info->scaley;
    float error = y * step;
    error -= ((int) error) - step;
    for (; y < ye; y++) {
        if (error >= 1.0) {
            info->src_y += (int) error;
            error -= (int) error;
            bar();
        }
        error += step;
    }
    return info->src_y;
}

/* Shape (1b): double variant with an alternating-sign reduction. */
struct S { double a, b, *c; unsigned long d; };
__attribute__((noinline)) static void foo(struct S *x, const struct S *y)
{
    const unsigned long n = y->d + 1;
    const double m = 0.25 * (y->b - y->a);
    x->a = y->a; x->b = y->b;
    if (n == 1) x->c[0] = 0.;
    else if (n == 2) { x->c[1] = m * y->c[0]; x->c[0] = 2.0 * x->c[1]; }
    else {
        double o = 0.0, p = 1.0; unsigned long i;
        for (i = 1; i <= n - 2; i++) {
            x->c[i] = m * (y->c[i - 1] - y->c[i + 1]) / (double) i;
            o += p * x->c[i];
            p = -p;
        }
        x->c[n - 1] = m * y->c[n - 2] / (n - 1.0);
        o += p * x->c[n - 1];
        x->c[0] = 2.0 * o;
    }
}

/* Shape (2): the same slot is written by a GP store, then by an XMM store,
 * then reloaded into a GP register — the reload must observe the XMM bits. */
union Bits { float f; unsigned u; };
__attribute__((noinline)) static unsigned reslot(unsigned seed, float f)
{
    union Bits b;
    b.u = seed;            /* GP store to the slot */
    if (seed & 1)
        b.f = f;           /* XMM store fully covering the slot */
    return b.u + 1;        /* GP reload: must not forward `seed` */
}

int main(void)
{
    RenderInfo info = { 0, 1.0f, 0 };
    if (render(&info) != 256)
        abort();

    struct S x, y;
    double c[4] = { 10, 20, 30, 40 }, d[4], e[4] = { 118, 118, 118, 118 };
    y.a = 10; y.b = 6; y.c = c; x.c = d; y.d = 3;
    __builtin_memcpy(d, e, sizeof d);
    foo(&x, &y);
    if (d[0] != 0 || d[1] != 20 || d[2] != 10 || d[3] != -10)
        abort();
    y.d = 0;
    __builtin_memcpy(d, e, sizeof d);
    foo(&x, &y);
    if (d[0] != 0 || d[1] != 118)
        abort();

    union Bits probe; probe.f = 1.5f;
    if (reslot(7u, 1.5f) != probe.u + 1)
        abort();
    if (reslot(6u, 1.5f) != 7u)
        abort();
    return 0;
}
