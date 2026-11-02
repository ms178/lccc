#include <stdio.h>
/* Bit-exact models of the three emitted sequences, run natively. */
static unsigned par32(unsigned x) {
    unsigned r;
    __asm__ ("movl %%eax, %%ecx\n\tshrl $16, %%ecx\n\txorl %%ecx, %%eax\n\t"
             "xorb %%ah, %%al\n\tsetnp %%al\n\tmovzbl %%al, %%eax"
             : "=a" (r) : "a" (x) : "rcx", "cc");
    return r;
}
static unsigned par64(unsigned long x) {
    unsigned long r;
    __asm__ ("movq %%rax, %%rcx\n\tshrq $32, %%rcx\n\txorq %%rcx, %%rax\n\t"
             "movq %%rax, %%rcx\n\tshrq $16, %%rcx\n\txorl %%ecx, %%eax\n\t"
             "xorb %%ah, %%al\n\tsetnp %%al\n\tmovzbl %%al, %%eax"
             : "=a" (r) : "a" (x) : "rcx", "cc");
    return r;
}
/* i686 64-bit wrapper: fold edx into eax first, then the 32-bit sequence. */
static unsigned par64_wrap(unsigned lo, unsigned hi) {
    unsigned r;
    __asm__ ("xorl %%edx, %%eax\n\tmovl %%eax, %%ecx\n\tshrl $16, %%ecx\n\t"
             "xorl %%ecx, %%eax\n\txorb %%ah, %%al\n\tsetnp %%al\n\t"
             "movzbl %%al, %%eax"
             : "=a" (r) : "a" (lo), "d" (hi) : "rcx", "cc");
    return r;
}
int main(void) {
    unsigned bad = 0;
    for (unsigned x = 0; x < 65536u; x++) {
        if (par32(x) != __builtin_parity(x)) bad++;
        if (par64(x) != __builtin_parityll((unsigned long)x)) bad++;
        if (par64_wrap(x, 0) != __builtin_parityll((unsigned long)x)) bad++;
        if (par64_wrap(0, x) != __builtin_parityll((unsigned long)x << 32)) bad++;
    }
    for (unsigned long i = 0; i < 3000000ul; i++) {
        unsigned long v = (unsigned long)(i * 2654435761ul) * 0x9e3779b97f4a7c15ul;
        if (par64(v) != __builtin_parityll(v)) bad++;
        unsigned long w = (unsigned long)i * 0x2545f4914f6cdd1dul;
        if (par64_wrap((unsigned)w, (unsigned)(w >> 32)) != __builtin_parityll(w)) bad++;
        unsigned u = (unsigned)(v ^ (v >> 32));
        if (par32(u) != __builtin_parity(u)) bad++;
    }
    printf("mismatches=%u\n", bad);
    return bad != 0;
}
