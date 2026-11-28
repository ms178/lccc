/* x86_gpr_intrinsics_runtime.c for 32-bit code: the forms i686 has. The
 * 64-bit members of each family do not exist here; the 32-bit ones are the
 * same functions, and the flags register is read and written as 32 bits
 * through `pushfl`/`popfl` with no red zone to step around. */
#include <immintrin.h>
#include <stdio.h>
#include <string.h>

static unsigned int state = 0x9e3779b9u;
static unsigned int next(void)
{
    state ^= state << 13;
    state ^= state >> 17;
    state ^= state << 5;
    return state;
}

static int carry_chains(void)
{
    static const unsigned int edge[] = {0, 1, 2, 0x7fffffffu, 0x80000000u, 0xfffffffeu, 0xffffffffu};
    enum { EDGES = sizeof edge / sizeof edge[0] };
    int bad = 0;
    for (int round = 0; round < 4000 + EDGES * EDGES * 2; round++) {
        unsigned int x, y, sum, diff;
        unsigned char cf;
        if (round < EDGES * EDGES * 2) {
            x = edge[round % EDGES];
            y = edge[round / EDGES % EDGES];
            cf = (unsigned char)(round / (EDGES * EDGES) ? 0x80 : 0);
        } else {
            x = next();
            y = next();
            cf = (unsigned char)(next() & 3);
        }
        unsigned char c = _addcarry_u32(cf, x, y, &sum);
        unsigned long long w = (unsigned long long)x + y + (cf != 0);
        bad += sum != (unsigned int)w || c != (unsigned char)(w >> 32);
        c = _addcarryx_u32(cf, x, y, &sum);
        bad += sum != (unsigned int)w || c != (unsigned char)(w >> 32);
        c = _subborrow_u32(cf, x, y, &diff);
        w = (unsigned long long)x - y - (cf != 0);
        bad += diff != (unsigned int)w || c != (unsigned char)((w >> 32) & 1);
    }
    unsigned int a[4] = {~0u, ~0u, ~0u, 1}, b[4] = {1, 0, 0, 2}, r[4];
    unsigned char c = 0;
    for (int i = 0; i < 4; i++)
        c = _addcarry_u32(c, a[i], b[i], &r[i]);
    bad += !(r[0] == 0 && r[1] == 0 && r[2] == 0 && r[3] == 4 && c == 0);
    c = 0;
    for (int i = 0; i < 4; i++)
        c = _subborrow_u32(c, r[i], b[i], &r[i]);
    bad += memcmp(r, a, sizeof r) != 0 || c != 0;
    return bad;
}

__attribute__((noinline)) static unsigned int flags_after_compare(long a, long b)
{
    unsigned int before = __readeflags();
    __writeeflags((before | 1) & ~0x40u);
    unsigned int after = __readeflags();
    __writeeflags(before);
    return (after & 0x41) + (unsigned int)(a < b);
}

int main(void)
{
    printf("carry chains: %d wrong\n", carry_chains());
    printf("eflags: %u\n", flags_after_compare(1, 2));
    printf("andn %x tzcnt16 %u %u\n", _andn_u32(0xf0f0f0f0u, 0xffff0000u), _tzcnt_u16(0x80),
           _tzcnt_u16(0));
    if (__builtin_cpu_supports("sse4.2")) {
        unsigned int c = __crc32b(0, 'a');
        c = __crc32w(c, 0x6362);
        printf("crc32c %08x\n", __crc32d(c, 0x67666564));
    } else {
        printf("crc32c absent\n");
    }
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
    static unsigned char area[4096] __attribute__((aligned(64)));
    if (__builtin_cpu_supports("xsaveopt")) {
        _xsaveopt(area, 3);
        unsigned long long bv;
        memcpy(&bv, area + 512, 8);
        printf("xsaveopt %d\n", (bv & ~3ull) == 0);
    } else {
        printf("xsaveopt absent\n");
    }
    printf("ssp %s\n", _get_ssp() == 0 ? "zero" : "nonzero");
    return 0;
}
