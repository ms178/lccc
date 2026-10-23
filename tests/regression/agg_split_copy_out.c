/* aggregate_sroa form 4, whole-object copy-OUT: a local array or struct is
 * filled by memcpy from caller memory, worked on with constant subscripts, and
 * written back by memcpy (zstd's HUF_DecompressFastArgs loop shape). The split
 * turns the copy-out into one store per field, and bytes that no access names
 * travel in gap carriers; every destination byte must still be written.
 * Checked against GCC at run time. */
#include <stdint.h>
#include <stdio.h>
#include <string.h>

struct fast_args {
    uint64_t bits[4];
    const uint8_t *ip[4];
};

/* All four lanes are read and written: every field is named. */
__attribute__((noinline)) static void lanes(struct fast_args *a, int rounds) {
    uint64_t bits[4];
    const uint8_t *ip[4];
    memcpy(bits, a->bits, sizeof bits);
    memcpy((void *)ip, a->ip, sizeof ip);
    for (int r = 0; r < rounds; r++) {
        bits[0] = (bits[0] << 3) ^ (uint64_t)(*ip[0]++);
        bits[1] = (bits[1] << 5) ^ (uint64_t)(*ip[1]++);
        bits[2] = (bits[2] << 7) ^ (uint64_t)(*ip[2]++);
        bits[3] = (bits[3] << 1) ^ (uint64_t)(*ip[3]++);
    }
    memcpy(a->bits, bits, sizeof bits);
    memcpy(a->ip, (void *)ip, sizeof ip);
}

/* Only one element is touched: the other three travel in carriers. */
__attribute__((noinline)) static void one_of_four(uint32_t *dst, const uint32_t *src, uint32_t k) {
    uint32_t v[4];
    memcpy(v, src, sizeof v);
    v[2] = v[2] * k + 1;
    memcpy(dst, v, sizeof v);
}

/* Uneven layout: a 2-byte field at offset 2 leaves gaps of 2, 4 and 8. */
struct mixed {
    uint16_t a;
    uint16_t b;
    uint32_t c;
    uint64_t d;
};

__attribute__((noinline)) static void bump_b(struct mixed *out, const struct mixed *in) {
    struct mixed m;
    memcpy(&m, in, sizeof m);
    m.b = (uint16_t)(m.b + 0x1111);
    memcpy(out, &m, sizeof m);
}

int main(void) {
    static uint8_t buf[4][64];
    for (int s = 0; s < 4; s++)
        for (int i = 0; i < 64; i++)
            buf[s][i] = (uint8_t)(i * 37 + s * 11 + 5);
    struct fast_args a;
    for (int s = 0; s < 4; s++) {
        a.bits[s] = 0x0123456789ABCDEFull >> s;
        a.ip[s] = buf[s];
    }
    lanes(&a, 40);
    for (int s = 0; s < 4; s++)
        printf("lane %d bits=%016llx ip+%d\n", s, (unsigned long long)a.bits[s],
               (int)(a.ip[s] - buf[s]));

    uint32_t src[4] = {0xDEADBEEFu, 0x01020304u, 7u, 0xCAFEF00Du};
    uint32_t dst[4] = {0, 0, 0, 0};
    one_of_four(dst, src, 9);
    printf("v = %08x %08x %08x %08x\n", dst[0], dst[1], dst[2], dst[3]);

    struct mixed in = {0xA1A2, 0xB1B2, 0xC1C2C3C4u, 0xD1D2D3D4D5D6D7D8ull};
    struct mixed out;
    memset(&out, 0x5A, sizeof out);
    bump_b(&out, &in);
    printf("m = %04x %04x %08x %016llx\n", out.a, out.b, out.c, (unsigned long long)out.d);
    return 0;
}
