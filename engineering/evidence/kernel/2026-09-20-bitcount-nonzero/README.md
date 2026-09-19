# Nonzero bitcount lowering — 2026-09-20

This evidence re-runs the fixed Linux workqueue reduction against every pinned
Compiler Explorer oracle after introducing explicit nonzero-precondition CLZ
and CTZ IR operations.

Commands:

```sh
python3 scripts/godbolt.py audit
python3 scripts/godbolt.py compare tests/regression/vectorize_isa_gate.c \
  --local target/fastbuild/lccc --function branch_index_store \
  --flags=-O2 --local-flags=-O2 \
  --artifact-dir engineering/evidence/kernel/2026-09-20-bitcount-nonzero/baseline \
  --json engineering/evidence/kernel/2026-09-20-bitcount-nonzero/baseline.json

python3 scripts/godbolt.py compare tests/regression/vectorize_isa_gate.c \
  --local target/fastbuild/lccc --function branch_index_store \
  --flags='-O2 -march=x86-64-v3' --local-flags='-O2 -march=x86-64-v3' \
  --artifact-dir engineering/evidence/kernel/2026-09-20-bitcount-nonzero/x86-64-v3 \
  --json engineering/evidence/kernel/2026-09-20-bitcount-nonzero/x86-64-v3.json
```

## Results

### Baseline x86-64, `-O2`

| compiler | instructions |
|---|---:|
| GCC 16.2 | 16 |
| ICX latest | 16 |
| **LCCC** | **17** |
| Clang 23.1 | 19 |
| ICC 2021.10 | 20 |

LCCC improved from 23 to 17 instructions: it now beats current Clang by two
and ICC by three, while remaining one instruction behind GCC/ICX. The hot loop
uses one direct `bsfq` and no zero test, fallback move, fallback branch, or dead
source-to-accumulator preload.

### x86-64-v3, `-O2 -march=x86-64-v3`

| compiler | instructions |
|---|---:|
| GCC 16.2 | 14 |
| ICX latest | 16 |
| **LCCC** | **16** |
| Clang 23.1 | 19 |
| ICC 2021.10 | 20 |

The remaining gap is no longer bitcount lowering or phi-diamond layout. LCCC
still emits a per-iteration sign extension of the signed `int` destination index where
GCC/ICX widen the induction variable. This is a separate IV improvement;
they must not be hidden by weakening the source or counting only a selected
subset of instructions.

All assembly and machine-readable manifests are retained alongside this file.
