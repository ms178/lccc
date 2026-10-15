# VEX flag-input architectural witness

These paired assembly functions differ only by removal of the candidate
`vmovapd %xmm6, %xmm7`. For each of eleven flag/string tests, the real ISA
observably distinguishes the original and incorrectly deleted sequences.
They are not generated C code or a claim of a production C miscompile.

On an AVX2/SSE4.2-capable x86-64 Linux host:

```sh
gcc -O2 -mavx2 tests/regression/fixtures/peephole-vex-flags/{driver.c,witness.s} -o /tmp/vex-flags
/tmp/vex-flags
```

All eleven rows must report `original=1 removed=0`; exit status is zero only
when all assertions hold. Do not execute on a CPU lacking AVX2 or SSE4.2.

The hermetic Rust regression
`dead_code::tests::vex_flag_only_tests_read_both_vector_operands` checks the
optimizer itself without requiring AVX2 execution. It failed on the uncorrected
classifier at `vucomisd`, then passed with flag-only VEX operands modeled as
reads. This native fixture independently checks the architectural dependency.

The four VPCMP{E,I}STR{I,M} forms read both vector operands and produce
implicit index/mask outputs. The prior seven-case repair still failed the
extended optimizer regression at `vpcmpistri`; positive full-definition
admission fixes that separate defect without relying on a `v` prefix.
