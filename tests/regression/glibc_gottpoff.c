/* glibc_gottpoff.c — lowercase @gottpoff TLS relocation (glibc multiarch
 * strcmp-sse2.S uses `movq __libc_tsd_LOCALE@gottpoff(%rip)`). LCCC used to
 * emit R_X86_64_PC32 (modifier case-sensitive) -> "TLS definition mismatches
 * non-TLS reference" at the libc_pic link. Verifies the TLS IE sequence
 * assembles to R_X86_64_GOTTPOFF and works at runtime. */
#include <stdio.h>

__thread int tls_var = 42;

static int __attribute__((noinline)) read_tls(void) {
    int r;
    __asm__ ("movq tls_var@gottpoff(%%rip), %%rax\n\t"
             "movl %%fs:(%%rax), %0"
             : "=r"(r));
    return r;
}

int main(void) {
    if (read_tls() != 42) { printf("FAIL gottpoff tls\n"); return 1; }
    tls_var = 7;
    if (read_tls() != 7) { printf("FAIL gottpoff tls 2\n"); return 1; }
    printf("PASS gottpoff\n");
    return 0;
}
