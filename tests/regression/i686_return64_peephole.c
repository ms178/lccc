__attribute__((noinline)) static unsigned long long wide(unsigned x)
{
    return ((unsigned long long)(x + 1) << 32) | (x ^ 0x12345678u);
}

void _start(void)
{
    unsigned long long value = wide(7);
    int ok = (unsigned)value == 0x1234567fu
             && (unsigned)(value >> 32) == 8;
    __asm__ volatile("int $0x80" :: "a"(1), "b"(ok ? 0 : 1) : "memory");
    __builtin_unreachable();
}
