/* Caller half of the dirty-upper-bits interop fixture; see
 * abi_dirty_upper_interop.h.  Self-checking: exit status 0 is success,
 * otherwise the number of the first failing check.
 *
 * Every alias below names a callee symbol through a prototype that widens
 * its narrow parameters (and, for the du_ret_* functions, its result) to
 * long; DU_*() supplies the dirty (AAPCS64) or psABI-extended (RV64)
 * register image of each narrow value. */
#include "abi_dirty_upper_interop.h"

#define CHECK(id, cond)                                                                        \
    do {                                                                                       \
        if (!(cond))                                                                           \
            return id;                                                                         \
    } while (0)

int du_select(long a, long b, long c, long d, long k) __asm__("du_select");
int du_branch(long a, long b, long c, long d, long k) __asm__("du_branch");
int du_select8(long a, long b, long c, long d, long e, long f, long g, long k)
    __asm__("du_select8");
int du_select_u8(long a, long b, long k) __asm__("du_select_u8");
int du_bool(long f, long x) __asm__("du_bool");
int du_bool_branch(long x, long f) __asm__("du_bool_branch");
long du_widen_s32(long a, long b, long c, long d, long k) __asm__("du_widen_s32");
unsigned long du_widen_u32(long a, long b, long c, long d, long k) __asm__("du_widen_u32");
long du_widen_s16(long s) __asm__("du_widen_s16");
unsigned long du_widen_u16(long s) __asm__("du_widen_u16");
long du_widen_s8(long c) __asm__("du_widen_s8");
unsigned long du_widen_u8(long c) __asm__("du_widen_u8");
int du_switch(long a, long b, long c, long d, long k) __asm__("du_switch");
int du_index(const int *p, long i) __asm__("du_index");
int du_index_u(const int *p, long i) __asm__("du_index_u");
long du_sum(const int *p, long n) __asm__("du_sum");
int du_div(long a, long b) __asm__("du_div");
unsigned du_divu(long a, long b) __asm__("du_divu");
int du_lt(long a, long b) __asm__("du_lt");
int du_ltu(long a, long b) __asm__("du_ltu");
int du_eq8(long a, long b) __asm__("du_eq8");
int du_lt8(long a, long b) __asm__("du_lt8");
int du_lt16(long a, long b) __asm__("du_lt16");
long du_mul_wide(long a, long b) __asm__("du_mul_wide");
long du_add_wide(long a, long b) __asm__("du_add_wide");
unsigned du_shift(long x, long s) __asm__("du_shift");
int du_live(long a, long b, long k) __asm__("du_live");
int du_live_branch(long a, long b, long k) __asm__("du_live_branch");
long du_live_widen(long a, long k) __asm__("du_live_widen");
unsigned long du_live_widen_u(long a, long k) __asm__("du_live_widen_u");
int du_stack9(long a, long b, long c, long d, long e, long f, long g, long h, long k)
    __asm__("du_stack9");
long du_stack9_widen(long a, long b, long c, long d, long e, long f, long g, long h, long k)
    __asm__("du_stack9_widen");
long du_stack9_short(long a, long b, long c, long d, long e, long f, long g, long h, long k)
    __asm__("du_stack9_short");
int du_ret_s32(long x) __asm__("du_ret_s32");
unsigned short du_ret_u16(long x) __asm__("du_ret_u16");
signed char du_ret_s8(long x) __asm__("du_ret_s8");

static const int table[8] = {10, 20, 30, 40, 50, 60, 70, 80};

