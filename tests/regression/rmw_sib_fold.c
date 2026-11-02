// Differential fixture for the two new x86 peephole folds:
//   * `fold_memory_rmw`     -- load / increment / store-back of one address
//                              becomes a single `add $imm, MEM`;
//   * `fold_shift_into_sib` -- an index `shl` byte-shifted before an `addq`
//                              becomes the SIB scale of the addressing mode
//                              (and its result is folded into the consumer).
//
// Every shape here is checked against GCC at -O1/-O2/-O3 and against the
// same binary built with the passes killed (CCC_NO_MEM_RMW / CCC_NO_SHL_SIB),
// so a wrong rewrite shows up as a value mismatch even where the emitted
// instruction count would look better.
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#define N 4096

// 1. The histogram shape: byte-indexed table increment with a scale of 8.
static unsigned char src[N];
static uint64_t bins[256];
static uint64_t hist(void) {
    memset(bins, 0, sizeof bins);
    for (unsigned i = 0; i < N; ++i)
        bins[src[i]]++;
    uint64_t acc = 0;
    for (unsigned b = 0; b < 256; ++b)
        acc = acc * 31 + bins[b];
    return acc;
}

// 2. Scaled index written by hand: `t[i * 8] += v` (the shift feeds the add).
static uint32_t t32[2048];
static uint64_t scaled_index(uint64_t seed) {
    for (unsigned i = 0; i < 512; ++i)
        t32[i * 4] += (uint32_t)(seed + i);
    uint64_t acc = 0;
    for (unsigned i = 0; i < 2048; ++i)
        acc = acc * 131 + t32[i];
    return acc;
}

// 3. Byte and word widths: `u8[i]++`, `u16[i]++`, and a decrement.
static uint8_t u8[256];
static uint16_t u16[256];
static uint64_t widths(uint64_t seed) {
    for (unsigned i = 0; i < 256; ++i) {
        u8[(unsigned char)seed % 256]++;
        u16[(unsigned char)(seed >> 8) % 256] += 3;
        seed = seed * 6364136223846793005ull + 1442695040888963407ull;
    }
    uint64_t acc = 0;
    for (unsigned i = 0; i < 256; ++i)
        acc += u8[i] * 1000 + u16[i];
    return acc;
}

// 4. The loaded count must stay live: a reader sees the value the RMW fold
//    would otherwise delete.
static uint64_t live_reader(uint64_t seed) {
    uint64_t acc = 0;
    for (unsigned i = 0; i < 256; ++i) {
        uint64_t v = bins[i];
        bins[i] += 1;
        acc += v * (i + 1); // reads the PRE-increment value: must not change
    }
    return acc;
}

// 5. Flags must stay live across the fold point.
static uint64_t flags_live(uint64_t seed) {
    uint64_t acc = 0;
    for (unsigned i = 0; i < 256; ++i) {
        bins[i] += 7;
        acc += (bins[i] > 7) ? i : 0; // reads the flags of the add
    }
    return acc;
}

// 6. A doubling written as `x + x` (folds to a scale-2 SIB) and one as `x * 8`.
static uint64_t doubling(uint64_t seed) {
    uint64_t acc = seed;
    unsigned char *p = src;
    for (unsigned i = 0; i < 64; ++i) {
        unsigned idx = (unsigned)(p[i] * 2u) & 255u;
        acc += bins[idx];
        acc += bins[(unsigned)(p[i] * 8u) & 255u];
    }
    return acc;
}

// 7. Byte- and word-wide counters, the shape real parsers/compressors use
//    (`counts[c]++`).  The backend reaches the store through a narrow copy of
//    the low byte (`movzbl %r8b,%r9d`), which the RMW fold has to see through
//    -- including its own deadness obligation.
static unsigned char c8[256];
static unsigned short c16[256];
static signed char sc8[256];
static uint64_t counters(uint64_t seed) {
    for (unsigned i = 0; i < 512; ++i) {
        unsigned char a = (unsigned char)(seed + i * 7u);
        unsigned char b = (unsigned char)((seed >> 3) + i * 5u);
        c8[a]++;
        c16[b] += 5;
        // In-bounds on purpose: a negative `signed char` subscript here is UB
        // and made GCC's binary segfault while lccc's survived -- the oracle
        // comparison caught the FIXTURE's bug before it could be mistaken for
        // a codegen difference.
        sc8[(unsigned)((a ^ (unsigned char)i) & 0x7f)] += 3;
    }
    uint64_t acc = 0;
    for (unsigned i = 0; i < 256; ++i)
        acc = acc * 131 + c8[i] + c16[i] + (unsigned)(unsigned char)sc8[i];
    return acc;
}

// 8. The counter value must stay live: reading before the bump pins the
//    pre-increment value of a NARROW counter (the copy path's obligation).
static uint64_t narrow_live_reader(uint64_t seed) {
    uint64_t acc = 0;
    for (unsigned i = 0; i < 256; ++i) {
        unsigned char v = c8[i];
        c8[i] += 2;
        acc += (uint64_t)v * (i + 1);
    }
    return acc;
}

// Named entry points for the gate's asm contracts (static helpers are inlined
// away and leave no symbol to grep).  Same shapes as `counters`, one width per
// function.
unsigned char g_c8[256];
unsigned short g_c16[256];
void rmw_bump_u8(unsigned char c) { g_c8[c]++; }
void rmw_bump_u16(unsigned short c) { g_c16[c]++; }

int main(void) {
    for (unsigned i = 0; i < N; ++i)
        src[i] = (unsigned char)((i * 2654435761u) >> 24);
    memset(bins, 0, sizeof bins);
    memset(t32, 0, sizeof t32);
    memset(u8, 0, sizeof u8);
    memset(u16, 0, sizeof u16);
    memset(c8, 0, sizeof c8);
    memset(c16, 0, sizeof c16);
    memset(sc8, 0, sizeof sc8);

    printf("hist=%llu\n", (unsigned long long)hist());
    printf("scaled=%llu\n", (unsigned long long)scaled_index(12345));
    printf("widths=%llu\n", (unsigned long long)widths(99));
    printf("live=%llu\n", (unsigned long long)live_reader(0));
    printf("flags=%llu\n", (unsigned long long)flags_live(0));
    printf("double=%llu\n", (unsigned long long)doubling(7));
    printf("counters=%llu\n", (unsigned long long)counters(0x51ab3u));
    printf("narrow=%llu\n", (unsigned long long)narrow_live_reader(0));
    return 0;
}
