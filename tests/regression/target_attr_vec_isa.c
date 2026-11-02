/* Per-function `__attribute__((target(...)))` must pick the vectorizer's
 * register width; the TU's `-march` must not be the only thing that can.
 *
 * `slp_vectorizer::family_for` gates every double-width arm on AVX2:
 *
 *     (I16, 16) if avx2   vs  (I16, 8)  the widest ungated arm
 *     (I8,  32) if avx2   vs  (I8, 16)  the widest ungated arm
 *
 * (and likewise I32 4->8, I64 2->4, F32 4->8, F64 2->4), so the ISA the pass
 * sees is exactly what separates one ymm op from two xmm ones.  That ISA used
 * to be the TU's for the BB-SLP, slp_late and vec_arx entry points even though
 * six sibling vector entry points already folded the function's own target
 * features, so a `target("avx2")` function in a baseline TU was silently
 * capped at 128-bit - and `vec_ssa`'s promotion table, built once per module,
 * had no ymm families to promote it with either.
 *
 * Every pair below has a byte-identical body (one macro) and differs only in
 * the attribute, so nothing but the per-function ISA fold can separate their
 * codegen.  The 20-lane u16 and 34-lane u8 runs are the shapes
 * bb_slp_redteam.c already proves BB-SLP seeds at -O2.
 *
 * Part of tests/regression/check_target_attr_vec_isa.sh.
 */
#include <stdio.h>
#include <stdint.h>

typedef struct { uint16_t h[20]; } H20;
typedef struct { uint8_t  b[34]; } B34;

#define RUN_H20(o, i) \
    (o)->h[0 ] = (i)->h[0 ] + 1; \
    (o)->h[1 ] = (i)->h[1 ] + 1; \
    (o)->h[2 ] = (i)->h[2 ] + 1; \
    (o)->h[3 ] = (i)->h[3 ] + 1; \
    (o)->h[4 ] = (i)->h[4 ] + 1; \
    (o)->h[5 ] = (i)->h[5 ] + 1; \
    (o)->h[6 ] = (i)->h[6 ] + 1; \
    (o)->h[7 ] = (i)->h[7 ] + 1; \
    (o)->h[8 ] = (i)->h[8 ] + 1; \
    (o)->h[9 ] = (i)->h[9 ] + 1; \
    (o)->h[10] = (i)->h[10] + 1; \
    (o)->h[11] = (i)->h[11] + 1; \
    (o)->h[12] = (i)->h[12] + 1; \
    (o)->h[13] = (i)->h[13] + 1; \
    (o)->h[14] = (i)->h[14] + 1; \
    (o)->h[15] = (i)->h[15] + 1; \
    (o)->h[16] = (i)->h[16] + 1; \
    (o)->h[17] = (i)->h[17] + 1; \
    (o)->h[18] = (i)->h[18] + 1; \
    (o)->h[19] = (i)->h[19] + 1;

#define RUN_B34(o, i) \
    (o)->b[0 ] = (i)->b[0 ] + 1; \
    (o)->b[1 ] = (i)->b[1 ] + 1; \
    (o)->b[2 ] = (i)->b[2 ] + 1; \
    (o)->b[3 ] = (i)->b[3 ] + 1; \
    (o)->b[4 ] = (i)->b[4 ] + 1; \
    (o)->b[5 ] = (i)->b[5 ] + 1; \
    (o)->b[6 ] = (i)->b[6 ] + 1; \
    (o)->b[7 ] = (i)->b[7 ] + 1; \
    (o)->b[8 ] = (i)->b[8 ] + 1; \
    (o)->b[9 ] = (i)->b[9 ] + 1; \
    (o)->b[10] = (i)->b[10] + 1; \
    (o)->b[11] = (i)->b[11] + 1; \
    (o)->b[12] = (i)->b[12] + 1; \
    (o)->b[13] = (i)->b[13] + 1; \
    (o)->b[14] = (i)->b[14] + 1; \
    (o)->b[15] = (i)->b[15] + 1; \
    (o)->b[16] = (i)->b[16] + 1; \
    (o)->b[17] = (i)->b[17] + 1; \
    (o)->b[18] = (i)->b[18] + 1; \
    (o)->b[19] = (i)->b[19] + 1; \
    (o)->b[20] = (i)->b[20] + 1; \
    (o)->b[21] = (i)->b[21] + 1; \
    (o)->b[22] = (i)->b[22] + 1; \
    (o)->b[23] = (i)->b[23] + 1; \
    (o)->b[24] = (i)->b[24] + 1; \
    (o)->b[25] = (i)->b[25] + 1; \
    (o)->b[26] = (i)->b[26] + 1; \
    (o)->b[27] = (i)->b[27] + 1; \
    (o)->b[28] = (i)->b[28] + 1; \
    (o)->b[29] = (i)->b[29] + 1; \
    (o)->b[30] = (i)->b[30] + 1; \
    (o)->b[31] = (i)->b[31] + 1; \
    (o)->b[32] = (i)->b[32] + 1; \
    (o)->b[33] = (i)->b[33] + 1;

/* AVX2 requested per function: the 20-lane run packs as I16x16 (one ymm). */
__attribute__((target("avx2")))
void tgt_h20(H20 *restrict o, const H20 *restrict i) { RUN_H20(o, i) }

/* No attribute: the widest family is I16x8, so this stays 128-bit. */
void base_h20(H20 *restrict o, const H20 *restrict i) { RUN_H20(o, i) }

/* Same differential one lane width down: I8x32 (ymm) vs I8x16 (xmm). */
__attribute__((target("avx2")))
void tgt_b34(B34 *restrict o, const B34 *restrict i) { RUN_B34(o, i) }

void base_b34(B34 *restrict o, const B34 *restrict i) { RUN_B34(o, i) }

int main(void) {
    H20 a, r1, r2;
    B34 c, s1, s2;
    int fails = 0;
    int k;
    for (k = 0; k < 20; k++) a.h[k] = (uint16_t)(k * 3);
    for (k = 0; k < 34; k++) c.b[k] = (uint8_t)(k * 5);

    tgt_h20(&r1, &a);
    base_h20(&r2, &a);
    tgt_b34(&s1, &c);
    base_b34(&s2, &c);

    for (k = 0; k < 20; k++) {
        uint16_t want = (uint16_t)(a.h[k] + 1);
        if (r1.h[k] != want) fails++;
        if (r2.h[k] != want) fails++;
        if (r1.h[k] != r2.h[k]) fails++;   /* same body, same answer */
    }
    for (k = 0; k < 34; k++) {
        uint8_t want = (uint8_t)(c.b[k] + 1);
        if (s1.b[k] != want) fails++;
        if (s2.b[k] != want) fails++;
        if (s1.b[k] != s2.b[k]) fails++;
    }
    printf("target_attr_vec_isa: all pass (%d fails)\n", fails);
    return fails != 0;
}
