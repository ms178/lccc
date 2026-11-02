/* The select-from-compare fusion: a comparison's flags feed the `cmov`
 * directly, without materialising the comparison as a 0/1 byte first.
 *
 * Why this file exists.  `flags_peepholes::fold_setcc_test_cmov` recovers
 * `cmp; cmovcc` from the backend's `setcc; (widen); test; cmov` idiom.  It
 * originally required the tested register to be the boolean's OWN register,
 * but the IR's `Cast(bool -> i32)` lowers to a sign-extending copy into a fresh
 * register (`movsbq %bl, %r11`), so the idiom was not recognised and the
 * ASCII case-fold kernel kept `setbe; movzbl; movsbq; test; cmovne` -- four
 * pointless instructions per byte (Callgrind on
 * tests/benchmark/programs/ascii_case_fold.c: 2,522,590 Ir before, 2,211,009
 * after, GCC 16.2 at 1,188,803 with its own vectorized loop).
 *
 * The shapes below cover both widening directions and the refusals:
 *   fold_u8 / fold_u8_wide   the enabled shape (byte range compare, u32 result)
 *   fold_i8                  signed byte source -> `movsbl`
 *   pick_max / pick_abs      the pre-existing plain-boolean shapes
 *   bool_kept                the boolean has a second use: the definition must
 *                            survive, so the fusion must NOT fire
 *
 * The shapes are non-static so each keeps its own symbol: inlined into `main`
 * they would be unreachable for the object-code contract, which is exactly how
 * the first version of this gate passed the semantics and failed to find the
 * `setcc` it wanted to pin.
 *   sign_bit                 a 16-bit-source extension of a family whose high
 *                            byte is not the boolean: the walk must refuse it
 *                            (reading `%bx` after `setcc %bl` would sign-extend
 *                            garbage) and the result must stay exact
 */
#include <stdio.h>

static unsigned char buf[4096];

unsigned fold_u8(unsigned char c) {
    return (c >= 'A' && c <= 'Z') ? c + 32u : c;
}

unsigned fold_u8_wide(unsigned char c) {
    /* the same test, but the result is used as a wide accumulator step */
    unsigned x = (c >= 0101 && c <= 0132) ? (unsigned)c + 32u : (unsigned)c;
    return x * 3u;
}

int fold_i8(signed char c) {
    return (c >= 'a' && c <= 'z') ? c - 32 : c;
}

int pick_max(int a, int b) { return a > b ? a : b; }

int pick_abs(int x) { return x < 0 ? -x : x; }

int bool_kept(int x) {
    int t = x > 0;
    int s = t ? 7 : 9;   /* select */
    return s + t * 100;  /* the boolean is used again: keep its definition */
}

int sign_bit(signed char c) {
    /* the compare's value widened through a 16-bit intermediate; must stay
     * exact whether or not the fusion applies */
    short w = (short)(c - 5);
    return (w <= 0) ? 1 : 2;
}

int main(void) {
    unsigned acc = 0;
    int iacc = 0;
    for (int i = 0; i < 4096; i++) {
        unsigned char c = (unsigned char)((i * 37 + 11) & 0xff);
        buf[i] = c;
        if (fold_u8(c) != (c >= 'A' && c <= 'Z' ? c + 32u : c)) return 1;
        if (fold_u8_wide(c) != (unsigned)((c >= 0101 && c <= 0132 ? c + 32u : c) * 3u)) return 2;
        if (fold_i8((signed char)c) !=
            ((signed char)c >= 'a' && (signed char)c <= 'z' ? (signed char)c - 32
                                                             : (signed char)c))
            return 3;
        if (pick_max((int)c - 128, 3) != ((int)c - 128 > 3 ? (int)c - 128 : 3)) return 4;
        if (pick_abs((int)c - 128) != ((int)c - 128 < 0 ? 128 - (int)c : (int)c - 128)) return 5;
        if (bool_kept((int)c - 128) != (((int)c - 128 > 0 ? 7 : 9) + ((int)c - 128 > 0) * 100))
            return 6;
        if (sign_bit((signed char)c) != (((short)((signed char)c - 5) <= 0) ? 1 : 2)) return 7;
        acc += fold_u8(c) + fold_u8_wide(c) + (unsigned)fold_i8((signed char)c);
        iacc += pick_max(c, 200) + pick_abs((int)c - 128) + bool_kept((int)c - 200) +
                sign_bit((signed char)c);
    }
    printf("select-from-compare: acc=%u iacc=%d\n", acc, iacc);
    return 0;
}
