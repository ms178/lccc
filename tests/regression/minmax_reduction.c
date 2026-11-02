/* MINMAX-1 corpus entry: the randomized min/max/sum differential harness,
 * built as ONE translation unit.
 *
 * `run_regression.py` compiles every `tests/regression/*.c` standalone with a
 * single compiler invocation, so a root-level `.c` must be a complete TU.  The
 * harness itself deliberately declares its kernels `extern` — the
 * `check_minmax_reduction.sh` gate links them as a SEPARATE object so a
 * miscompile can be localised to the vectorized side — which makes the harness
 * file itself unlinkable on its own.  It therefore lives in `minmax_shapes/`,
 * out of the corpus glob, and this wrapper pulls it in together with the
 * kernel definitions.
 *
 * What the corpus adds on top of the gate: the gate compares lccc against the
 * GCC oracle over two link shapes; this entry additionally runs the kernels
 * against the harness's own independent scalar references inside one lccc
 * compilation at the corpus's default -O2, on every corpus run, with no oracle
 * and no extra script.  The program is deterministic (fixed xorshift seed) and
 * exits non-zero on the first disagreement, so it needs no `.flags` and no
 * `.env`.
 */
#define LCCC_MINMAX_KERNELS_INLINE 1
#include "minmax_shapes/minmax_harness.c"
