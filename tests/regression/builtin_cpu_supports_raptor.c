/* Compile-time __builtin_cpu_supports must match the invocation's PERMISSION,
 * not a hard-coded CPU.  AUD-CPU-1: the default target level is x86-64-v3, so
 * every v3 feature folds to 1 while the ones no microarchitecture level
 * implies (AES-NI, PCLMUL, AVX-512, AMX, ...) fold to 0 unless the invocation
 * asks for them.  `-maes` / `-march=raptorlake` grant AES and PCLMUL, and
 * tests/regression/check_cpu_supports_fold.sh pins that half (with the folded
 * constant measured in the emitted code, not by running a host-dependent
 * binary). */
int main(void) {
    if (!__builtin_cpu_supports("sse2")) return 1;
    if (!__builtin_cpu_supports("avx2")) return 2;
    if (!__builtin_cpu_supports("bmi2")) return 3;
    if (!__builtin_cpu_supports("fma"))  return 4;
    /* AES is not part of x86-64-v1/v2/v3: the default must NOT promise it, or
     * runtime dispatch takes an AES path the build was never allowed to
     * assume (this is the SIGILL class AUD-CPU-1 fixed). */
    if (__builtin_cpu_supports("aes"))   return 5;
    if (__builtin_cpu_supports("pclmul")) return 6;
    if (__builtin_cpu_supports("avx512f")) return 7;
    if (__builtin_cpu_supports("avx512bw")) return 8;
    if (__builtin_cpu_supports("amx-tile")) return 9;
    if (__builtin_cpu_supports("sse4a")) return 10;
    if (__builtin_cpu_supports("xop")) return 11;
    return 0;
}
