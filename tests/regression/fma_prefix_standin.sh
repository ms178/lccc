#!/usr/bin/env bash
# Pre-fix stand-in compiler for the FMA digit-permutation gate.
#
# `check_fma_digit_permutation.sh --expect-defect` must go RED, and it must go
# red because the DEFECT is present -- not because the fixture failed to build.
# Earlier revisions of this wrapper only `-D`-renamed the 213/231 prototypes
# onto 132, which a reference compiler cannot resolve (the probe calls LCCC's
# raw `__lccc_simd*` builtins, which no other compiler declares), so the gate
# was red for a build error and proved nothing about the placement.
#
# This wrapper instead `-include`s `fma_prefix_standin.h`, which DEFINES those
# builtins with the pre-fix semantics: one shared placement (`rm = b, vvvv = c,
# dst = a`) applied to all three digit spellings. Under Intel SDM pseudocode
# that assignment is right for 132 and wrong for 213 and 231, so the `_213` and
# `_231` rows disagree with the fixture's own scalar `ref_*` rows and the gate
# reports exactly those rows red. The `_132` rows stay green, which is the
# point: the defect never touched 132.
#
# Not a compiler in its own right -- it delegates to $REAL_CC (default `cc`).
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
REAL_CC="${REAL_CC:-cc}"

if [ $# -eq 0 ]; then
    exec "$REAL_CC"
fi

# No `-I <repo>/include`: the emulation header is self-contained (vector types
# from `<immintrin.h>`), because pointing a reference compiler at lccc's headers
# makes it reject lccc-specific builtins such as `__builtin_ia32_pmulld128`.
exec "$REAL_CC" "$@" -include "$HERE/fma_prefix_standin.h"
