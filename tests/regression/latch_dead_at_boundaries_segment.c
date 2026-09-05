/* Regression: hole-aware liveness segments must cover a block in which a
 * value is (re)defined and read while it is live-in to and live-out of
 * NOTHING at that block's boundaries.
 *
 * Shape (distilled from zstd's HUF_decompress4X2_usingDTable_internal on
 * the non-BMI2 path — the code a QEMU `qemu64` guest executes when the
 * kernel decompressor runs):
 *
 *   endSignal = 1;
 *   while ((op4 < oend) & endSignal) {        // rotated: latch re-tests
 *       ...decode 4 streams (calls)...
 *       endSignal  = reload(&s1) == OK;       // phi-elim: endSignal is
 *       endSignal &= reload(&s2) == OK;       // re-DEFINED in the latch
 *       ...                                   // block by `Copy v_phi <- v_next`
 *   }                                         // and READ by the rotated test
 *
 * After phi elimination + loop rotation the latch block both defines the
 * loop-carried `endSignal` (the copy from its new value) and reads it (the
 * duplicated exit condition) — with the value dead at the block entry
 * (killed by the copy before any read) and dead at the block exit (both
 * successors either re-enter the header, which reads the *copy source*'s
 * coalesced register only through this same value... or leave the loop).
 * The boundary-only segment builder produced NO segment for that block:
 * the register allocator's segment-aware scan saw a hole exactly where the
 * copy and the compare sit and parked the `op4 < oend` zero-extension in
 * `endSignal`'s register between the two. The loop then exited after the
 * first iteration whenever the streams had bits left — every zstd block
 * that used the 4-stream Huffman literals mode produced garbage, and the
 * kernel decompressor reported "ZSTD-compressed data is corrupt".
 *
 * The reproducer keeps the calls out-of-line (so the reloads are real call
 * points and the value must be caller-save-aware), uses four independent
 * cursors (register pressure comparable to the original) and checks the
 * exact iteration count: the correct loop runs until every stream is
 * exhausted, the miscompiled one stops after one round.
 *
 * Expected output:
 *   (see the gcc oracle; the suite compares stdout with gcc)
 */
#include <stdio.h>

struct stream {
    unsigned long bits;
    const unsigned char *ptr;
    const unsigned char *start;
};

__attribute__((noinline)) static int reload(struct stream *s) {
    /* 1 while bytes remain, 0 when the stream is exhausted (the "reload
     * status" that zstd folds into endSignal). */
    if (s->ptr > s->start) {
        s->ptr -= 1;
        s->bits = (s->bits << 8) | *s->ptr;
        return 1;
    }
    return 0;
}

__attribute__((noinline)) static unsigned decode(struct stream *s, const unsigned char *tab) {
    return tab[s->bits & 15u];
}

__attribute__((noinline)) static int run(unsigned char *out, unsigned char *oend,
                                         struct stream *s1, struct stream *s2,
                                         struct stream *s3, struct stream *s4,
                                         const unsigned char *tab, int *rounds) {
    unsigned char *op1 = out;
    unsigned char *op2 = out + 50;
    unsigned char *op3 = out + 100;
    unsigned char *op4 = out + 150;
    unsigned endSignal = 1;
    int n = 0;

    /* Several short-lived temporaries in the rotated exit test: with the
     * hole in endSignal's coverage one of them lands in its register. */
    while (((op1 < oend) & (op2 < oend) & (op3 < oend) & (op4 < oend)) & endSignal) {
        *op1++ = (unsigned char)decode(s1, tab);
        *op2++ = (unsigned char)decode(s2, tab);
        *op3++ = (unsigned char)decode(s3, tab);
        *op4++ = (unsigned char)decode(s4, tab);
        endSignal = (unsigned)reload(s1);
        endSignal &= (unsigned)reload(s2);
        endSignal &= (unsigned)reload(s3);
        endSignal &= (unsigned)reload(s4);
        n++;
    }
    /* endSignal is NOT read after the loop: that is what leaves the latch
     * block with the value dead at both boundaries. */
    *rounds = n;
    return n;
}

int main(void) {
    static unsigned char in1[64], in2[64], in3[64], in4[64];
    static unsigned char out[200];
    static unsigned char tab[16];
    struct stream s1 = {0, in1 + 40, in1};
    struct stream s2 = {0, in2 + 40, in2};
    struct stream s3 = {0, in3 + 40, in3};
    struct stream s4 = {0, in4 + 40, in4};
    int rounds = -1, e, i;
    unsigned sum = 0;

    for (i = 0; i < 16; i++)
        tab[i] = (unsigned char)(i * 3 + 1);
    for (i = 0; i < 64; i++) {
        in1[i] = (unsigned char)i;
        in2[i] = (unsigned char)(i * 5);
        in3[i] = (unsigned char)(i * 7);
        in4[i] = (unsigned char)(i * 11);
    }

    /* Each stream holds 40 bytes, each output lane 50: the loop is bounded
     * by endSignal alone and must run 41 rounds (the 41st reload finds the
     * streams exhausted and clears endSignal). The miscompiled build exits
     * after the first round. */
    e = run(out, out + 200, &s1, &s2, &s3, &s4, tab, &rounds) - rounds;
    for (i = 0; i < 200; i++)
        sum += out[i];
    printf("rounds=%d sum=%u e=%d\n", rounds, sum, e);
    return 0;
}
