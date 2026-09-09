/* i686 narrow-atomic correctness regression.
 *
 * Covers the result-extension, subtraction-via-neg/xadd, bitwise CAS-loop,
 * compare-exchange, test-and-set, and SeqCst-store paths. The exhaustive byte
 * sweep catches stale upper-register bits as well as modulo-width arithmetic.
 */

typedef signed char i8;
typedef unsigned char u8;
typedef short i16;
typedef unsigned short u16;
typedef unsigned u32;

__attribute__((noinline)) static i8 fetch_sub_i8(i8 *p, i8 v) {
    return __atomic_fetch_sub(p, v, __ATOMIC_SEQ_CST);
}

__attribute__((noinline)) static u8 fetch_add_u8(u8 *p, u8 v) {
    return __atomic_fetch_add(p, v, __ATOMIC_SEQ_CST);
}

__attribute__((noinline)) static u8 fetch_and_u8(u8 *p, u8 v) {
    return __atomic_fetch_and(p, v, __ATOMIC_RELAXED);
}

__attribute__((noinline)) static u8 fetch_or_u8(u8 *p, u8 v) {
    return __atomic_fetch_or(p, v, __ATOMIC_RELAXED);
}

__attribute__((noinline)) static u8 fetch_xor_u8(u8 *p, u8 v) {
    return __atomic_fetch_xor(p, v, __ATOMIC_RELAXED);
}

__attribute__((noinline)) static u8 fetch_nand_u8(u8 *p, u8 v) {
    return __atomic_fetch_nand(p, v, __ATOMIC_RELAXED);
}

__attribute__((noinline)) static u8 exchange_u8(u8 *p, u8 v) {
    return __atomic_exchange_n(p, v, __ATOMIC_SEQ_CST);
}

static int exhaustive_bytes(void) {
    for (unsigned old = 0; old != 256; ++old) {
        for (unsigned val = 0; val != 256; ++val) {
            u8 x = (u8)old;
            if (fetch_add_u8(&x, (u8)val) != (u8)old || x != (u8)(old + val))
                return 1;

            i8 sx = (i8)old;
            if (fetch_sub_i8(&sx, (i8)val) != (i8)old ||
                (u8)sx != (u8)(old - val))
                return 2;

            x = (u8)old;
            if (fetch_and_u8(&x, (u8)val) != (u8)old || x != (u8)(old & val))
                return 3;
            x = (u8)old;
            if (fetch_or_u8(&x, (u8)val) != (u8)old || x != (u8)(old | val))
                return 4;
            x = (u8)old;
            if (fetch_xor_u8(&x, (u8)val) != (u8)old || x != (u8)(old ^ val))
                return 5;
            x = (u8)old;
            if (fetch_nand_u8(&x, (u8)val) != (u8)old ||
                x != (u8)~(old & val))
                return 6;
            x = (u8)old;
            if (exchange_u8(&x, (u8)val) != (u8)old || x != (u8)val)
                return 7;
        }
    }
    return 0;
}

static int wider_and_cas(void) {
    static const u32 cases[] = {
        0, 1, 0x7f, 0x80, 0xff, 0x7fff, 0x8000, 0xffff,
        0x7fffffffU, 0x80000000U, 0xffffffffU, 0x55aa55aaU,
    };

    for (unsigned i = 0; i != sizeof(cases) / sizeof(cases[0]); ++i) {
        for (unsigned j = 0; j != sizeof(cases) / sizeof(cases[0]); ++j) {
            u16 x16 = (u16)cases[i];
            u16 v16 = (u16)cases[j];
            u16 old16 = __atomic_fetch_xor(&x16, v16, __ATOMIC_ACQ_REL);
            if (old16 != (u16)cases[i] || x16 != (u16)(cases[i] ^ cases[j]))
                return 10;

            i16 s16 = (i16)cases[i];
            i16 sold = __atomic_fetch_sub(&s16, (i16)cases[j], __ATOMIC_SEQ_CST);
            if (sold != (i16)cases[i] ||
                (u16)s16 != (u16)(cases[i] - cases[j]))
                return 11;

            u32 x32 = cases[i];
            u32 old32 = __atomic_fetch_nand(&x32, cases[j], __ATOMIC_RELAXED);
            if (old32 != cases[i] || x32 != ~(cases[i] & cases[j]))
                return 12;
        }
    }

    u8 x = 0x80;
    u8 expected = 0x7f;
    if (__atomic_compare_exchange_n(&x, &expected, 3, 0,
                                    __ATOMIC_SEQ_CST, __ATOMIC_SEQ_CST) ||
        x != 0x80 || expected != 0x80)
        return 13;
    expected = 0x80;
    if (!__atomic_compare_exchange_n(&x, &expected, 0xff, 0,
                                     __ATOMIC_SEQ_CST, __ATOMIC_SEQ_CST) ||
        x != 0xff)
        return 14;
    if ((u8)__sync_val_compare_and_swap(&x, 0xff, 0x81) != 0xff || x != 0x81)
        return 15;

    x = 0xfe;
    if (__atomic_test_and_set(&x, __ATOMIC_SEQ_CST) != 1 || x != 1)
        return 16;
    __atomic_clear(&x, __ATOMIC_SEQ_CST);
    if (x != 0)
        return 17;

    return 0;
}

int main(void) {
    int rc = exhaustive_bytes();
    return rc ? rc : wider_and_cas();
}
