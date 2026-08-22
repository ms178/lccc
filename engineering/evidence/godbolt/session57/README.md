# Session 57 AArch64 sieve code-generation oracle

Source: `tests/benchmark/programs/sieve.c`, function `count_primes`.

* `sieve-gcc16.1-aarch64.s`: Compiler Explorer `carm64g1610`, `-O2`,
  captured with `scripts/godbolt.py` on 2026-08-22.
* `sieve-lccc-aarch64.s`: local `lccc-arm -O2` from the Session 58 SSA-combine
  revision, built with the `fastbuild` O1/J2 compiler preset.

GCC writes its conditional count update as the `cinc` alias. LCCC writes its
canonical `csinc` form through the Session 58 SSA machine combine. The dumps
establish instruction-selection parity for this idiom, not whole-function
parity and not runtime performance.
