/* IR-verifier fixture: bit-field record layout under every mode lccc supports.
 *
 * The verifier gate compiles each fixture at -O2 with CCC_VERIFY_IR=abort and
 * requires a clean exit, so this file must be well defined C with no layout
 * assertions: it exists to push bit-field extraction, insertion, zero-width
 * units, unions, packed records and cross-unit accesses through the pass
 * pipeline, not to pin a layout number.
 */
struct Flags {
    unsigned a : 1;
    unsigned b : 3;
    unsigned c : 12;
    unsigned d : 16;
};

struct Mixed {
    char tag;
    unsigned lo : 4;
    unsigned hi : 4;
    short s : 9;
    short t : 7;
    int tail;
};

struct WithZero {
    int a : 5;
    int : 0;
    int b : 5;
    char c;
};

struct __attribute__((packed)) PackedBits {
    char c;
    unsigned x : 3;
    unsigned y : 13;
    short z;
};

union Bits {
    struct Flags f;
    unsigned raw;
};

static struct Flags flip(struct Flags in, unsigned bits) {
    struct Flags out = in;
    out.a ^= bits & 1u;
    out.b = (unsigned)(out.b + 1u) & 7u;
    out.c = (unsigned)(in.c ^ (bits << 2)) & 0xfffu;
    out.d = (unsigned)(in.d + bits) & 0xffffu;
    return out;
}

static unsigned round_trip(struct Mixed *m, unsigned seed) {
    m->tag = (char)(seed & 0x7f);
    m->lo = seed & 0xfu;
    m->hi = (seed >> 4) & 0xfu;
    m->s = (short)((int)(seed & 0xffu) - 128);
    m->t = (short)((int)((seed >> 8) & 0x3fu) - 32);
    m->tail = (int)seed;
    return (unsigned)m->lo + (unsigned)m->hi + (unsigned)m->tail
         + (unsigned)(m->s + m->t);
}

static int zero_width(const struct WithZero *w) {
    struct WithZero copy = *w;
    copy.b = (int)(copy.a + 3);
    return copy.b + copy.c;
}

static unsigned packed_bits(struct PackedBits *p, unsigned v) {
    p->x = v & 7u;
    p->y = (v >> 3) & 0x1fffu;
    p->z = (short)(p->x + p->y);
    return (unsigned)p->z + (unsigned)(unsigned char)p->c;
}

int main(void) {
    struct Flags f = {0, 0, 0, 0};
    struct Mixed m;
    struct WithZero w;
    struct PackedBits p;
    union Bits u;

    f = flip(f, 0x9u);
    u.f = f;
    f = flip(u.f, u.raw);

    w.a = 7;
    w.b = 0;
    w.c = 3;

    p.c = (char)1;
    p.x = 0;
    p.y = 0;
    p.z = 0;

    int acc = zero_width(&w);
    acc += (int)round_trip(&m, 0x1234u);
    acc += (int)packed_bits(&p, 0x5a5au);
    acc += (int)f.a + (int)f.b + (int)f.c + (int)f.d;
    return acc & 0x7f;
}
