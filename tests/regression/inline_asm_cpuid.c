/* v4 regression: inline asm — cpuid with memory-output operands + matching
 * constraints (the v1 bug class: previously wrong slots at -O1+ and SIGSEGV
 * at -O0), plus simple asm blocks with clobbers. */
#include <stdio.h>

static unsigned long long rdtsc(void) {
    unsigned lo, hi;
    __asm__ volatile("rdtsc" : "=a"(lo), "=d"(hi));
    return ((unsigned long long)hi << 32) | lo;
}

static int cpuid_vendor(char *out) {
    unsigned a, b, c, d;
    __asm__ volatile("cpuid" : "=a"(a), "=b"(b), "=c"(c), "=d"(d) : "a"(0) : "memory");
    /* vendor string "GenuineIntel" or "AuthenticAMD" in ebx:edx:ecx */
    out[0] = (char)(b & 0xFF); out[1] = (char)((b >> 8) & 0xFF); out[2] = (char)((b >> 16) & 0xFF); out[3] = (char)((b >> 24) & 0xFF);
    out[4] = (char)(d & 0xFF); out[5] = (char)((d >> 8) & 0xFF); out[6] = (char)((d >> 16) & 0xFF); out[7] = (char)((d >> 24) & 0xFF);
    out[8] = (char)(c & 0xFF); out[9] = (char)((c >> 8) & 0xFF); out[10] = (char)((c >> 16) & 0xFF); out[11] = (char)((c >> 24) & 0xFF);
    out[12] = 0;
    return 0;
}

static int add_asm(int x, int y) {
    int r;
    __asm__ volatile("addl %2, %0" : "=r"(r) : "0"(x), "r"(y) : "cc");
    return r;
}

static int use_rcx(int x) {
    int r;
    __asm__ volatile("imull %2, %0" : "=a"(r) : "a"(x), "c"(5) : "cc");
    return r;
}

int main(void) {
    char v[16];
    cpuid_vendor(v);
    /* vendor must be non-empty and printable ASCII */
    if (v[0] < 32 || v[0] > 126) return 1;
    if (v[1] < 32 || v[1] > 126) return 2;

    if (add_asm(10, 32) != 42) return 3;
    if (use_rcx(7) != 35) return 4;

    unsigned long long t0 = rdtsc(), t1 = rdtsc();
    if (t1 < t0) return 5;   /* timestamps non-decreasing */

    /* asm with memory clobber + early-clobber */
    int arr[4] = {1, 2, 3, 4};
    int sum = 0;
    __asm__ volatile(
        "movl %1, %0\n\t"
        "addl %2, %0\n\t"
        "addl %3, %0\n\t"
        "addl %4, %0"
        : "=&r"(sum)
        : "m"(arr[0]), "m"(arr[1]), "m"(arr[2]), "m"(arr[3])
        : "memory");
    if (sum != 10) return 6;

    printf("OK inline_asm_cpuid\n");
    return 0;
}
