/* x86-64 atomic return-value and narrow canonicalization regression. */

typedef signed char i8;
typedef unsigned char u8;
typedef short i16;
typedef unsigned short u16;
typedef int i32;
typedef unsigned u32;
typedef long long i64;
typedef unsigned long long u64;

__attribute__((noinline)) static i64 add_one(i64 *p) {
    return __atomic_fetch_add(p, 1, __ATOMIC_SEQ_CST);
}

__attribute__((noinline)) static i8 add_i8(i8 *p, i8 v) {
    return __atomic_fetch_add(p, v, __ATOMIC_SEQ_CST);
}

__attribute__((noinline)) static i16 sub_i16(i16 *p, i16 v) {
    return __atomic_fetch_sub(p, v, __ATOMIC_SEQ_CST);
}

__attribute__((noinline)) static i8 nand_i8(i8 *p, i8 v) {
    return __atomic_fetch_nand(p, v, __ATOMIC_RELAXED);
}

int main(void) {
    i64 q = 0x123456789abcdefLL;
    if (add_one(&q) != 0x123456789abcdefLL || q != 0x123456789abcdf0LL)
        return 1;

    for (unsigned old = 0; old != 256; ++old) {
        for (unsigned val = 0; val != 256; ++val) {
            i8 x = (i8)old;
            if (add_i8(&x, (i8)val) != (i8)old ||
                (u8)x != (u8)(old + val))
                return 2;
            x = (i8)old;
            if (nand_i8(&x, (i8)val) != (i8)old ||
                (u8)x != (u8)~(old & val))
                return 3;
        }
    }

    static const u32 cases[] = {
        0, 1, 0x7fff, 0x8000, 0xffff, 0x7fffffffU,
        0x80000000U, 0xffffffffU, 0x55aa55aaU,
    };
    for (unsigned i = 0; i != sizeof(cases) / sizeof(cases[0]); ++i) {
        for (unsigned j = 0; j != sizeof(cases) / sizeof(cases[0]); ++j) {
            i16 x = (i16)cases[i];
            if (sub_i16(&x, (i16)cases[j]) != (i16)cases[i] ||
                (u16)x != (u16)(cases[i] - cases[j]))
                return 4;

            u32 ux = cases[i];
            u32 old = __atomic_fetch_xor(&ux, cases[j], __ATOMIC_ACQ_REL);
            if (old != cases[i] || ux != (cases[i] ^ cases[j]))
                return 5;
        }
    }

    i8 c = (i8)0x80;
    if (__sync_val_compare_and_swap(&c, (i8)0x7f, 1) != (i8)0x80 ||
        (u8)c != 0x80)
        return 6;
    if (__sync_val_compare_and_swap(&c, (i8)0x80, (i8)0xff) != (i8)0x80 ||
        (u8)c != 0xff)
        return 7;

    u32 u = 0x80000000U;
    if (__sync_val_compare_and_swap(&u, 0x80000000U, 0xffffffffU) !=
            0x80000000U ||
        u != 0xffffffffU)
        return 8;

    return 0;
}
