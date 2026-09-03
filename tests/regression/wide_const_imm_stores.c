/* Narrow stores of wide unsigned constants must write exactly the store
 * width, both for register-homed pointers, folded-GEP offsets and global
 * symbols — and the value must land bit-exact when the constant sits above
 * i32::MAX (e.g. 3041712678u). Regression for the PR #363/#364 immediate
 * store width work; the codegen gate lives in check_wide_const_imm_stores.sh.
 * This file is differential (run under lccc and gcc, compare stdout). */
#include <stdio.h>
#include <stdint.h>

unsigned ga[8];
int8_t gb[8];
int16_t gc[8];
uint64_t gd[8];
unsigned ge;
uint8_t gu8[4];
uint16_t gu16[4];

void store_unsigned_narrow(void) {
    gu8[0] = 255u;
    gu8[1] = 128u;
    gu16[0] = 65535u;
    gu16[1] = 0x8000u;
}

void store_p(unsigned *p) {
    p[0] = 0xFFFFFFFFu;
    p[1] = 3041712678u;
    p[2] = 2147483648u;
    p[3] = 1u;
}

void store_glob(void) {
    ga[0] = 0xFFFFFFFFu;
    ga[1] = 3041712678u;
    ga[2] = 0;
    ga[3] = 3041712679u;
    ge = 4294967295u;
}

void store_p8(int8_t *p) { p[0] = -1; p[1] = 127; p[2] = -128; p[3] = 0x80; }
void store_p16(int16_t *p) { p[0] = -1; p[1] = 32767; p[2] = 65535; }
void store_p64(uint64_t *p) {
    p[0] = 0xFFFFFFFFFFFFFFFFull;
    p[1] = 0x8000000000000000ull;
    p[2] = 3041712678u;
    p[3] = 0x123456789ABCDEF0ull;
}

int main(void) {
    unsigned q[4];
    store_p(q);
    store_glob();
    store_unsigned_narrow();
    int8_t b[4];
    store_p8(b);
    int16_t c[3];
    store_p16(c);
    uint64_t d[4];
    store_p64(d);
    printf("%u %u %u %u\n", q[0], q[1], q[2], q[3]);
    printf("%u %u %u %u %u\n", ga[0], ga[1], ga[2], ga[3], ge);
    printf("%d %d %d %d\n", b[0], b[1], b[2], b[3]);
    printf("%d %d %d\n", c[0], c[1], c[2]);
    printf("%llu %llu %llu %llu\n",
           (unsigned long long)d[0], (unsigned long long)d[1],
           (unsigned long long)d[2], (unsigned long long)d[3]);
    printf("%u %u %u %u\n", gu8[0], gu8[1], gu16[0], gu16[1]);
    return 0;
}