static int check_conditions(void)
{
    CHECK(1, du_select(DU_S32(1), DU_S32(2), DU_S32(3), DU_S32(4), DU_S32(0)) == 6);
    CHECK(2, du_select(DU_S32(1), DU_S32(2), DU_S32(3), DU_S32(4), DU_S32(9)) == 4);
    CHECK(3, du_branch(DU_S32(10), DU_S32(6), DU_S32(3), DU_S32(7), DU_S32(0)) == 42);
    CHECK(4, du_branch(DU_S32(10), DU_S32(6), DU_S32(3), DU_S32(7), DU_S32(-1)) == 7);
    CHECK(5, du_select8(DU_S32(1), DU_S32(2), DU_S32(3), DU_S32(4), DU_S32(5), DU_S32(6),
                        DU_S32(7), DU_S32(0)) == 6 - 1 + 2 - 3 + 4);
    CHECK(6, du_select8(DU_S32(1), DU_S32(2), DU_S32(3), DU_S32(4), DU_S32(5), DU_S32(6),
                        DU_S32(7), DU_S32(1)) == 12);
    CHECK(7, du_select_u8(DU_S32(5), DU_S32(8), DU_U8(0)) == 8);
    CHECK(8, du_select_u8(DU_S32(5), DU_S32(8), DU_U8(0x80)) == 5);
    CHECK(9, du_bool(DU_U8(0), DU_S32(9)) == -9);
    CHECK(10, du_bool(DU_U8(1), DU_S32(9)) == 9);
    CHECK(11, du_bool_branch(DU_S32(100), DU_U8(0)) == 93);
    CHECK(12, du_bool_branch(DU_S32(100), DU_U8(1)) == 107);
    return 0;
}

static int check_widening(void)
{
    CHECK(20, du_widen_s32(DU_S32(1), DU_S32(2), DU_S32(3), DU_S32(4), DU_S32(-7)) == -7L);
    CHECK(21, du_widen_s32(DU_S32(1), DU_S32(2), DU_S32(3), DU_S32(4), DU_S32(0x7fffffff)) ==
                  0x7fffffffL);
    CHECK(22, du_widen_u32(DU_U32(1), DU_U32(2), DU_U32(3), DU_U32(4), DU_U32(0xfffffff9u)) ==
                  0xfffffff9UL);
    CHECK(23, du_widen_s16(DU_S16(-300)) == -300L);
    CHECK(24, du_widen_u16(DU_U16(0xfed4)) == 0xfed4UL);
    CHECK(25, du_widen_s8(DU_S8(-100)) == -100L);
    CHECK(26, du_widen_u8(DU_U8(0x9c)) == 0x9cUL);
    return 0;
}

static int check_indexing(void)
{
    CHECK(30, du_switch(DU_S32(1), DU_S32(2), DU_S32(3), DU_S32(4), DU_S32(0)) == 11);
    CHECK(31, du_switch(DU_S32(1), DU_S32(2), DU_S32(3), DU_S32(4), DU_S32(3)) == 46);
    CHECK(32, du_switch(DU_S32(1), DU_S32(2), DU_S32(3), DU_S32(4), DU_S32(7)) == 92);
    CHECK(33, du_switch(DU_S32(1), DU_S32(2), DU_S32(3), DU_S32(4), DU_S32(8)) == -1);
    CHECK(34, du_switch(DU_S32(1), DU_S32(2), DU_S32(3), DU_S32(4), DU_S32(-1)) == -1);
    CHECK(35, du_index(table + 4, DU_S32(-3)) == 20);
    CHECK(36, du_index(table + 4, DU_S32(3)) == 80);
    CHECK(37, du_index_u(table, DU_U32(6)) == 70);
    CHECK(38, du_sum(table, DU_S32(5)) == 150);
    CHECK(39, du_sum(table, DU_S32(-2)) == 0);
    return 0;
}

