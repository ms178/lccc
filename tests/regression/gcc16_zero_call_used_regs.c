#include <stdio.h>

#if !__has_attribute(zero_call_used_regs)
#error "missing __has_attribute(zero_call_used_regs)"
#endif

__attribute__((noinline, zero_call_used_regs("all-gpr")))
long zero_all_gpr_callee(long a, long b) {
    return a * 3 + b + 11;
}

__attribute__((noinline, zero_call_used_regs("used-gpr-arg")))
long zero_arg_gpr_callee(long a, long b, long c) {
    return (a ^ b) + c;
}

__attribute__((noinline, zero_call_used_regs("leafy-gpr")))
long leafy_nonleaf(long x) {
    return zero_arg_gpr_callee(x, x + 1, x + 2) + 5;
}

__attribute__((noinline, zero_call_used_regs("skip")))
long skipped_callee(long x) {
    return x * 9;
}

/* Exercises caller live-range preservation across a zero_call_used_regs("all-gpr") call. */
__attribute__((noinline))
long caller_live_across(long x, long y, long z) {
    long before = x * 13 + y * 7;
    long mid = zero_all_gpr_callee(x, z);
    long leaf = leafy_nonleaf(y);
    return before + mid + leaf + skipped_callee(z);
}

int main(void) {
#if defined(__x86_64__)
    long ret, rcx_out, rdx_out, rsi_out, rdi_out;
    __asm__ __volatile__(
        "movq $10, %%rdi\n\t"
        "movq $20, %%rsi\n\t"
        "movq $0x13572468, %%rdx\n\t"
        "movq $0x24681357, %%rcx\n\t"
        "call zero_all_gpr_callee\n\t"
        : "=a"(ret), "=c"(rcx_out), "=d"(rdx_out), "=S"(rsi_out), "=D"(rdi_out)
        :
        : "r8", "r9", "r10", "r11", "cc", "memory"
    );
    if (ret != 61) return 1;
    if (rcx_out != 0 || rdx_out != 0 || rsi_out != 0 || rdi_out != 0) return 2;
#endif
    long total = caller_live_across(4, 5, 6);
    printf("%ld\n", total);
    return 0;
}
