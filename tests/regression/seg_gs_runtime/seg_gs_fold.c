/* Runtime, self-checking contract for constant-displacement folding of
 * __seg_gs loads and stores.
 *
 * A __seg_gs access whose address is a GEP with a constant offset (a struct
 * member, an array element at a constant index, a `p + k` pointer) is emitted
 * as one `%gs:disp(%base)` operand. This program forces every integer width
 * through that form, with constant and register sources, with sign- and
 * zero-extending loads, with read-modify-write, and with the base in a
 * register reached through a global segment pointer. Each value is checked
 * against the same bytes read through an ordinary pointer to the same area,
 * so the verdict does not depend on any other compiler.
 *
 * The GS base is set to `area`; a segment pointer with value `off` addresses
 * area[off]. Every check prints "ok <name>" or "FAIL <name>". The program
 * prints "PASSED (0 failures)" and exits 0 only when all checks pass.
 */
#include <stdio.h>
#include <sys/syscall.h>
#include <unistd.h>

#define ARCH_SET_GS 0x1001
#define AREA 4096
#define MIX_OFF 256

struct mix {
    char c;
    unsigned char uc;
    short s;
    unsigned short us;
    int i;
    unsigned int ui;
    long l;
    unsigned long ul;
    char *p;
};

static unsigned char area[AREA] __attribute__((aligned(64)));
static int failures;

#define CHECK(cond, name)                 \
    do {                                  \
        if (cond) {                       \
            printf("ok %s\n", name);      \
        } else {                          \
            printf("FAIL %s\n", name);    \
            failures++;                   \
        }                                 \
    } while (0)

/* Global segment pointers: the base comes from memory, not from a parameter. */
static struct mix __seg_gs *GM = (struct mix __seg_gs *)MIX_OFF;
static long __seg_gs *GA = (long __seg_gs *)0;
static unsigned short __seg_gs *GS = (unsigned short __seg_gs *)512;

/* Constant stores at constant member offsets. */
__attribute__((noinline)) static void store_consts(struct mix __seg_gs *m) {
    m->c = -3;
    m->uc = 250;
    m->s = -1000;
    m->us = 60000;
    m->i = -70000;
    m->ui = 4000000000u;
    m->l = -5;
    m->ul = 1ul << 40;
    m->p = (char *)0x1234;
}

/* Register-valued stores at the same offsets. */
__attribute__((noinline)) static void store_vars(struct mix __seg_gs *m, long v) {
    m->c = (char)v;
    m->uc = (unsigned char)(v + 1);
    m->s = (short)(v * 3);
    m->us = (unsigned short)(v * 5);
    m->i = (int)(v * 7);
    m->ui = (unsigned int)(v * 11);
    m->l = v * 13;
    m->ul = (unsigned long)v * 17;
}

/* Sign- and zero-extending loads at constant offsets, combined so every
 * extension matters: the result differs if any width or extension is wrong. */
__attribute__((noinline)) static long load_mix(struct mix __seg_gs *m) {
    long r = 0;
    r += m->c;
    r = r * 3 + m->uc;
    r = r * 3 + m->s;
    r = r * 3 + m->us;
    r = r * 3 + m->i;
    r = r * 3 + (long)m->ui;
    r = r * 3 + m->l;
    r = r * 3 + (long)m->ul;
    return r;
}

/* Read-modify-write at constant offsets. */
__attribute__((noinline)) static void rmw_mix(struct mix __seg_gs *m) {
    m->c += 9;
    m->uc -= 3;
    m->s ^= 0x55;
    m->us |= 0x8001;
    m->i *= 2;
    m->ui >>= 1;
    m->l -= 100;
    m->ul &= ~0xffUL;
}

/* Array elements at constant indices, through a pointer parameter. */
__attribute__((noinline)) static long arr_sum(long __seg_gs *a) {
    long s = 0;
    for (int k = 0; k < 4; k++) s += a[k];
    a[5] = s;
    a[6] = a[0] ^ a[3];
    return s + a[5] + a[6];
}

/* Store of a value that is itself a loaded segment value (value in a
 * register, base in another register). */
__attribute__((noinline)) static void copy_field(struct mix __seg_gs *m) {
    m->ui = m->i;
    m->us = m->s;
    m->l = m->ui;
}

/* Base kept live across a call so it sits in a callee-saved register, plus
 * enough live values to force register pressure. */
__attribute__((noinline)) static long pressure(struct mix __seg_gs *m, long a, long b, long c,
                                                long d, long e, long f) {
    long keep = a * b + c;
    m->i = (int)(keep + d);
    long got = m->i;
    m->us = (unsigned short)(e + f);
    long back = m->us;
    return keep + got + back + a + b + c + d + e + f;
}

