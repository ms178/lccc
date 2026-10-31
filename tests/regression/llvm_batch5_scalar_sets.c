/* Batch-5 scalar sets: shift-pair masks and {0,1} membership.
 *
 * Every function is self-checking: `main` compares the optimized result with a
 * reference computed from volatile inputs, so a wrong fold shows up as a
 * runtime failure rather than as a length difference.  The probes cover the
 * mask edges (all-ones, the bit just below the kept field, alternating bits)
 * and the membership boundaries (0, 1, 2, -1, INT_MIN, INT_MAX).
 */
#include <stdio.h>
#include <stdint.h>

static volatile uint32_t vx;
static volatile uint64_t vy;

__attribute__((noinline)) static uint32_t shl_lshr_3(uint32_t x) { return (x << 3) >> 3; }
__attribute__((noinline)) static uint32_t shl_lshr_12(uint32_t x) { return (x << 12) >> 12; }
__attribute__((noinline)) static uint32_t shl_lshr_31(uint32_t x) { return (x << 31) >> 31; }
__attribute__((noinline)) static uint32_t shl_lshr_sig(uint32_t x) { return (x << 16) >> 16; }
__attribute__((noinline)) static uint64_t shl_lshr64_41(uint64_t x) { return (x << 41) >> 41; }
__attribute__((noinline)) static int memb_or(uint32_t x) { return (x == 0u) | (x == 1u); }
__attribute__((noinline)) static int memb_or_rev(uint32_t x) { return (x == 1u) | (x == 0u); }
__attribute__((noinline)) static int memb_and(uint32_t x) { return (x != 0u) & (x != 1u); }
/* A non-member that must NOT fold: the set is {0, 2}. */
__attribute__((noinline)) static int memb_control(uint32_t x) { return (x == 0u) | (x == 2u); }

int main(void) {
    static const uint32_t probes[] = {
        0u, 1u, 2u, 3u, 7u, 8u, 15u, 4095u, 4096u,
        0x7fffffffu, 0x80000000u, 0xfffffffeu, 0xffffffffu,
        0xfffffff8u, 0x12345678u, 0xdeadbeefu, 0xaaaaaaaau, 0x55555555u,
    };
    static const uint64_t probes64[] = {
        0ull, 1ull, 0x7ffffffffffull, 0x80000000000ull,
        0xfffffffffffull, 0xffffffffffffffffull, 0x0123456789abcdefull,
    };
    uint64_t bad = 0;
    for (unsigned i = 0; i < sizeof probes / sizeof *probes; i++) {
        uint32_t x = probes[i];
        vx = x;
        uint32_t ref3 = (uint32_t)((x << 3) >> 3);
        uint32_t ref12 = (uint32_t)((x << 12) >> 12);
        uint32_t ref31 = (uint32_t)((x << 31) >> 31);
        uint32_t refsig = (uint32_t)((x << 16) >> 16);
        if (shl_lshr_3(vx) != ref3) bad++;
        if (shl_lshr_12(vx) != ref12) bad++;
        if (shl_lshr_31(vx) != ref31) bad++;
        if (shl_lshr_sig(vx) != refsig) bad++;
        if (memb_or(vx) != ((x == 0u) | (x == 1u))) bad++;
        if (memb_or_rev(vx) != ((x == 0u) | (x == 1u))) bad++;
        if (memb_and(vx) != ((x != 0u) & (x != 1u))) bad++;
        if (memb_control(vx) != ((x == 0u) | (x == 2u))) bad++;
    }
    for (unsigned i = 0; i < sizeof probes64 / sizeof *probes64; i++) {
        uint64_t y = probes64[i];
        vy = y;
        if (shl_lshr64_41(vy) != (uint64_t)((y << 41) >> 41)) bad++;
    }
    /* Deterministic sweep: 200k values through both folds. */
    uint64_t s = 0x243f6a8885a308d3ull;
    for (int i = 0; i < 200000; i++) {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        vx = (uint32_t)s;
        vy = s;
        uint32_t x = (uint32_t)s;
        if (shl_lshr_3(vx) != (uint32_t)((x << 3) >> 3)) bad++;
        if (shl_lshr_sig(vx) != (uint32_t)((x << 16) >> 16)) bad++;
        if (shl_lshr64_41(vy) != (uint64_t)((s << 41) >> 41)) bad++;
        if (memb_or(vx) != ((x == 0u) | (x == 1u))) bad++;
        if (memb_and(vx) != ((x != 0u) & (x != 1u))) bad++;
        if (memb_control(vx) != ((x == 0u) | (x == 2u))) bad++;
    }
    if (bad) {
        printf("batch5: %llu mismatches\n", (unsigned long long)bad);
        return 1;
    }
    printf("batch5: ok\n");
    return 0;
}
