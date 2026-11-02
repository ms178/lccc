/* LCCC extension names: GCC rejects these spellings. The canonical-name
 * runtime CPUID differential remains in builtin_cpu_supports_default.c. */
int main(void) {
    if (!!__builtin_cpu_supports("sse4_2") != !!__builtin_cpu_supports("sse4.2")) return 1;
    if (!!__builtin_cpu_supports("cx16") != !!__builtin_cpu_supports("cmpxchg16b")) return 2;
    return 0;
}
