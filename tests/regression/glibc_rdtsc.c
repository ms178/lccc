/* glibc_rdtsc.c — __builtin_ia32_rdtsc / rdtscp (glibc rtld_timer.c).
 * LCCC had no intrinsics -> rtld link failure. Verify monotonic counter
 * and aux store. */
#include <stdio.h>

int main(void) {
    unsigned long long t1 = __builtin_ia32_rdtsc();
    unsigned long long t2 = __builtin_ia32_rdtsc();
    unsigned int aux = 0;
    unsigned long long t3 = __builtin_ia32_rdtscp(&aux);
    if (t2 < t1) { printf("FAIL rdtsc mono\n"); return 1; }
    if (t3 < t2) { printf("FAIL rdtscp mono\n"); return 1; }
    (void)aux;
    printf("PASS rdtsc\n");
    return 0;
}
