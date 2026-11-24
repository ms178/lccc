/* The intrinsics of x86gprintrin.h that an ordinary process can execute.
 *
 * The others are privileged or need hardware a test machine may lack;
 * check_x86_gpr_intrinsics.py holds their instruction and operands to
 * GCC's without running them. Here the ones that run are run, each behind
 * the processor's (and, for FSGSBASE, the kernel's) own word that it may:
 * a line reads "absent" on a machine without the extension, for GCC's
 * program as for lccc's, so the two outputs are the same on any host.
 *
 * The add-with-carry family is ordinary arithmetic and is checked against
 * 128-bit arithmetic on a few thousand operand triples, including every
 * combination of the boundary values. */
#include <immintrin.h>
#include <stdio.h>
#include <string.h>

static unsigned long long state = 0x9e3779b97f4a7c15ull;
static unsigned long long next(void)
{
    state ^= state << 13;
    state ^= state >> 7;
    state ^= state << 17;
    return state;
}

static int carry_chains(void)
{
    static const unsigned long long edge[] = {0, 1, 2, 0x7fffffffffffffffull, 0x8000000000000000ull,
                                              0xfffffffffffffffeull, 0xffffffffffffffffull,
                                              0xffffffffull, 0x100000000ull};
    enum { EDGES = sizeof edge / sizeof edge[0] };
    int bad = 0;
    for (int round = 0; round < 4000 + EDGES * EDGES * 2; round++) {
        unsigned long long x, y;
        unsigned char cf;
        if (round < EDGES * EDGES * 2) {
            x = edge[round % EDGES];
            y = edge[round / EDGES % EDGES];
            cf = (unsigned char)(round / (EDGES * EDGES) ? 0xff : 0); /* any nonzero carry counts */
        } else {
            x = next();
            y = next();
            cf = (unsigned char)(next() & 3);
        }
        unsigned long long sum, diff;
        unsigned int sum32, diff32;
        unsigned char c = _addcarry_u64(cf, x, y, &sum);
        unsigned __int128 wide = (unsigned __int128)x + y + (cf != 0);
        bad += sum != (unsigned long long)wide || c != (unsigned char)(wide >> 64);
        c = _addcarryx_u64(cf, x, y, &sum);
        bad += sum != (unsigned long long)wide || c != (unsigned char)(wide >> 64);
        c = _subborrow_u64(cf, x, y, &diff);
        wide = (unsigned __int128)x - y - (cf != 0);
        bad += diff != (unsigned long long)wide || c != (unsigned char)((wide >> 64) & 1);
        unsigned int x32 = (unsigned int)x, y32 = (unsigned int)y;
        c = _addcarry_u32(cf, x32, y32, &sum32);
        unsigned long long w = (unsigned long long)x32 + y32 + (cf != 0);
        bad += sum32 != (unsigned int)w || c != (unsigned char)(w >> 32);
        c = _addcarryx_u32(cf, x32, y32, &sum32);
        bad += sum32 != (unsigned int)w || c != (unsigned char)(w >> 32);
        c = _subborrow_u32(cf, x32, y32, &diff32);
        w = (unsigned long long)x32 - y32 - (cf != 0);
        bad += diff32 != (unsigned int)w || c != (unsigned char)((w >> 32) & 1);
    }
    /* A four-limb chain, as multi-precision code writes it. */
    unsigned long long a[4] = {~0ull, ~0ull, ~0ull, 1}, b[4] = {1, 0, 0, 2}, r[4];
    unsigned char c = 0;
    for (int i = 0; i < 4; i++)
        c = _addcarry_u64(c, a[i], b[i], &r[i]);
    bad += !(r[0] == 0 && r[1] == 0 && r[2] == 0 && r[3] == 4 && c == 0);
    c = 0;
    for (int i = 0; i < 4; i++)
        c = _subborrow_u64(c, r[i], b[i], &r[i]);
    bad += memcmp(r, a, sizeof r) != 0 || c != 0;
    return bad;
}

__attribute__((noinline)) static unsigned long long flags_after_compare(long a, long b)
{
    unsigned long long before = __readeflags();
    /* Set CF (bit 0) and clear ZF (bit 6) through the register itself. */
    __writeeflags((before | 1) & ~0x40ull);
    unsigned long long after = __readeflags();
    __writeeflags(before);
    return (after & 0x41) + (unsigned long long)(a < b);
}

int main(void)
{
    printf("carry chains: %d wrong\n", carry_chains());
    printf("eflags: %llu\n", flags_after_compare(1, 2));
    printf("andn %x %llx tzcnt16 %u %u\n", _andn_u32(0xf0f0f0f0u, 0xffff0000u),
           _andn_u64(0xf0f0f0f0f0f0f0f0ull, 0xffffffff00000000ull), _tzcnt_u16(0x80), _tzcnt_u16(0));

    if (__builtin_cpu_supports("sse4.2")) {
        unsigned int c = __crc32b(0, 'a');
        c = __crc32w(c, 0x6362);
        c = __crc32d(c, 0x67666564);
        printf("crc32c %08x %016llx\n", c, __crc32q(c, 0x6f6e6d6c6b6a6968ull));
    } else {
        printf("crc32c absent\n");
    }

    /* RDPID is IA32_TSC_AUX, which RDTSCP also returns. A thread may move
     * between the two reads, so ask until both agree (or give up). */
    if (__builtin_cpu_supports("rdpid")) {
        int agree = 0;
        for (int i = 0; i < 100 && !agree; i++) {
            unsigned int aux;
            (void)__rdtscp(&aux);
            agree = _rdpid_u32() == aux;
        }
        printf("rdpid %s\n", agree ? "agrees with rdtscp" : "DISAGREES");
    } else {
        printf("rdpid absent\n");
    }

    static char line[256] __attribute__((aligned(64)));
    memset(line, 0x5a, sizeof line);
    if (__builtin_cpu_supports("clflushopt")) {
        _mm_clflushopt(line + 64);
        printf("clflushopt %d\n", line[64] == 0x5a && line[127] == 0x5a);
    } else {
        printf("clflushopt absent\n");
    }
    if (__builtin_cpu_supports("clwb")) {
        _mm_clwb(line + 128);
        printf("clwb %d\n", line[128] == 0x5a && line[191] == 0x5a);
    } else {
        printf("clwb absent\n");
    }

    /* XSAVEOPT and XSAVEC of the x87 and SSE state: the header's XSTATE_BV
     * (offset 512) names nothing beyond what was asked for, and XSAVEC's
     * XCOMP_BV (offset 520) has the compacted-format bit 63. */
    static unsigned char area[4096] __attribute__((aligned(64)));
    if (__builtin_cpu_supports("xsaveopt")) {
        memset(area, 0, sizeof area);
        _xsaveopt(area, 3);
        unsigned long long bv;
        memcpy(&bv, area + 512, 8);
        printf("xsaveopt %d\n", (bv & ~3ull) == 0);
    } else {
        printf("xsaveopt absent\n");
    }
    if (__builtin_cpu_supports("xsavec")) {
        memset(area, 0, sizeof area);
        _xsavec(area, 3);
        unsigned long long bv, comp;
        memcpy(&bv, area + 512, 8);
        memcpy(&comp, area + 520, 8);
        printf("xsavec %d %d\n", (bv & ~3ull) == 0, (int)(comp >> 63));
    } else {
        printf("xsavec absent\n");
    }

    /* The shadow-stack pointer: zero in a process without shadow stacks,
     * where RDSSP does nothing to the register the intrinsic cleared. */
    printf("ssp %s\n", _get_ssp() == 0 ? "zero" : "nonzero");
    return 0;
}