int main(void) {
    if (syscall(SYS_arch_prctl, ARCH_SET_GS, area) != 0) {
        perror("arch_prctl");
        return 2;
    }
    setvbuf(stdout, NULL, _IONBF, 0);

    struct mix *g = (struct mix *)(area + MIX_OFF);
    struct mix __seg_gs *m = (struct mix __seg_gs *)MIX_OFF;

    /* Constant stores, read back through generic memory. */
    store_consts(m);
    CHECK(g->c == -3, "const_store_i8");
    CHECK(g->uc == 250, "const_store_u8");
    CHECK(g->s == -1000, "const_store_i16");
    CHECK(g->us == 60000, "const_store_u16");
    CHECK(g->i == -70000, "const_store_i32");
    CHECK(g->ui == 4000000000u, "const_store_u32");
    CHECK(g->l == -5, "const_store_i64");
    CHECK(g->ul == (1ul << 40), "const_store_u64");
    CHECK(g->p == (char *)0x1234, "const_store_ptr");

    /* Constant loads with sign and zero extension, against generic loads. */
    {
        long want = 0;
        want += g->c;
        want = want * 3 + g->uc;
        want = want * 3 + g->s;
        want = want * 3 + g->us;
        want = want * 3 + g->i;
        want = want * 3 + (long)g->ui;
        want = want * 3 + g->l;
        want = want * 3 + (long)g->ul;
        CHECK(load_mix(m) == want, "const_load_extend");
    }

    /* Register-valued stores, each width. */
    store_vars(m, -7);
    CHECK(g->c == (char)-7, "var_store_i8");
    CHECK(g->uc == (unsigned char)-6, "var_store_u8");
    CHECK(g->s == (short)-21, "var_store_i16");
    CHECK(g->us == (unsigned short)-35, "var_store_u16");
    CHECK(g->i == -49, "var_store_i32");
    CHECK(g->ui == (unsigned int)-77, "var_store_u32");
    CHECK(g->l == -91, "var_store_i64");
    CHECK(g->ul == (unsigned long)-7 * 17, "var_store_u64");

    /* Read-modify-write, against the same operations on generic memory. */
    store_consts(m);
    rmw_mix(m);
    CHECK(g->c == (char)(-3 + 9), "rmw_i8");
    CHECK(g->uc == (unsigned char)(250 - 3), "rmw_u8");
    CHECK(g->s == (short)(-1000 ^ 0x55), "rmw_i16");
    CHECK(g->us == (unsigned short)(60000 | 0x8001), "rmw_u16");
    CHECK(g->i == -140000, "rmw_i32");
    CHECK(g->ui == 2000000000u, "rmw_u32");
    CHECK(g->l == -105, "rmw_i64");
    CHECK(g->ul == ((1ul << 40) & ~0xffUL), "rmw_u64");

    /* Field-to-field copies: loaded value stored at another offset. */
    store_vars(m, 4);
    copy_field(m);
    CHECK(g->ui == (unsigned int)g->i, "copy_i32_to_u32");
    CHECK(g->us == (unsigned short)g->s, "copy_i16_to_u16");
    CHECK(g->l == (long)g->ui, "copy_u32_to_i64");

    /* Array elements at constant indices through a pointer parameter. */
    {
        long __seg_gs *a = (long __seg_gs *)1024;
        long *ga = (long *)(area + 1024);
        for (int k = 0; k < 7; k++) ga[k] = 100 + k * 10;
        long got = arr_sum(a);
        /* s = 100+110+120+130 = 460; a[5] = s; a[6] = a[0]^a[3]; result s + a[5] + a[6]. */
        CHECK(got == 460 + 460 + (100 ^ 130), "arr_sum_value");
        CHECK(ga[5] == 460, "arr_sum_store_const_index");
        CHECK(ga[6] == (100 ^ 130), "arr_xor_store");
    }

    /* Base from a global segment pointer, offset zero and non-zero. */
    GA[2] = 0x1122334455667788L;
    CHECK(*(long *)(area + 16) == 0x1122334455667788L, "global_base_long");
    GA[3] = GA[2] + 1;
    CHECK(*(long *)(area + 24) == 0x1122334455667789L, "global_base_rmw_source");
    GS[1] = 0xbeef;
    CHECK(*(unsigned short *)(area + 514) == 0xbeef, "global_base_u16");
    CHECK(GM->l == 28, "global_pointer_member_load");
    GM->us = 7;
    CHECK(g->us == 7, "global_pointer_member_store");

    /* Register pressure with the base reused across calls. */
    store_consts(m);
    {
        long r = pressure(m, 3, 4, 5, 6, 7, 8);
        /* keep = 3*4+5 = 17; i = 17+6 = 23; us = 7+8 = 15; r = 17+23+15+3+4+5+6+7+8. */
        CHECK(g->i == 23, "pressure_store_i32");
        CHECK(g->us == 15, "pressure_store_u16");
        CHECK(r == 88, "pressure_value");
    }

    if (failures == 0) {
        printf("PASSED (0 failures)\n");
        return 0;
    }
    printf("FAILED (%d failure%s)\n", failures, failures == 1 ? "" : "s");
    return 1;
}