static int check_arithmetic(void)
{
    CHECK(40, du_div(DU_S32(-91), DU_S32(7)) == -13);
    CHECK(41, du_divu(DU_U32(0xfffffff0u), DU_U32(16)) == 0x0fffffffu);
    CHECK(42, du_lt(DU_S32(-1), DU_S32(1)) == 1);
    CHECK(43, du_lt(DU_S32(1), DU_S32(-1)) == 0);
    CHECK(44, du_ltu(DU_U32(0xffffffffu), DU_U32(1)) == 0);
    CHECK(45, du_ltu(DU_U32(1), DU_U32(0xffffffffu)) == 1);
    CHECK(46, du_eq8(DU_U8(0x41), DU_U8(0x41)) == 1);
    CHECK(47, du_eq8(DU_U8(0x41), DU_U8(0x42)) == 0);
    CHECK(48, du_lt8(DU_S8(-2), DU_S8(1)) == 1);
    CHECK(49, du_lt8(DU_S8(1), DU_S8(-2)) == 0);
    CHECK(50, du_lt16(DU_S16(-2000), DU_S16(1000)) == 1);
    CHECK(51, du_lt16(DU_S16(1000), DU_S16(-2000)) == 0);
    CHECK(52, du_mul_wide(DU_S32(-65536), DU_S32(65536)) == -4294967296L);
    CHECK(53, du_add_wide(DU_S32(-1), DU_U32(0xffffffffu)) == 0xfffffffeL);
    CHECK(54, du_shift(DU_U32(3), DU_S32(30)) == 0xc0000000u);
    return 0;
}

static int check_live_and_stack(void)
{
    CHECK(60, du_live(DU_S32(3), DU_S32(4), DU_S32(0)) == 4);
    CHECK(61, du_live(DU_S32(3), DU_S32(4), DU_S32(2)) == 3);
    CHECK(62, du_live_branch(DU_S32(3), DU_S32(4), DU_S32(0)) == 4);
    CHECK(63, du_live_branch(DU_S32(3), DU_S32(4), DU_S32(-8)) == 3);
    CHECK(64, du_live_widen(DU_S32(3), DU_S32(-12)) == -12L);
    CHECK(65, du_live_widen_u(DU_S32(3), DU_U32(0xfffffff4u)) == 0xfffffff4UL);
    CHECK(66, du_stack9(DU_S32(1), DU_S32(2), DU_S32(3), DU_S32(4), DU_S32(5), DU_S32(6),
                        DU_S32(7), DU_S32(8), DU_S32(0)) == 2 - 7 + 3 - 6 + 4 - 5);
    CHECK(67, du_stack9(DU_S32(1), DU_S32(2), DU_S32(3), DU_S32(4), DU_S32(5), DU_S32(6),
                        DU_S32(7), DU_S32(8), DU_S32(5)) == 9);
    CHECK(68, du_stack9_widen(DU_S32(1), DU_S32(2), DU_S32(3), DU_S32(4), DU_S32(5), DU_S32(6),
                              DU_S32(7), DU_S32(8), DU_S32(-77)) == -77L);
    CHECK(69, du_stack9_short(DU_S32(1), DU_S32(2), DU_S32(3), DU_S32(4), DU_S32(5),
                              DU_S32(6), DU_S32(7), DU_S32(8), DU_S16(-1234)) == -1234L);
    return 0;
}

static int check_results(void)
{
    if (du_ret_s32(DU_S32(0)))
        return 70;
    CHECK(71, (du_ret_s32(DU_S32(0)) ? 1 : 2) == 2);
    CHECK(72, du_ret_s32(DU_S32(0)) == 0);
    CHECK(73, (long)du_ret_s32(DU_S32(-5)) == -5L);
    CHECK(74, (int)du_ret_u16(DU_U16(0xfffe)) == 0xfffe);
    CHECK(75, (long)du_ret_s8(DU_S8(-3)) == -3L);
    CHECK(76, table[du_ret_s32(DU_S32(2))] == 30);
    return 0;
}

int main(void)
{
    int r;
    if ((r = check_conditions()) || (r = check_widening()) || (r = check_indexing()) ||
        (r = check_arithmetic()) || (r = check_live_and_stack()) || (r = check_results()))
        return r;
    return 0;
}
