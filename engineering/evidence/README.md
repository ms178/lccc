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
| `ms09-peephole-utf8-audit-2026-09-06.md` | bounded historic/publication audit, live UTF-8 repair, and validation record |
| `godbolt/ms09-inline-asm-utf8.c` | noinline inline-asm companion used for the successful four-oracle MS-09 screen |
| `pmu/` | (future) hardware PMU snapshots from the 14700KF metal runner (MS-14) |
