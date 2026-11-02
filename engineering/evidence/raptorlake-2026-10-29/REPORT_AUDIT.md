# Raptor Lake report audit — 2026-10-29

## Scope and evidence boundary

Source: the user-provided `report.txt` (generated 2026-10-29). It contains two
separately pinned classes on an Intel Core i7-14700KF: P-core CPU 0 and E-core
CPU 16. The top-15 list below is sorted by the report's paired-median LCCC/GCC
wall-time ratio over the 126 benchmark/class rows; a benchmark on both classes
is intentionally two results. Times include startup. For each row, the report
also provides per-task PMU cycles and retired instructions in the detailed
perf section. Those counts are medians of the available samples; they are
supporting diagnostics, not proof of whole-program speedups.

The open-PR audit against fetched `origin/main` found that the primary
per-function AVX2 vectorization change from #998 and the RISC-V volatile/jump
range work from #1010 are already present in main (the latter is recorded in
main's `8688d88` history). Their stale PR diffs were not reapplied over newer
main implementations. The remaining open PR #1015 was reapplied on the rebased
session branch; its regression fixtures and source changes are included in the
working patch and were audited/tested individually as listed below.

The uploaded report does **not** include the named evidence directory's source
revision checkout, raw benchmark binaries, assembly artifacts, or a separate
follow-up checklist. Its metadata says the measured checkout was revision
`bd3e9ab914b9abcaf22bbc48e1c026054aaff3ce` and had uncommitted changes in the
linker, workflow, and hybrid-benchmark files. The current Arena tree is not that
revision. Therefore, an assembly observation made on the current rebuilt tree
is a lead, not a byte-for-byte explanation of the uploaded run. No result below
is claimed fixed unless a corresponding implementation and target-hardware
A/B are subsequently recorded.

The report's own metadata marks its initial PMU probe unavailable for the
corpus run, while the later detailed perf sections contain per-workload
counter samples. The medians below use those explicit per-workload samples;
no derived aggregate PMU score is asserted.

## Fifteen worst measured rows

`cycles ×` and `instructions ×` are the report's per-task LCCC/GCC medians.
Numbers greater than 1 indicate more LCCC work in that counter; a ratio below
1 indicates fewer. They are not a substitute for wall-time results.

| Rank | Class | Benchmark | Wall ratio | LCCC / GCC median wall | Cycles × | Instructions × | Initial disposition |
|---:|:---:|---|---:|---:|---:|---:|---|
| 1 | P | `zlib_ng_slide_hash` | 1.905 | 2.36 / 1.23 ms | 2.507 | 2.635 | The unsigned saturating U16 subtraction has now been lowered to SSE2 `psubusw` / AVX2 `vpsubusw` under a U16/x86 feature gate, with signed/nonzero/overlap/tail/ISA negatives. Static codegen is fixed; no Raptor Lake post-change timing was collected. Parameter-specialized loops remain a separate question. |
| 2 | E | `zlib_ng_slide_hash` | 1.725 | 3.57 / 2.09 ms | 2.018 | 2.635 | Same U16 saturation codegen fix and validation; short wall region and no post-change target timing, so no runtime improvement is claimed. |
| 3 | E | `byte_load_reassembly` | 1.623 | 3.19 / 1.96 ms | 1.856 | 1.366 | Short wall region; PMU counts also differ. Need analyze safe load combining (alignment, object bounds, alias, volatile, and page-boundary semantics). |
| 4 | E | `fp64_product_map` | 1.332 | 538.68 / 404.34 ms | 1.320 | 1.077 | Long enough to prioritize. Current-tree screening shows GCC has a `constprop` clone while LCCC uses a generic call target. Investigate generic static-function specialization; no workload-name tuning. |
| 5 | E | `sqlite_vdbe_serial` | 1.313 | 16.91 / 12.93 ms | 1.331 | 1.234 | Nontrivial instruction gap; inspect switch lowering and big-endian decode under report revision. |
| 6 | E | `zlib_ng_compare256` | 1.289 | 8.13 / 6.31 ms | 1.329 | 1.109 | Short wall region; analyze unaligned compare lowering and mismatch exits. |
| 7 | E | `arith_loop` | 1.275 | 75.82 / 59.45 ms | 1.279 | 0.808 | Important counter divergence: LCCC retires fewer instructions yet takes more cycles. Inspect dependency chains, execution-port pressure, and frequency; do not use instruction count as the objective. |
| 8 | E | `linux_list_sort` | 1.246 | 394.28 / 316.20 ms | 1.313 | 1.161 | Long workload. Current-tree assembly shows an indirect comparator call in LCCC's merge loop while GCC specializes the comparator into direct field comparisons. Candidate generic IPA/devirtualization gap. |
| 9 | P | `linux_list_sort` | 1.233 | 244.14 / 198.00 ms | 1.233 | 1.161 | Same specialization lead; class-specific wall and cycle results differ, so preserve per-class validation. |
| 10 | P | `sqlite_vdbe_serial` | 1.231 | 11.17 / 9.01 ms | 1.241 | 1.234 | Same switch/decode gap. |
| 11 | P | `byte_load_reassembly` | 1.227 | 1.83 / 1.48 ms | 1.339 | 1.366 | Short region; only pursue after proving a legal, generic widening/combining rule. |
| 12 | P | `sqlite_varint` | 1.195 | 14.34 / 11.97 ms | 1.245 | 1.049 | Cycles diverge more than retired instructions; analyze serial dependency and branch behavior. |
| 13 | P | `hot_web_loop` | 1.193 | 33.99 / 28.45 ms | 1.199 | 1.048 | Cycles diverge more than instructions; inspect branches, phi copies, and register homes, then verify a whole-workload A/B. |
| 14 | E | `sha256_transform` | 1.178 | 280.25 / 237.94 ms | 1.178 | 0.941 | Fewer LCCC instructions but more cycles; prioritize scheduling/dependency investigation and retain digest correctness. |
| 15 | E | `loop_patterns` | 1.165 | 40.14 / 34.55 ms | 1.171 | 1.075 | Investigate generated loops individually; the benchmark aggregates multiple shapes and is not a single optimization target. |

