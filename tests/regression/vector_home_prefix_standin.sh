#!/usr/bin/env bash
# Pre-fix stand-in compiler for the vector home-homing gate.
#
# `check_vector_fma_homing.sh --expect-defect` must go RED, and it must go red
# because the DEFECT is present -- not because the fixture failed to build or
# because a reference compiler rejects LCCC's headers.  The defect this wrapper
# reproduces is exactly the one `fuse_vector_loads` closes: a direct vector
# load's scratch home is read back instead of re-pointing the reader at the
# load's source, so `_mm256_loadu_ps(p)` becomes store-to-home + reload and the
# FMA consumes `40(%rsp)` inside a 72-byte frame.
#
# `CCC_DISABLE_PASSES=vecpromote` disables precisely that pass family in LCCC
# (`promote_vector_temps`, `fuse_vector_loads`, the alignment relaxation it
# guards).  It is the same A/B the gate documents, so the counterfactual is the
# real code path and not a manufacturing of the symptom: with the pass off the
# emitted bodies are the pre-fix ones byte for byte.
#
# Not a compiler in its own right -- it delegates to $REAL_CC, which must be
# an LCCC build (default `target/fastbuild/lccc`, i.e. repo-root relative, the
# way both mirrors invoke it).  It never runs as part of a normal build.
set -euo pipefail

REAL_CC="${REAL_CC:-target/fastbuild/lccc}"

# Forward the compiler mode probes (`--version`, `-dumpmachine`, ...) so a
# caller that asks the wrapper about itself gets a real answer.
if [ $# -eq 0 ]; then
    exec "$REAL_CC"
fi

exec env CCC_DISABLE_PASSES="${CCC_DISABLE_PASSES:+$CCC_DISABLE_PASSES,}vecpromote" \
    "$REAL_CC" "$@"
