/* Godbolt/oracle kernel for the baseline (x86-64-v1) popcount SWAR
 * sequence (#328). noinline so CE keeps a named function.
 *
 * Compare:
 *   scripts/codegen_oracle.py engineering/kernels/popcount_swar.c \
 *     --function popcount32 --local target/fastbuild/lccc \
 *     --flags '-O3 -march=x86-64' \
 *     --oracles gcc16.2,clang23.1,icc,icx
 */
unsigned popcount32(unsigned x) __attribute__((noinline));
unsigned long popcount64(unsigned long x) __attribute__((noinline));

unsigned popcount32(unsigned x) { return (unsigned)__builtin_popcount(x); }
unsigned long popcount64(unsigned long x) { return (unsigned long)__builtin_popcountl(x); }
