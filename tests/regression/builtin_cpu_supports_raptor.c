/* AUD-CPU-1 differential under -march=raptorlake (see the .flags sidecar).
 *
 * The target guarantees AVX-VNNI/GFNI/VAES/VPCLMULQDQ, and the pre-fix
 * compiler folded those to 1 on *any* host while folding avx512f/amx-tile to
 * 0; GCC, Clang and ICX keep probing at run time, so the answers here must
 * match GCC's on the same host (run_regression.py differential) and must not
 * depend on -march at all — the point of the test is that this file's answers
 * are the *same kind of answer* as the default-target file's.
 *
 * Literal names only: a loop over a name array is rejected by every compiler
 * (see the sibling default-target test).
 */
#include <stdio.h>

#define PROBE(name) printf(name "=%d\n", !!__builtin_cpu_supports(name))

int main(void) {
    PROBE("sse2");
    PROBE("avx2");
    PROBE("avxvnni");
    PROBE("gfni");
    PROBE("vaes");
    PROBE("vpclmulqdq");
    PROBE("aes");
    PROBE("sha");
    PROBE("bmi2");
    PROBE("fma");
    PROBE("avx512f");
    PROBE("avx512bw");
    PROBE("avxvnniint8");
    PROBE("amx-tile");
    PROBE("avx10.1");
    PROBE("movrs");
    PROBE("clzero");
    PROBE("wbnoinvd");
    PROBE("ptwrite");

    if (!__builtin_cpu_supports("sse2")) return 1;
    return 0;
}
