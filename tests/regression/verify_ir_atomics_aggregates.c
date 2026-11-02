/* IR-verifier fixture: atomics, aggregates and pointer punning.
 *
 * Same contract as verify_ir_bitfield_record.c: it compiles at -O2 with
 * CCC_VERIFY_IR=abort, must stay well defined, and returns 0 so the regression
 * harness can execute it. It exists to push atomic RMW, compare-and-swap,
 * aggregate moves across calls, and pointer-type punning through the pass
 * pipeline -- the shapes that rewrite memory operations.
 */
struct Pair {
    long a;
    long b;
};

struct Triple {
    int x, y, z;
};

static _Atomic int counter;
static _Atomic long wide;
static volatile int sink;

static struct Pair swap(struct Pair p) {
    struct Pair out;
    out.a = p.b + 1;
    out.b = p.a - 1;
    return out;
}

static struct Triple bump(struct Triple t) {
    t.x += t.y;
    t.y ^= t.z;
    t.z -= t.x;
    return t;
}

static int atomics(int n) {
    int seen = 0;
    for (int i = 0; i < n; i++) {
        int old = __atomic_fetch_add(&counter, 1, __ATOMIC_SEQ_CST);
        seen += old & 3;
        int expected = old + 1;
        __atomic_compare_exchange_n(&counter, &expected, old + 2, 0,
                                    __ATOMIC_SEQ_CST, __ATOMIC_SEQ_CST);
    }
    long prev = __atomic_exchange_n(&wide, (long)n, __ATOMIC_SEQ_CST);
    return seen + (int)prev;
}

static unsigned pun(const struct Triple *t) {
    const unsigned char *bytes = (const unsigned char *)t;
    unsigned sum = 0;
    for (unsigned i = 0; i < sizeof *t; i++) {
        sum = (sum << 3) ^ bytes[i];
    }
    return sum;
}

int main(void) {
    struct Pair p = {1, 2};
    struct Triple t = {3, 4, 5};
    int acc = 0;

    for (int i = 0; i < 4; i++) {
        p = swap(p);
        t = bump(t);
        acc += (int)(p.a - p.b) + t.x + t.y + t.z;
    }
    acc += atomics(8);
    acc += (int)pun(&t);
    sink = acc;
    return sink == ~0 ? 1 : 0;
}
