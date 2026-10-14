/* AUD-SILENT-ZERO-1 corpus entry.
 *
 * The fixture the trap gate compiles lives in `cross_target_intrinsics/`, out
 * of the corpus glob: `run_regression.py` compiles every `tests/regression/*.c`
 * as a complete stand-alone TU, and the gate needs the SAME source compiled
 * once per target (and, on x86, with `-maes` so AUD-CPU-3's feature gate
 * permits the intrinsic).  This wrapper pulls the fixture in so the corpus
 * still compiles it on every run — with `-maes` from the `.flags` file — which
 * is what keeps the x86 half of the contract (the intrinsic still lowers to
 * `aesenc` where the target has it) exercised by the ordinary corpus.
 */
#include "cross_target_intrinsics/x86_only.c"

int main(void) { return 0; }
