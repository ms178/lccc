/* AUD-CPU-1 differential: __builtin_cpu_supports answers the question "does
 * the CPU executing this program have the feature?", for every modelled name,
 * under the default target — GCC, Clang and ICX all lower it to a runtime
 * CPU-model lookup and never fold it against -march (measured with
 * scripts/godbolt.py; see tests/data/cpu_supports_name_census.json for the
 * name-space oracle).
 *
 * Every name is written as a literal at the call site, because a loop over a
 * name array is not accepted by any of the four compilers ("parameter to
 * builtin must be a string constant or literal" / "parameter to builtin not
 * valid"), and `tests/workloads/dispatch/README.md` records the same rule for
 * the measurement harnesses.
 *
 * run_regression.py's GCC differential compares two *runtime* probes of the
 * same host: any answer that depends on the compile flags (the AUD-CPU-1
 * defect — `-march=x86-64` answered 0 for avx2/bmi2/sha on a CPU that has
 * them, the default target answered 1 on machines that lack them) shows up as
 * a diff.
 *
 * Host-independent invariants: SSE2 exists on every CPU this compiler can run
 * on (x86-64 baseline / i686 with SSE2), and compiler-specific alias spellings are covered separately.
 */
#include <stdio.h>

#define PROBE(name) printf(name "=%d\n", !!__builtin_cpu_supports(name))

int main(void) {
    PROBE("sse");
    PROBE("sse2");
    PROBE("sse3");
    PROBE("ssse3");
    PROBE("sse4.1");
    PROBE("sse4.2");
    PROBE("sse4a");
    PROBE("popcnt");
    PROBE("cmov");
    PROBE("mmx");
    PROBE("aes");
    PROBE("pclmul");
    PROBE("sha");
    PROBE("avx");
    PROBE("avx2");
    PROBE("fma");
    PROBE("f16c");
    PROBE("bmi");
    PROBE("bmi2");
    PROBE("lzcnt");
    PROBE("movbe");
    PROBE("adx");
    PROBE("rdseed");
    PROBE("fsgsbase");
    PROBE("avx512f");
    PROBE("avx512bw");
    PROBE("avx512vl");
    PROBE("gfni");
    PROBE("vaes");
    PROBE("vpclmulqdq");
    PROBE("amx-tile");
    PROBE("cmpxchg16b");
    PROBE("xsave");

    /* A compile-time answer would also fail these two. */
    if (!__builtin_cpu_supports("sse2")) return 1;
    /* Non-GCC alias spellings are checked in builtin_cpu_supports_aliases.c. */
    return 0;
}
