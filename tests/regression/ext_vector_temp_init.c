/* A14-13 companion: ext-vector *temporary* initialization.
 *
 * A lane-wise vector temporary is lowered as an alloca followed by one store
 * per lane. That defines the whole object only when (a) no lane store is a
 * read-modify-write and (b) the lanes cover every byte of the object. The two
 * shapes that break it are exercised here:
 *
 *   1. bit-packed bool ext-vectors: a lane store keeps the neighbouring lanes
 *      of the shared byte (load/and/or), so the byte must be defined first;
 *   2. padded ext-vectors: `u8x3` is three lanes in a four-byte object, and the
 *      fourth byte is observable through `sizeof` and through whole-object
 *      copies.
 *
 * The other direction matters just as much: clearing a temporary whose lanes do
 * cover it exactly is dead stores, and those inflate the IR enough to change
 * inlining decisions in real kernels. `v2df`/`i32x2` below are the shapes that
 * must stay uncovered; their bytes are checked too, so a lowering that drops
 * the clear where it IS needed fails here. A short initializer list is one such
 * case: C (C17 6.7.9p21) requires the remaining lanes to be zero, and no store
 * touches them.
 *
 * Padding of an object with automatic storage duration is INDETERMINATE in C,
 * and Clang leaves it that way (measured: i386 -O0 emits 0xf6/0xf7 for the
 * fourth byte of a local `u8x3`). LCCC clears it deterministically, which this
 * test locks down, but only under -DLCCC_PADDING_DETERMINISTIC=1: the
 * unguarded build is the one compared against the Clang oracle, and it checks
 * logical lanes only.
 */
#include <stdio.h>
#include <string.h>

#ifndef LCCC_PADDING_DETERMINISTIC
#define LCCC_PADDING_DETERMINISTIC 0
#endif

typedef double v2df __attribute__((ext_vector_type(2)));
typedef int i32x2 __attribute__((ext_vector_type(2)));
typedef unsigned char u8x3 __attribute__((ext_vector_type(3)));
typedef _Bool b4 __attribute__((ext_vector_type(4)));

static int failures = 0;

static void check_bytes(const char *what, const void *obj, size_t n, const char *expect) {
    const unsigned char *p = (const unsigned char *)obj;
    char got[160];
    size_t off = 0;
    for (size_t i = 0; i < n && off + 3 < sizeof got; i++) {
        off += (size_t)snprintf(got + off, sizeof got - off, "%02x", p[i]);
    }
    if (strcmp(got, expect) != 0) {
        printf("FAIL %s: bytes %s, expected %s\n", what, got, expect);
        failures++;
    } else {
        printf("ok   %s = %s\n", what, got);
    }
}

static void check_int(const char *what, long long got, long long expect) {
    if (got != expect) {
        printf("FAIL %s: %lld, expected %lld\n", what, got, expect);
        failures++;
    }
}

/* The byte(s) past the last lane. Only LCCC defines them, so the assertion is
 * compiled in only for the LCCC build; the oracle build must not assume it. */
static void check_padding(const char *what, const void *obj, size_t obj_size, size_t lane_bytes) {
#if LCCC_PADDING_DETERMINISTIC
    const unsigned char *p = (const unsigned char *)obj;
    char got[160];
    size_t off = 0;
    for (size_t i = lane_bytes; i < obj_size && off + 3 < sizeof got; i++) {
        off += (size_t)snprintf(got + off, sizeof got - off, "%02x", p[i]);
    }
    if (off != 0 && strcmp(got, "00") != 0) {
        printf("FAIL %s: padding %s, expected 00\n", what, got);
        failures++;
    } else {
        printf("ok   %s padding = %s\n", what, off == 0 ? "(none)" : got);
    }
#else
    (void)what;
    (void)obj;
    (void)obj_size;
    (void)lane_bytes;
#endif
}

int main(void) {
    /* Partial initializer, no padding, no RMW: the missing lane must read 0
     * even though no store writes it. */
    v2df a = { 1.0 };
    check_int("v2df partial lane 1", (long long)a[1], 0);
    check_bytes("v2df partial object", &a, sizeof a, "000000000000f03f0000000000000000");
    check_padding("v2df partial object", &a, sizeof a, sizeof a);

    v2df b = { 3.0, 4.0 };
    v2df c = a + b;
    check_bytes("v2df binary-op temporary", &c, sizeof c, "00000000000010400000000000001040");
    check_padding("v2df binary-op temporary", &c, sizeof c, sizeof c);

    /* Rvalue operands: both operands and the result live in temporaries. */
    v2df z = (v2df){ 2.0, 3.0 } * (v2df){ 4.0, 5.0 };
    check_bytes("v2df compound-literal product", &z, sizeof z, "00000000000020400000000000002e40");

    /* Short initializer list, no padding: the object is covered by the lanes,
     * but lane 1 is still only defined by the C-mandated zero fill. */
    i32x2 p = { 5 };
    check_int("i32x2 partial lane 1", p[1], 0);
    i32x2 q = { 6, 7 };
    i32x2 r = p * q;
    check_int("i32x2 product lane 0", r[0], 30);
    check_int("i32x2 product lane 1", r[1], 0);
    check_bytes("i32x2 partial object", &p, sizeof p, "0500000000000000");
    check_padding("i32x2 partial object", &p, sizeof p, sizeof p);

    /* Padding: three lanes in a four-byte object. The lanes are checked
     * everywhere; the fourth byte only in the LCCC build. */
    u8x3 u = { 1, 2, 3 };
    check_bytes("u8x3 initializer lanes", &u, 3, "010203");
    check_padding("u8x3 initializer", &u, sizeof u, 3);
    u8x3 v = { 9, 8, 7 };
    u8x3 w = u + v;
    check_bytes("u8x3 binary-op temporary lanes", &w, 3, "0a0a0a");
    check_padding("u8x3 binary-op temporary", &w, sizeof w, 3);
    u8x3 zz = (u8x3){ 1, 1, 1 } + (u8x3){ 2, 3, 4 };
    check_bytes("u8x3 compound-literal sum lanes", &zz, 3, "030405");
    check_padding("u8x3 compound-literal sum", &zz, sizeof zz, 3);

    /* Bit-packed bool lanes: every lane store is a read-modify-write on a
     * shared byte, so the byte must start defined. Both a full and a partial
     * (C-mandated zero tail) initializer are checked, plus a computed value. */
    b4 t = { 1, 0, 1, 0 };
    check_bytes("b4 initializer", &t, sizeof t, "05");
    b4 f = { 1 };
    check_bytes("b4 partial initializer", &f, sizeof f, "01");
    b4 g = t | f;
    check_bytes("b4 computed", &g, sizeof g, "05");
    check_int("b4 lane readback 0", g[0], 1);
    check_int("b4 lane readback 1", g[1], 0);
    check_int("b4 lane readback 2", g[2], 1);
    check_int("b4 lane readback 3", g[3], 0);

    if (failures != 0) {
        printf("ext-vector temporary initialization: %d failure(s)\n", failures);
        return 1;
    }
    printf("ext-vector temporary initialization: ok\n");
    return 0;
}
