/* Width-partitioned 4-byte spill slots (small slots) correctness gate.
 *
 * Three independent miscompile classes were found and fixed while enabling
 * 4-byte spill slots on x86-64; each class is exercised below:
 *
 *  1. MachInst slot-to-slot Copy relays used unconditional 64-bit `movq`
 *     through 4-byte slots (rot/mix loop phis at -O0 read stale neighbour
 *     bytes out of a small src slot and clobbered the neighbour of a small
 *     dst slot).
 *  2. The Tier-3 block-local pool let finalize-time alignment rounding
 *     shift an 8-byte slot ONTO a following 4-byte slot's bytes
 *     (rot()'s v11/v14 [40,48) vs [44,48) overlap).
 *  3. Greedy free-list expiry is order-relative: processing values in any
 *     order other than definition order hands a still-live occupant's slot
 *     to a later value (zlib-ng build_tree v89/v319).
 *
 * Expected values are GCC-derived (verified identical at -O0/-O1/-O2).
 * Also validated under CCC_NO_SMALL_SLOTS=1 in the A/B differential gate.
 */
#include <stdint.h>
#include <stdio.h>

static volatile uint64_t observe;
static uint64_t rot(uint64_t x, unsigned n) { n &= 63u; return n ? ((x << n) | (x >> ((64u-n)&63u))) : x; }
static uint64_t mix(uint64_t a, uint64_t b, unsigned n) { a ^= rot(b + UINT64_C(0x9e3779b97f4a7c15), n); a *= UINT64_C(0xbf58476d1ce4e5b9); return a ^ (a >> 31); }

/* Class 1+2: 32-bit loop induction through mixed 4/8-byte slots. */
static int small_slot_loop_phi(void) {
    uint64_t a = UINT64_C(0x1e2feb89414c343c);
    uint64_t b = UINT64_C(0xc2ce6f447ed4d57b);
    uint64_t c = UINT64_C(0x78e510617311d8a3);
    for (unsigned i = 0; i < 4u; ++i) {
        switch ((unsigned)((a ^ b ^ c ^ i) & 7u)) {
        case 0: a = mix(a ^ UINT64_C(0x612e7696a6cecc1b), b + c, i); b ^= rot(c + UINT64_C(0x35bf992dc9e9c616), i); break;
        case 1: c = mix(c + UINT64_C(0x7ce42c8218072e8c), a ^ b, i+1u); a += rot(b ^ UINT64_C(0xe4b06ce60741c7a8), i); break;
        case 2: b = mix(b + UINT64_C(0x63ca828dd5f4b3b2), c, i+2u); c ^= rot(a + UINT64_C(0x9b810e766ec9d286), i); break;
        default: a ^= mix(b, c, i); c += mix(a, b, i+3u); break;
        }
        if ((a + i) & 1u) { uint64_t old = b++; a ^= mix(old, c, i); }
        else              { uint64_t old = c--; b ^= mix(old, a, i+1u); }
    }
    return (a ^ rot(b,17) ^ rot(c,39)) == UINT64_C(0xdd01d18a3214aa4f) ? 0 : 1;
}

/* Class 3: many simultaneously live 32-bit values around a call, forcing
 * small slots and Tier-3 reuse (order-relative expiry). */
static int sink_int(int x) { return x + 1; }
static int small_slot_tier3_reuse(void) {
    int v[48];
    int s = 0;
    for (int i = 0; i < 48; ++i) v[i] = i * 3 + 1;
    s += sink_int(s);
    for (int i = 0; i < 48; i += 2) s ^= v[i] + i;
    for (int i = 1; i < 48; i += 2) s += v[i] * 2;
    return s == 3505 ? 0 : 1;
}

/* Cross-width neighbour integrity: 64-bit values interleaved with 32-bit
 * values must never observe each other's bytes. */
static int small_slot_neighbours(void) {
    uint64_t wide = UINT64_C(0x1122334455667788);
    uint32_t narrow = 0xAABBCCDDu;
    uint64_t wide2 = UINT64_C(0x8877665544332211);
    uint32_t narrow2 = 0x1E2D3C4Bu;
    for (unsigned i = 0; i < 32u; ++i) {
        narrow ^= (uint32_t)i;
        wide += narrow;
        narrow2 += (uint32_t)(i * 7u);
        wide2 ^= narrow2;
    }
    return (wide == UINT64_C(0x11223359ace01248) &&
            wide2 == UINT64_C(0x8877665544332cf1) &&
            narrow == 0xAABBCCDDu &&
            narrow2 == 0x1E2D49DBu) ? 0 : 1;
}

int main(void) {
    int r = 0;
    r |= small_slot_loop_phi();
    observe = r;
    r |= small_slot_tier3_reuse();
    r |= small_slot_neighbours();
    if (r) printf("FAIL small_slot_width_partition (r=%d)\n", r);
    else   printf("PASS small_slot_width_partition\n");
    return r;
}
