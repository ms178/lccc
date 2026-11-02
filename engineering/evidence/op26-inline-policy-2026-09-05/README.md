# OP-26 / inline-policy raw evidence — 2026-09-05

All timings use a single `taskset -c 0` CPU and alternate arm order each
round.  They are shared-VM wall-clock screens, not PMU measurements.  Each
binary's stdout and exit status were compared before timing.

* `pre-clone-cap-old-vs-policy.{json,txt}` compares a compiler built from
  `59edc453` (before the normal-`-O2` wrapper/nested-loop policy) against the
  then-current policy compiler.  The wrapper/nested policy is byte-identical
  to the final compiler for `spectral_norm` and `hash_table`; glibc's final
  clone-cap result is deliberately recorded separately below.
* `glibc-clone-cap-amplified.txt` compares the same old compiler with final
  clone-cap code after mechanically changing only the benchmark's
  `#define PASSES 4096U` to `65536U`.  It captures the five-way
  `glibc_memcmp_bytes` clone decision above the process-start noise floor.
* `tls-globaladdr-cse-ab.txt` compares two otherwise identical current
  compilers, immediately before and after the TLS use-class CSE change.

Compiler build profile is the repository `fastbuild` configuration (`-O1`,
`-j2`); generated benchmark code is `-O2`.  Local GCC was Debian 14.2.0.
The cross-compiler structural oracle result is under
`engineering/evidence/godbolt/op26-spectral-main/` and records its own exact
remote compiler identities and flags.
