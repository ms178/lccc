/* Semantics of the bundled scalar x86 intrinsics (BMI1, BMI2, LZCNT,
 * ia32 helpers) over edge inputs; stdout must match GCC on its headers. */
#include <immintrin.h>
#include <stdio.h>

static const unsigned long long in64[] = {
    0, 1, 2, 3, 0x80, 0x8000, 0x80000000ULL, 0xFFFFFFFFULL, 0x100000000ULL,
    0x8000000000000000ULL, 0xFFFFFFFFFFFFFFFFULL, 0x00F0F0F0F0F0F0F0ULL,
    0x123456789ABCDEF0ULL, 0x0000000100000000ULL, 0x7FFFFFFFFFFFFFFFULL,
};
#define N (sizeof in64 / sizeof in64[0])
static const int rot[] = {0, 1, 7, 8, 15, 16, 31, 32, 33, 63, 64, 65, -1, -33};
static const unsigned ctl[] = {0, 0x0800, 0x0804, 0x2000, 0x1F01, 0x2001,
                               0x401C, 0xFF00, 0x0020, 0x00FF, 0x3F3F, 0x4000};

int main(void)
{
    volatile unsigned long long sink;
    for (unsigned i = 0; i < N; i++) {
        volatile unsigned long long vx = in64[i];
        unsigned long long x = vx;
        unsigned x32 = (unsigned)x;
        unsigned short x16 = (unsigned short)x;
        printf("%2u tz %u %u %u lz %u %u %u", i, (unsigned)__tzcnt_u16(x16),
               _tzcnt_u32(x32), (unsigned)__lzcnt16(x16), _lzcnt_u32(x32),
               __lzcnt32(x32), (unsigned)__tzcnt_u32(x32));
        printf(" bl %x %x %x an %x pc %d", _blsi_u32(x32), _blsmsk_u32(x32),
               _blsr_u32(x32), __andn_u32(x32, 0x0FF00FF0u), _popcnt32((int)x32));
        printf(" bs %x", __bswapd((int)x32));
        if (x32)
            printf(" sc %d %d", _bit_scan_forward((int)x32), _bit_scan_reverse((int)x32));
        printf(" pd %x %x", _pdep_u32(x32, 0xF0F00F0Fu), _pext_u32(x32, 0xF0F00F0Fu));
        printf(" bz %x %x", _bzhi_u32(x32, 5), _bzhi_u32(x32, 40));
#ifdef __x86_64__
        printf(" q %llu %llu %llx %llx %llx %lld", _tzcnt_u64(x), _lzcnt_u64(x),
               _blsi_u64(x), _blsmsk_u64(x), _blsr_u64(x), _popcnt64((long long)x));
        printf(" %llx %llx %llx", _pdep_u64(x, 0xF0F0F0F00F0F0F0FULL),
               _pext_u64(x, 0xF0F0F0F00F0F0F0FULL), __andn_u64(x, ~0ULL >> 4));
        printf(" %llx", (unsigned long long)_bswap64((long long)x));
        if (x)
            printf(" %d %d", __bsfq((long long)x), __bsrq((long long)x));
#endif
        for (unsigned c = 0; c < sizeof ctl / sizeof ctl[0]; c++) {
            printf(" e%x", __bextr_u32(x32, ctl[c]));
#ifdef __x86_64__
            printf(".%llx", __bextr_u64(x, ctl[c]));
#endif
        }
        printf(" E%x", _bextr_u32(x32, 4, 8));
        for (unsigned r = 0; r < sizeof rot / sizeof rot[0]; r++) {
            int n = rot[r];
            printf(" r%x.%x.%x.%x.%x.%x", __rold(x32, n), __rord(x32, n),
                   (unsigned)__rolw(x16, n), (unsigned)__rorw(x16, n),
                   (unsigned)__rolb((unsigned char)x, n), (unsigned)__rorb((unsigned char)x, n));
#ifdef __x86_64__
            printf(".%llx.%llx", __rolq(x, n), __rorq(x, n));
#endif
        }
        printf("\n");
        sink = x;
    }
    /* Compile-time constants: the folded path must agree too. */
    printf("k %u %u %u %u %u %x %x %x %d %d\n", _tzcnt_u32(0), _lzcnt_u32(0),
           _tzcnt_u32(0x100), _lzcnt_u32(1), (unsigned)__lzcnt16(0),
           __bextr_u32(0xDEADBEEF, 0x0804), _rotl(0x80000001u, 32),
           _rotr(0x80000001u, 0), _bit_scan_reverse(0x10), _bit_scan_forward(0x10));
    (void)sink;
    return 0;
}
