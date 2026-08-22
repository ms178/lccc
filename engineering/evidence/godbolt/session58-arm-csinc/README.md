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
