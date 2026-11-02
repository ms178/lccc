# Session 58 AArch64 conditional-increment oracle

Reproduce from the repository root:

```bash
python3 scripts/codegen_oracle.py tests/regression/arm_csinc_select.c \
  --function inc_if_sge_i32 --arch aarch64 \
  --local target/fastbuild/lccc-arm --local-flags=-O2 \
  --flags=-O2 --oracles carm64g1610
```

The local compiler was built only with `scripts/build_lccc_fast.sh`. Both
compilers select a direct compare plus conditional increment. GCC's complete
leaf is much smaller because LCCC still assigns the parameters/results to
callee-saved homes; `report.md` records that gap rather than hiding it.
Static counts are screening evidence, not hardware performance measurements.


---

<!-- folded from `godbolt/session58-arm-csinc/report.md` (consolidation 2026-09-16) -->

# Codegen oracle report

Static code-size/structure statistics from local LCCC and Compiler Explorer.
These are screening metrics, not PMU evidence; verify wins with controlled
runtime and hardware counters on the intended target before making claims.

| Source | Function | LCCC | Best | Best compiler | LCCC/best | Loads | Stores | Spills | Branches |
|---|---:|---:|---:|---|---:|---:|---:|---:|---:|
| `tests/regression/arm_csinc_select.c` | `inc_if_sge_i32` | 16 | 3 | carm64g1610 | 5.33x | 4 | 4 | 8 | 1 |
