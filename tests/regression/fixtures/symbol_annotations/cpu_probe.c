/* `__builtin_cpu_supports` makes the driver append the CPUID probe helper to
 * the unit: its cached words, the `ready` flag, the initialiser, and under
 * i686 PIC a PC thunk. check_symbol_annotations.py reads their symbols. */
int has_avx2(void) { return __builtin_cpu_supports("avx2"); }
int has_popcnt(void) { return __builtin_cpu_supports("popcnt") && __builtin_cpu_supports("sse4.2"); }
int main(void) { return has_avx2() + has_popcnt() > 2; }
