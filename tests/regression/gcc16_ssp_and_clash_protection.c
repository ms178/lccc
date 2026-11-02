#include <stdio.h>
#include <string.h>

#if !__has_attribute(no_stack_protector)
#error "missing __has_attribute(no_stack_protector)"
#endif

__attribute__((noinline))
static int fill_protected_buffer(int seed) {
    char buf[64];
    for (int i = 0; i < 64; i++) {
        buf[i] = (char)((seed + i * 3) & 0x7f);
    }
    int acc = 0;
    for (int i = 0; i < 64; i++) {
        acc += buf[i];
    }
    return acc;
}

/* Multi-page static stack frame (> 16 KiB) exercising -fstack-clash-protection page probing. */
__attribute__((noinline))
static int multi_page_frame(int x) {
    volatile unsigned char pages[20480];
    pages[0] = (unsigned char)x;
    pages[4096] = (unsigned char)(x + 1);
    pages[8192] = (unsigned char)(x + 2);
    pages[12288] = (unsigned char)(x + 3);
    pages[16384] = (unsigned char)(x + 4);
    pages[20479] = (unsigned char)(x + 5);
    return (int)pages[0] + (int)pages[4096] + (int)pages[8192]
         + (int)pages[12288] + (int)pages[16384] + (int)pages[20479];
}

/* Dynamic VLA (> 8 KiB) exercising runtime stack-clash probing + SSP canary epilogue check. */
__attribute__((noinline))
static int dynamic_vla_probe(int n, int seed) {
    volatile int vla[n];
    vla[0] = seed;
    vla[n / 2] = seed * 2;
    vla[n - 1] = seed * 3;
    return vla[0] + vla[n / 2] + vla[n - 1];
}

__attribute__((noinline, no_stack_protector))
static int unguarded_leaf(int a, int b) {
    int tmp[8];
    for (int i = 0; i < 8; i++) tmp[i] = a + b + i;
    return tmp[0] + tmp[7];
}

int main(void) {
    int a = fill_protected_buffer(5);
    int b = multi_page_frame(10);
    int c = dynamic_vla_probe(2500, 7);
    int d = unguarded_leaf(3, 4);
    printf("%d %d %d %d\n", a, b, c, d);
    return 0;
}
