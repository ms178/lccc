# Kernel if-conversion evidence — 2026-09-19

This directory records the Compiler Explorer code-generation comparison for
`branch_index_store` in `tests/regression/vectorize_isa_gate.c`, the reduced
form of the Linux `llc_populate_cpu_shard_id()` miscompile.

Command:

```sh
python3 scripts/godbolt.py audit
python3 scripts/godbolt.py compare tests/regression/vectorize_isa_gate.c \
  --local target/fastbuild/lccc --function branch_index_store \
  --flags '-O2' --local-flags '-O2' \
  --artifact-dir engineering/evidence/kernel/2026-09-19-ifconvert/oracle \
  --json engineering/evidence/kernel/2026-09-19-ifconvert/oracle.json
```

The audit resolved GCC 16.2, Clang 23.1.0, ICC 2021.10.0, and the moving ICX
latest channel. All oracle compilers use scale-4 addressing for the `int`
array. Fixed LCCC does too. LCCC currently emits 23 static instructions versus
16 for GCC/ICX, 19 for Clang, and 20 for ICC; the remaining gap is primarily
redundant zero handling around `ctz` despite the loop's nonzero-mask guard. It
is documented as an optimization opportunity, not conflated with this
correctness fix.

The generated `.s`, JSON manifest, and text summary are checked in so this
claim remains independently reviewable even if Compiler Explorer aliases move.
