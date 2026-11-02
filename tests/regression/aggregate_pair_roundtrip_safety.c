/* Scalar carriers may share users, sign-extend, or copy overlapping objects.
 * None of those contracts may be replaced with an unproven 128-bit copy.
 * No libc dependency: the same fixture can run freestanding on every target. */
typedef unsigned long long u64;
typedef unsigned __int128 u128;

__attribute__((noinline)) static u64 copy_shared(u64 *dst, const u64 *src)
{
    u64 low = src[0];
    u128 bits = ((u128)src[1] << 64) | (u128)low;
    dst[0] = (u64)bits;
    dst[1] = (u64)(bits >> 64);
    return low ^ (u64)(bits >> 64);
}

__attribute__((noinline)) static void copy_signed_low(u64 *dst, const u64 *src)
{
    u128 bits = ((u128)src[1] << 64) | (u128)(long long)src[0];
    dst[0] = (u64)bits;
    dst[1] = (u64)(bits >> 64);
}

__attribute__((noinline)) static u64 observe_first_store(u64 *dst, const u64 *src,
                                                       const volatile u64 *observer)
{
    u128 bits = ((u128)src[1] << 64) | (u128)src[0];
    dst[0] = (u64)bits;
    u64 seen = *observer;
    dst[1] = (u64)(bits >> 64);
    return seen;
}

static volatile u64 inputs[] = {
    0, 1, 0x7fffffffffffffffULL, 0x8000000000000000ULL,
    0xffffffffffffffffULL, 0x0123456789abcdefULL
};

int main(void)
{
    for (unsigned i = 0; i < sizeof(inputs) / sizeof(inputs[0]); ++i) {
        u64 low = inputs[i], high = inputs[(i + 1) % 6];
        u64 src[2] = {low, high}, dst[2] = {7, 9};
        if (copy_shared(dst, src) != (low ^ high)
            || dst[0] != low || dst[1] != high)
            return 1;
        copy_signed_low(dst, src);
        if (dst[0] != low || dst[1] != (low >> 63 ? ~0ULL : high))
            return 2;
        if (observe_first_store(dst, src, dst) != low
            || dst[0] != low || dst[1] != high)
            return 3;
        u64 overlap[4] = {low, high, 17, 19};
        if (copy_shared(overlap + 1, overlap) != (low ^ high)
            || overlap[1] != low || overlap[2] != high)
            return 4;
        if (copy_shared(overlap, overlap + 1) != (low ^ high)
            || overlap[0] != low || overlap[1] != high)
            return 5;
    }
    return 0;
}
