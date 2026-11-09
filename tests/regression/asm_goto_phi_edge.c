/* Asm-goto PHI-edge regression.
 *
 * Deliberately HEADER-FREE.  This file is compiled for both -m64 and -m32;
 * <stdint.h>/<stdio.h> pull in glibc's multiarch bits/ tree, which a host
 * without libc6-dev-i386 does not have.  That turned a pure assembler-shape
 * regression into an environment-dependent gate failure.  Nothing here needs
 * a libc header: the 64-bit hash type is spelled out and printf is declared
 * locally (the C prototype is identical on LP64 and ILP32).
 *
 * Keep the hash arithmetic bit-identical to the original fixture: the gate
 * pins the exact 16-hex-digit result fd6c281bebb77835.
 */
typedef unsigned long long u64;
extern int printf(const char *, ...);

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

__attribute__((noinline)) u64 wide_phi(unsigned mode) {
    u64 x;
    if (mode) { x = 0x100000001ULL; goto a; }
    x = 1ULL; goto b;
a: goto c;
b: goto c;
c: return x;
}

static u64 hash = 14695981039346656037ULL;
static void feed(u64 x) {
    unsigned i;
    for (i = 0; i < 8; ++i) { hash ^= x & 255; hash *= 1099511628211ULL; x >>= 8; }
}
int main(void) {
    unsigned i, j;
    for (i = 0; i < 32; ++i) for (j = 0; j < 32; ++j) {
        unsigned a = asm_phi(i, j);
        unsigned b = relay_phi(i, j);
        u64 c = wide_phi(i);
        if (a != ((i && !j) ? 9u : 1u) || b != (i ? (j ? 11u : 19u) : 23u)
            || c != (i ? 0x100000001ULL : 1ULL)) {
            printf("FAIL %u %u: %u %u %llu\n", i, j, a, b, (unsigned long long)c);
            return 1;
        }
        feed(a); feed(b); feed(c);
    }
    printf("%016llx\n", (unsigned long long)hash);
    return 0;
}
