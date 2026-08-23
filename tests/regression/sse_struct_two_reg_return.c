/* IS-02/AB-01: all-SSE two-eightbyte struct returns.
 *
 * `struct pair { double, double }` is classified [Sse, Sse] and returns in
 * xmm0/xmm1. GCC-built objects and LCCC-built objects must interoperate in
 * BOTH directions (the .sibling object below is compiled by the host GCC
 * in the compare-gcc mode; in lccc-only mode this file alone pins the
 * runtime values). The mixed [Integer, Sse] shape keeps the integer path. */
struct pair { double x, y; };
struct mixed { long tag; double val; };

struct pair mk(double a, double b) {
    struct pair p;
    p.x = a;
    p.y = b;
    return p;
}

struct pair swap(struct pair *p) {
    struct pair r;
    r.x = p->y;
    r.y = p->x;
    return r;
}

struct mixed mk_mixed(long t, double v) {
    struct mixed m;
    m.tag = t;
    m.val = v;
    return m;
}

struct pair quad(float a, float b, float c, float d) {
    struct pair p;
    p.x = (double)a + (double)b;
    p.y = (double)c + (double)d;
    return p;
}

int main(void) {
    struct pair a = mk(1.5, 2.5);
    if (a.x != 1.5 || a.y != 2.5) return 1;

    struct pair b = swap(&a);
    if (b.x != 2.5 || b.y != 1.5) return 2;

    struct mixed m = mk_mixed(42, 3.25);
    if (m.tag != 42 || m.val != 3.25) return 3;

    struct pair q = quad(1.0f, 2.0f, 3.0f, 4.0f);
    if (q.x != 3.0 || q.y != 7.0) return 4;

    /* Chained through a call result (xmm0/xmm1 must round-trip a call). */
    struct pair c = swap(&b);
    if (c.x != 1.5 || c.y != 2.5) return 5;
    return 0;
}
