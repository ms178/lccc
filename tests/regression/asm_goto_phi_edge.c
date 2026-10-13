#include <stdint.h>
#include <stdio.h>

__attribute__((noinline)) unsigned asm_phi(unsigned mode, unsigned skip) {
    unsigned result = 9;
    if (skip) goto relay;
#if defined(__x86_64__) || defined(__i386__)
    __asm__ goto("testl %0, %0; jnz %l[join]" : : "r"(mode) : "cc" : join);
#elif defined(__aarch64__)
    __asm__ goto("cbnz %w0, %l[join]" : : "r"(mode) : : join);
#elif defined(__riscv)
    __asm__ goto("bnez %0, %l[join]" : : "r"(mode) : : join);
#else
#error unsupported target
#endif
relay:
    result = 1;
join:
    return result;
}

__attribute__((noinline)) unsigned relay_phi(unsigned mode, unsigned choose) {
    unsigned x;
    if (mode) { x = choose ? 11 : 19; goto a; }
    x = 23; goto b;
a: goto c;
b: goto c;
c: return x;
}

__attribute__((noinline)) uint64_t wide_phi(unsigned mode) {
    uint64_t x;
    if (mode) { x = UINT64_C(0x100000001); goto a; }
    x = UINT64_C(1); goto b;
a: goto c;
b: goto c;
c: return x;
}

static uint64_t hash = UINT64_C(14695981039346656037);
static void feed(uint64_t x) {
    unsigned i;
    for (i = 0; i < 8; ++i) { hash ^= x & 255; hash *= UINT64_C(1099511628211); x >>= 8; }
}
int main(void) {
    unsigned i, j;
    for (i = 0; i < 32; ++i) for (j = 0; j < 32; ++j) {
        unsigned a = asm_phi(i, j);
        unsigned b = relay_phi(i, j);
        uint64_t c = wide_phi(i);
        if (a != ((i && !j) ? 9u : 1u) || b != (i ? (j ? 11u : 19u) : 23u)
            || c != (i ? UINT64_C(0x100000001) : UINT64_C(1))) {
            printf("FAIL %u %u: %u %u %llu\n", i, j, a, b, (unsigned long long)c);
            return 1;
        }
        feed(a); feed(b); feed(c);
    }
    printf("%016llx\n", (unsigned long long)hash);
    return 0;
}
