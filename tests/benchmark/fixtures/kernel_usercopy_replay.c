/* Native userspace replay of LCCC-built kernel object, NOT a kernel boot or
 * privileged-SMAP benchmark. No GCC compiles either target or this harness. */
#ifndef REPLAY_ITERATIONS
#define REPLAY_ITERATIONS 10000000UL
#endif
extern int check_zeroed_user(const void *, unsigned long);
unsigned long __ref_stack_chk_guard;
static long sys3(long nr, long a, long b, long c) {
    long r;
    __asm__ volatile("syscall" : "=a"(r) : "a"(nr), "D"(a), "S"(b), "d"(c) : "rcx", "r11", "memory", "cc");
    return r;
}
__attribute__((noreturn)) static void fail(long code) { sys3(60, code, 0, 0); __builtin_unreachable(); }
void __stack_chk_fail(void) { fail(98); }
void rep_movs_alternative(void) { fail(99); }
void *memset(void *p, int c, unsigned long n) {
    volatile unsigned char *q = p;
    for (unsigned long i = 0; i < n; ++i) q[i] = c;
    return p;
}
static unsigned char buffer[256] __attribute__((aligned(64)));
struct ts { long s, ns; };
int main(void) {
    unsigned long checks = 0;
    for (unsigned long align = 0; align < 8; ++align) {
        unsigned char *p = buffer + 16 + align;
        for (unsigned long n = 0; n <= 32; ++n) {
            memset(buffer, 0, sizeof buffer);
            p[-1] = 1; p[n] = 1;
            if (check_zeroed_user(p, n) != 1) fail(1);
            ++checks;
            for (unsigned long j = 0; j < n; ++j) {
                p[j] = 1;
                if (check_zeroed_user(p, n) != 0) fail(2);
                p[j] = 0;
                ++checks;
            }
        }
    }
    memset(buffer, 0, sizeof buffer);
    struct ts a, b;
    if (sys3(228, 1, (long)&a, 0)) fail(3);
    unsigned long sum = 0;
    for (unsigned long i = 0; i < REPLAY_ITERATIONS; ++i) {
        sum += check_zeroed_user(buffer + 16 + (i & 7), 1 + ((i >> 3) & 15));
    }
    if (sys3(228, 1, (long)&b, 0)) fail(4);
    if (sum != REPLAY_ITERATIONS || checks != 4488) fail(5);
    unsigned long ns = (unsigned long)(b.s - a.s) * 1000000000UL + (unsigned long)(b.ns - a.ns);
    char out[32]; unsigned count = 0;
    do { out[count++] = '0' + ns % 10; ns /= 10; } while (ns);
    for (unsigned i = 0; i < count / 2; ++i) { char t=out[i]; out[i]=out[count-1-i];out[count-1-i]=t; }
    out[count++]='\n';
    if (sys3(1, 1, (long)out, count) != (long)count) fail(6);
    return 0;
}
