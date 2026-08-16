/* Compile-time __builtin_cpu_supports must match Raptor Lake / x86-64-v3. */
int main(void) {
    if (!__builtin_cpu_supports("sse2")) return 1;
    if (!__builtin_cpu_supports("avx2")) return 2;
    if (!__builtin_cpu_supports("bmi2")) return 3;
    if (!__builtin_cpu_supports("fma"))  return 4;
    if (!__builtin_cpu_supports("aes"))  return 5;
    if (__builtin_cpu_supports("avx512f")) return 6;
    if (__builtin_cpu_supports("avx512bw")) return 7;
    if (__builtin_cpu_supports("amx-tile")) return 8;
    if (__builtin_cpu_supports("sse4a")) return 9;
    if (__builtin_cpu_supports("xop")) return 10;
    return 0;
}