## Current-tree checks performed (not Raptor Lake performance claims)

* Restored only hash-locked `harness-cache-7` toolchains (Rust 1.99.1, GCC
  16.2.0, Clang 23.1.4, binutils 2.47) and created the requested 8 GiB swap.
* `scripts/build_lccc_fast.sh`: PASS; rebuilt `lccc`, target drivers, and
  `lccc-ld` in fastbuild with two Cargo jobs. No compiler warnings were emitted.
* `cargo fmt --check`: PASS.
* `cargo clippy --all-targets --profile fastbuild --locked -j 2 -- -D warnings`:
  PASS, no warnings.
* A correctness-only (`--skip-perf --strict`) eight-kernel run against cached
  GCC 16.2.0 passed for `fp64_product_map`, `arith_loop`, `loop_patterns`,
  `byte_load_reassembly`, `sha256_transform`, `linux_list_sort`,
  `zlib_ng_slide_hash`, and `sqlite_vdbe_serial`. It is not a timing run and
  ran on the Arena VM, not the supplied Raptor Lake host.
* Before applying PR #1015's saturation change, current-tree inspection
  found a U16 saturating subtraction expanded to compare/mask/subtract while
  GCC used `psubusw`. After applying and rebuilding, the same current-tree
  workload emits `psubusw`; `check_u16_saturating_sub.sh` passes its signed,
  fallback, overlap, scalar-tail, SSE2, AVX2, and ISA-negative controls. This
  establishes the codegen pattern and semantic controls, not a Raptor Lake
  runtime win; the uploaded report's revision predates the change.
* A later strict correctness-only screen (fastbuild LCCC vs cached GCC 16.2)
  found the list-sort comparator lead was stale or build-dependent: current
  LCCC `merge_and_count` compares node keys directly in the hot merge loop,
  rather than making an indirect callback. The report's list-sort runtime gap
  remains unexplained; investigate pointer-chasing and branch behavior using
  the exact report revision before proposing a transform. No timing was taken.
* The same later screen confirms a concrete `fp64_product_map` specialization
  lead in the current build: LCCC emits a generic `n` loop, while GCC emits a
  `constprop.0` clone for the constant 512-element call. This is a candidate
  for general internal-function constant specialization, not a benchmark-name
  special case, and needs correctness safeguards, code-size analysis, and
  target-hardware measurement. The SQLite decoder uses a jump table in the
  screen; that is not itself evidence that a different lowering is faster.
* A workload-by-workload follow-up hypothesis table and acceptance plan is in
  `OPTIMIZATION_BACKLOG.md`. The later screen passed correctness for four
  report workloads only; it collected no timing and does not change any
  performance claim.
* After rebasing the fixed session branch onto current `origin/main` at
  `2d95c4e` (PR #1017), the lane-plan self-test/audit, shard-partition
  self-test/audit, and workflow-shell audit all pass with PyYAML 6.0.3 in
  `/tmp/arena-python-deps`. The current main plan has no duplicate lane steps.
* Rebase integration exposed a stale `Class::BenignNoop` use in `cli.rs`
  against the GP-22 ledger's newer `Class::Inert` enum. Updated the single
  classification check to `Inert` (the table's accepted-but-no-effect class);
  fastbuild then compiled, and `driver_option_contract_table` passed. This
  retains the strict refusal path for all other table rows.
* After the rebase and PR #1015 integration, the unsigned-reduction regression,
  macro-parity, UCN-identifier-table, per-function target-attribute ISA,
  loop-carried SLP, two-block-unroll red-team, in-loop register-home,
  include-next provenance and i686 debug-GOTOFF regressions pass. The new
  saturating-subtract unit test, four narrow-comparison-width tests and three
  SLP emitted-order tests pass. The eight selected benchmark executables pass
  correctness-only comparison against GCC 16.2.0; no timing was collected.
  The 45-test corpus-infrastructure group passes with local PyYAML available.
  Fastbuild rebuild, rustfmt, clippy with `-D warnings`, audit census and
  `git diff --check` pass on the rebased tree.

No local CI matrix was run, per request. Hosted GitHub Actions on PR #1019
failed at `062a96b` because the unsigned-reduction regression was orphaned
from both gate lists. That gate is now wired into the workflow, local gate
manifest, and hosted lane plan in `971fd40`. The full rerun on `971fd40` has
completed successfully, including CI, Debian exact-version oracle,
relocation/PGO red-team, IPA-PTA/live-range, and targeted compiler research
workflows. This green status applies to `971fd40`, not any later commit. The
upload does not provide the separate follow-up list mentioned in the request.
Those missing inputs, the revision mismatch, and lack of access to the user's
physical Raptor Lake machine prevent honest claims that all follow-ups or all
15 regressions are fixed. See `OPTIMIZATION_BACKLOG.md` for a prioritized,
explicitly unvalidated optimization backlog, and the attached report for the
full sample and per-task evidence.
