/*
 * Byte map vectorizer: same-size signedness casts inside the lane tree.
 *
 * lccc lowers `(signed char)p[i]` over an `unsigned char` stream to ONE
 * same-size `Cast U8 -> I8`.  The byte-lane parser treated every
 * non-narrowing cast as interval-preserving, so the operand kept the range
 * [0, 255], the compare arm chose the UNSIGNED byte predicate, and
 * `(signed char)p[i] < 0` came out false for every byte >= 0x80 (128 wrong
 * lanes per 256 bytes, both at -O2 and -march=x86-64-v3).  The mirror image
 * (`(unsigned char)sp[i]` over a `signed char` stream) is covered too, plus
 * a widening of a reinterpreted value and a compare against a negative
 * constant.  Every byte value at every lane position, all lengths 0..96.
 */
#include <stdio.h>
#include <string.h>

#define N 96

__attribute__((noinline)) void m_sneg(unsigned char *restrict o, const unsigned char *restrict p, int n) {
    for (int i = 0; i < n; i++) o[i] = ((signed char)p[i] < 0) ? 1 : 2;
}
__attribute__((noinline)) void m_sgt(unsigned char *restrict o, const unsigned char *restrict p, int n) {
    for (int i = 0; i < n; i++) o[i] = ((signed char)p[i] > -3) ? p[i] : 9;
}
__attribute__((noinline)) void m_ugt(unsigned char *restrict o, const signed char *restrict p, int n) {
    for (int i = 0; i < n; i++) o[i] = ((unsigned char)p[i] > 200) ? 5 : 6;
}
__attribute__((noinline)) void m_widen(unsigned char *restrict o, const unsigned char *restrict p, int n) {
    for (int i = 0; i < n; i++) o[i] = ((int)(signed char)p[i] < -100) ? 3 : 4;
}

int main(void) {
    unsigned char src[N], out[N];
    unsigned long sum = 0;
    int bad = 0;
    for (int base = 0; base < 256; base += 7) {
        for (int i = 0; i < N; i++) src[i] = (unsigned char)(base + i * 37);
        for (int n = 0; n <= N; n++) {
            memset(out, 0xEE, sizeof out);
            m_sneg(out, src, n);
            for (int i = 0; i < N; i++) {
                volatile signed char v = (signed char)src[i];
                int want = i < n ? (v < 0 ? 1 : 2) : 0xEE;
                bad += out[i] != want;
                sum = sum * 31 + out[i];
            }
            memset(out, 0xEE, sizeof out);
            m_sgt(out, src, n);
            for (int i = 0; i < N; i++) {
                volatile signed char v = (signed char)src[i];
                int want = i < n ? (v > -3 ? src[i] : 9) : 0xEE;
                bad += out[i] != want;
                sum = sum * 31 + out[i];
            }
            memset(out, 0xEE, sizeof out);
            m_ugt(out, (const signed char *)src, n);
            for (int i = 0; i < N; i++) {
                volatile unsigned char v = src[i];
                int want = i < n ? (v > 200 ? 5 : 6) : 0xEE;
                bad += out[i] != want;
                sum = sum * 31 + out[i];
            }
            memset(out, 0xEE, sizeof out);
            m_widen(out, src, n);
            for (int i = 0; i < N; i++) {
                volatile signed char v = (signed char)src[i];
                int want = i < n ? ((int)v < -100 ? 3 : 4) : 0xEE;
                bad += out[i] != want;
                sum = sum * 31 + out[i];
            }
        }
    }
    printf("checksum %lu\n", sum);
    if (bad) {
        printf("%d wrong bytes\n", bad);
        return 1;
    }
    printf("OK vec_byte_map_reinterpret\n");
    return 0;
}
