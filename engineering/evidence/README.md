# Evidence

Frozen, data-bearing artifacts. **Retention rule:** everything here is
either (a) an oracle baseline (the `godbolt/corpus/` assembly dumps the
scoreboard compares against), (b) a pinned measurement record (workload
kernels, A/B numbers, session-numbered screening dirs), or (c) a
methodology document. Evidence is point-in-time by definition — treat
numbers as screening records, not current claims; the live state is
[`../STATE.md`](../STATE.md) and the root `README.md` performance table.

| Path | What |
|------|------|
| `godbolt/` | Compiler Explorer oracle corpus (gcc16.2 / clang 22.1 / icx), scoreboard, methodology |
| `workloads/` | gzip / zlib-ng / Expat LCCC vs GCC measurements |
| `benchmarks/2026-08-28-1b3994e7/` | canonical 33-kernel screening run (raw JSON + verified merge) behind the root README table |
| `aarch64/session59..61/` | AArch64 torture + VM screening records (frozen) |
| `simd-fp-oracle.md` | SIMD/FP oracle audit methodology + 2026-08-18 record |
| `phi-acyclic-copy-order-2026-09-11/` | paired A/B behind landing `CCC_PHI_ACYCLIC_ORDER` opt-in (sha256 5.08% slower, p=0.0003) **and** the noise-floor record showing byte-identical arms can reach p=0.0164. Superseded on disposition by the row below; kept as the frozen record of the pre-rebase base `4f527199` |
| `ra-web-inloop-use-2026-09-11/` | amplified paired factorial behind landing the web-wide in-loop-use supply (`CCC_NO_WEB_INLOOP_USE`): sha256 **+3.63% / +4.33%**, p=0.0000, median and min agreeing in both replicates — **and** withdrawing the phi-resolver default, which on the rebased base `25ed36de` costs −1.99% / −0.71% on top of it. Carries the `rot()` isolation matrix for upstream's `evict_short_k` escape, the corrected gcc oracle counts (142 insns / 8 stkref, not the bogus 240/31 a silent whole-file fallback produced), and the withdrawal of an un-amplified +8.21% median. **Session 17 addendum:** the same directory now carries the audit of that fix — an 807-TU byte differential (`corpus-differential.txt`, via the new `scripts/differential_corpus.sh`) that found a **40.53% `lz4_compress` regression** at identical instruction count, the trace-level mechanism (admission-cap veto on v212 remcost 100 handing the pressure to a cost-blind valve that spilled v133 remcost 1110), and the decoupled design that ships instead: 13 → 10 TUs touched, `lz4_compress` byte-identical to base, `sha256_transform` and `linux_rbtree` byte-identical to the coupled build so every number above still holds |
| `ms09-peephole-utf8-audit-2026-09-06.md` | bounded historic/publication audit, live UTF-8 repair, and validation record |
| `godbolt/ms09-inline-asm-utf8.c` | noinline inline-asm companion used for the successful four-oracle MS-09 screen |
| `pmu/` | (future) hardware PMU snapshots from the 14700KF metal runner (MS-14) |
