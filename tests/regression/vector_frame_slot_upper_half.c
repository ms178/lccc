/* A 32-byte GNU vector built lane by lane must survive its frame slot.
 *
 * The defect this pins: the peephole classifier described every vector slot
 * move as 16 bytes, because it derived the footprint from the mnemonic
 * (`vmovdqu`) instead of from the register operand.  The never-read-store
 * pass therefore believed the top half of a 32-byte slot was not read, deleted
 * the stores that fill it, and the function returned a vector whose lanes
 * 16..31 were whatever the slot held.  Every shape below builds its result in
 * memory at some point -- lane-wise shifts, per-lane mul, lane extraction,
 * nine-argument spills -- and the frame slot is 32 bytes wide in all of them.
 *
 * The functions are deliberately NON-STATIC and NOINLINE.  Inlining them
 * removes the frame slot (the value stays in registers), and a static
 * definition is inlined even at -O0-adjacent levels, so a corpus of statics
 * passes with or without the fix -- measured: the defect reproduces only for
 * out-of-line definitions at -O2 -mavx2.
 *
 * This file is compiler-independent: it needs no reference compiler, no
 * ISA flags beyond x86-64, and no measurement.  It prints PASS or FAIL.
 */
#include <stdio.h>
#include <string.h>

typedef char v32c __attribute__((vector_size(32)));
typedef short v32s __attribute__((vector_size(32)));
typedef long v32l __attribute__((vector_size(32)));

/* Per-lane variable shift: lanes are extracted from, and re-inserted into, a
 * 32-byte object, which is the shape that allocates the wide slot. */
__attribute__((noinline)) v32c vec_shl32c(v32c a, v32c b) { return a << b; }
__attribute__((noinline)) v32s vec_shl32s(v32s a, v32s b) { return a << b; }
__attribute__((noinline)) v32l vec_mul32l(v32l a, v32l b) { return a * b; }

/* Lane read out of a 32-byte argument slot (MEMORY class below AVX here). */
__attribute__((noinline)) long vec_lane2(v32l a) { return a[2]; }

/* Lane insert: the value is staged through the same 32-byte slot. */
__attribute__((noinline)) v32l vec_setlane2(v32l a, long x) { a[2] = x; return a; }

/* Nine vector arguments: the ninth is passed on the stack, so the frame holds
 * a 32-byte incoming argument area as well as the result slot. */
__attribute__((noinline)) v32l vec_nine(v32l a, v32l b, v32l c, v32l d, v32l e,
                                       v32l f, v32l g, v32l h, v32l i) {
    return a + b + c + d + e + f + g + h + i;
}

/* A 32-byte value stored and reloaded through a named object. */
__attribute__((noinline)) v32l vec_roundtrip(v32l a) {
    v32l slot;
    memcpy(&slot, &a, sizeof slot);
    slot[1] = slot[1] + 1;
    return slot;
}

static int fails;

static void expect_bytes(const char *what, const void *got, const void *want,
                         size_t n) {
    if (memcmp(got, want, n) != 0) {
        printf("FAIL: %s\n", what);
        fails++;
    }
}

int main(void) {
    /* Lane values chosen so that a truncated upper half is unmistakable: the
     * low half is all ones and the high half carries a distinct pattern. */
    v32c ca, cb;
    for (int i = 0; i < 32; i++) {
        ca[i] = (char)(i * 3 + 1);
        cb[i] = (char)(i % 5);
    }
    v32c cr = vec_shl32c(ca, cb);
    for (int i = 0; i < 32; i++) {
        char want = (char)((unsigned char)ca[i] << cb[i]);
        if (cr[i] != want) {
            printf("FAIL: v32c shift lane %d: %d != %d\n", i, cr[i], want);
            fails++;
            break;
        }
    }

    v32s sa, sb;
    for (int i = 0; i < 16; i++) {
        sa[i] = (short)(i * 17 + 3);
        sb[i] = (short)(i % 4);
    }
    v32s sr = vec_shl32s(sa, sb);
    for (int i = 0; i < 16; i++) {
        short want = (short)((unsigned short)sa[i] << sb[i]);
        if (sr[i] != want) {
            printf("FAIL: v32s shift lane %d: %d != %d\n", i, sr[i], want);
            fails++;
            break;
        }
    }

    v32l la = {11, 22, 33, 44};
    if (vec_lane2(la) != 33) {
        printf("FAIL: lane2 = %ld, want 33 (upper-half lane read)\n",
               vec_lane2(la));
        fails++;
    }

    v32l parts[9];
    v32l want_sum = {0, 0, 0, 0};
    for (int i = 0; i < 9; i++) {
        for (int k = 0; k < 4; k++) {
            parts[i][k] = i * 4 + k;
            want_sum[k] += parts[i][k];
        }
    }
    v32l got_sum = vec_nine(parts[0], parts[1], parts[2], parts[3], parts[4],
                            parts[5], parts[6], parts[7], parts[8]);
    expect_bytes("nine-argument sum", &got_sum, &want_sum, sizeof got_sum);

    v32l rt = vec_roundtrip(la);
    v32l want_rt = {11, 23, 33, 44};
    expect_bytes("memcpy round trip", &rt, &want_rt, sizeof rt);

    /* Per-lane multiply over 64-bit lanes: the product of two 32-byte
     * arguments, computed from lane extracts. */
    v32l ma = {2, 3, 5, 7}, mb = {11, 13, 17, 19};
    v32l want_mul = {22, 39, 85, 133};
    v32l got_mul = vec_mul32l(ma, mb);
    expect_bytes("v32l per-lane multiply", &got_mul, &want_mul, sizeof got_mul);

    /* Lane insert into the upper half. */
    v32l want_set = {11, 22, 99, 44};
    v32l got_set = vec_setlane2(la, 99);
    expect_bytes("lane insert at index 2", &got_set, &want_set, sizeof got_set);

    if (fails) {
        printf("FAIL: %d check(s)\n", fails);
        return 1;
    }
    printf("PASS: 32-byte frame slots keep every lane\n");
    return 0;
}
