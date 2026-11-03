/* Standalone control-flow ABI fixture, not Linux target code. A side-effecting
 * always-inline helper returns exactly 0/1; its caller narrows and branches.
 * Narrowing a different integer domain must NOT inherit that truth proof. */
static volatile unsigned state;
static __attribute__((always_inline)) inline int begin(int valid) {
    if (!valid) return 0;
    state |= 1;
    __asm__ volatile("" ::: "memory");
    return 1;
}
__attribute__((noinline)) int guarded(int n) {
    if (!(unsigned char)begin(n)) return -14;
    state |= 2;
    __asm__ volatile("" ::: "memory");
    return 7;
}
__attribute__((noinline)) int narrow8(int n) {
    int v;
    if (n) { __asm__ volatile("" ::: "memory"); v = 256; }
    else v = 1;
    return (unsigned char)v != 0;
}
__attribute__((noinline)) int narrow16(int n) {
    int v;
    if (n) { __asm__ volatile("" ::: "memory"); v = 65536; }
    else v = 1;
    return (unsigned short)v != 0;
}
int main(void) {
    static const int inputs[] = {0, 1, -1, 2, 255, 256, 65535, 65536,
                                2147483647, -2147483647 - 1};
    for (unsigned i = 0; i < sizeof(inputs) / sizeof(inputs[0]); ++i) {
        int n = inputs[i];
        state = 0;
        if (guarded(n) != (n ? 7 : -14)) return 1;
        if (state != (n ? 3u : 0u)) return 2;
        if (narrow8(n) != !n) return 3;
        if (narrow16(n) != !n) return 4;
    }
    return 0;
}
