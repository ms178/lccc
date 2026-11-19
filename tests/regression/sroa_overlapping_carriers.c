/* An integer view of bytes [1,3) and a view of [0,8) both need a padding
 * carrier at offset 2, but with different widths. Sharing it must not lose
 * byte 3 or let the shorter write overwrite that byte. */
struct __attribute__((packed)) short_view {
    unsigned char prefix;
    unsigned short bits;
};
struct __attribute__((packed)) byte_view {
    unsigned char prefix;
    unsigned char bits;
};
union image {
    unsigned long long full;
    struct short_view s;
    struct byte_view b;
};

__attribute__((noinline)) unsigned long long merge(unsigned long long seed, unsigned short bits)
{
    union image u = { .full = seed };
    unsigned long long saved = u.b.bits;
    u.s.bits = bits;
    return u.full ^ (saved << 48);
}

int main(void)
{
    volatile unsigned long long seed = 0x4433221188776655ull;
    volatile unsigned short bits = 0xeeaau;
    return merge(seed, bits) != 0x4455221188eeaa55ull;
}
