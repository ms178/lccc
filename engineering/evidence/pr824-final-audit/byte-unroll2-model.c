/* Portable little-endian byte assembly, the shape Linux spells as
 * get_unaligned_le16/32/64 and Expat as SIP_U8TO64_LE: read an unaligned
 * integer field one byte at a time so the access is alignment-agnostic.
 *
 * This is a *codegen* kernel.  Every compiler in the oracle list folds the
 * chains back into one load; before src/passes/byte_load.rs LCCC did not, and
 * the difference is visible both statically (instruction count) and
 * dynamically (the record-decoding loop below, which reads 4096 length fields
 * per round out of a byte buffer).
 *
 * get_unaligned_le32(p) == 0x04030201 for p = 01 02 03 04, checked below, so a
 * compiler that mis-folds a chain fails the run, not just the timing.
 */
#include <stdio.h>
#include <stdint.h>

static uint16_t le16(const unsigned char *p) {
    return (uint16_t)((uint16_t)p[0] | ((uint16_t)p[1] << 8));
}

static uint32_t le32(const unsigned char *p) {
    return (uint32_t)p[0] | ((uint32_t)p[1] << 8) |
           ((uint32_t)p[2] << 16) | ((uint32_t)p[3] << 24);
}

static uint64_t le64(const unsigned char *p) {
    return (uint64_t)p[0] | ((uint64_t)p[1] << 8) | ((uint64_t)p[2] << 16) |
           ((uint64_t)p[3] << 24) | ((uint64_t)p[4] << 32) |
           ((uint64_t)p[5] << 40) | ((uint64_t)p[6] << 48) |
           ((uint64_t)p[7] << 56);
}

/* Big-endian twin: must NOT be turned into a plain load (it is a byte swap).
 * Keeping it in the same translation unit is the differential: the two
 * functions must stay distinguishable. */
static uint64_t be64(const unsigned char *p) {
    return ((uint64_t)p[0] << 56) | ((uint64_t)p[1] << 48) |
           ((uint64_t)p[2] << 40) | ((uint64_t)p[3] << 32) |
           ((uint64_t)p[4] << 24) | ((uint64_t)p[5] << 16) |
           ((uint64_t)p[6] << 8) | (uint64_t)p[7];
}

/* Struct-member run: the SipHash shape, bytes at a nonzero object offset. */
struct framed {
    unsigned char head[8];
    unsigned char body[8];
    uint32_t tail;
};

static uint64_t body_le64(const struct framed *f) {
    const unsigned char *p = f->body;
    return (uint64_t)p[0] | ((uint64_t)p[1] << 8) | ((uint64_t)p[2] << 16) |
           ((uint64_t)p[3] << 24) | ((uint64_t)p[4] << 32) |
           ((uint64_t)p[5] << 40) | ((uint64_t)p[6] << 48) |
           ((uint64_t)p[7] << 56);
}

#define ROUNDS 20000

int main(void) {
    static unsigned char buf[4096];
    static struct framed fr;
    unsigned long i, r;
    uint32_t acc = 0;
    uint64_t acc64 = 0;

    for (i = 0; i < sizeof buf; i++)
        buf[i] = (unsigned char)(i * 37 + 11);
    for (i = 0; i < sizeof fr.head; i++) {
        fr.head[i] = (unsigned char)(i + 1);
        fr.body[i] = (unsigned char)(0xF0 + i);
    }
    fr.tail = 0x11223344u;

    /* Known answers. */
    buf[0] = 0x01; buf[1] = 0x02; buf[2] = 0x03; buf[3] = 0x04;
    buf[4] = 0x05; buf[5] = 0x06; buf[6] = 0x07; buf[7] = 0x08;
    if (le16(buf) != 0x0201u)
        return 2;
    if (le32(buf) != 0x04030201u)
        return 3;
    if (le64(buf) != 0x0807060504030201ull)
        return 4;
    if (be64(buf) != 0x0102030405060708ull)
        return 5;
    if (body_le64(&fr) != 0xF7F6F5F4F3F2F1F0ull)
        return 6;
    for (r = 0; r < ROUNDS; r++) {
        for (i = 0; (i + 16) <= sizeof buf; i += 16) {
            acc += le32(buf + i) ^ (uint32_t)i;
            acc += le32(buf + i + 8) ^ (uint32_t)(i + 8);
        }
        acc64 += body_le64(&fr);
    }
    printf("%08x %016llx\n", acc, (unsigned long long)acc64);
    return 0;
}
