/* Peephole liveness — x86 string instructions name no register in their
 * text but read/write %rcx/%rsi/%rdi/%rax.  Before the fix the backward
 * dataflow treated `movq $N, %rcx; rep movsb` as a dead write and deleted
 * the count set-up.  Inline asm keeps this independent of the memcpy
 * lowering; the compiler-generated `rep stosb`/`rep movsb` forms are covered
 * by cpu_model_memcpy_raptorlake.c. */
#include <stdio.h>
#include <string.h>

__attribute__((noinline)) void copy_rep(void *d, const void *s, unsigned long n) {
    __asm__ volatile("rep movsb" : "+D"(d), "+S"(s), "+c"(n) : : "memory");
}
__attribute__((noinline)) void fill_rep(void *d, int v, unsigned long n) {
    __asm__ volatile("rep stosb" : "+D"(d), "+c"(n) : "a"(v) : "memory");
}
__attribute__((noinline)) unsigned long scan_rep(const void *p, int v, unsigned long n) {
    unsigned long left = n;
    __asm__ volatile("repne scasb" : "+D"(p), "+c"(left) : "a"(v) : "memory");
    return n - left;
}

static unsigned char src[3000], dst[3000];

int main(void) {
    for (int i = 0; i < 3000; i++) src[i] = (unsigned char)(i * 13 + 5);
    src[2500] = 0xEE;
    fill_rep(dst, 0x7F, sizeof dst);
    unsigned long fills = 0;
    for (int i = 0; i < 3000; i++) fills += dst[i] == 0x7F;
    copy_rep(dst, src, 2999);
    unsigned long same = memcmp(dst, src, 2999) == 0;
    unsigned long pos = scan_rep(src, 0xEE, 3000);
    printf("%lu %lu %lu %u\n", fills, same, pos, dst[2999]);
    return 0;
}
