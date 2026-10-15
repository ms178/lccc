#include <stdint.h>
#include <stdio.h>
extern uint64_t masked_merge_original(void), masked_merge_illegal(void);
extern uint64_t masked_zero_original(void), masked_zero_illegal(void);
int main(void) {
    if (!__builtin_cpu_supports("avx512f") || !__builtin_cpu_supports("avx512vl")) {
        puts("SKIP: native masked-copy witnesses require AVX-512F/VL; Rust legality tests remain mandatory");
        return 0;
    }
    uint64_t a = masked_merge_original(), b = masked_merge_illegal();
    uint64_t c = masked_zero_original(), d = masked_zero_illegal();
    printf("masked copies: merge original=%llx illegal=%llx; zero original=%llx illegal=%llx\n",
           (unsigned long long)a, (unsigned long long)b,
           (unsigned long long)c, (unsigned long long)d);
    return a != UINT64_MAX || b != 0 || c != 0 || d != UINT64_MAX;
}
