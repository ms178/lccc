# S05 follow-up — rebase onto #846, backlog rewrite, compile-time profile, -w

Date: 2026-10-17. Branch `arena/d3169e6d-lccc` (PR #844), rebased onto
`main` = `c39b6e8c` (merge of PR #846).

## 1. Rebase and semantic audit of PR #846

Mechanics: 24 commits rebased, one textual conflict (`scripts/ci_local.sh`,
union of the upstream `generator-map` gate and this branch's
`reg-off-sym-fold` gate — both kept). One semantic conflict the textual
merge missed: both sides moved `knob_literals` from 500 (+1 upstream
kill-switch, +3 this branch); the merged baseline kept 503 while the tree's
union is 504. Fixed in its own commit with the arithmetic spelled out.
The originally-empty CI-retrigger commit was dropped.

Audit verdict on #846: **agree with the merge.** The PR's own review
(docs/reviews/pr-846-audit.md) had flagged two P1 correctness blockers
(F1 unmatched stores silently ignored, F2 segment stores losing their
address space) plus a fail-open trace gate (F3). Verified on the merged
tree that the follow-up commits really closed them:

- the generator matcher now records the single nonvolatile
  default-address-space byte store, rejects any second `Store`/`GEP` via an
  explicit opcode-category arm, and the second body walk ends in
  `_ => return None` — every instruction must be accounted for (F1/F2);
- `check_generator_map.sh` no longer contains `|| true`; the Python helper
  raises on a nonzero producer exit before counting traces (F3);
- 21/21 generator-map Rust tests and 71/71 iv_widen tests pass locally;
  the compiled gates (`generator-map`, `ivw-paired-counter`) pass;
- hosted CI on the merged head: 12/12 success.

Where I remain critical (consistent with their own F7): the generator
vectorization is a legality transform with no profitability model, and the
PR's own benchmark still measures LCCC behind GCC on the generator kernel
(0.0427 s vs 0.0306 s). The structure (read-only match → staged lowering →
commit) is the right architecture.

## 2. Compile-time profile — the measured MS-20 record

Callgrind (Ir is deterministic under VM load; wall time is not), compiling
`tests/benchmark/programs/expat_siphash24.c`, fastbuild `lccc`:

| Level | Total Ir | peephole (inclusive) | peephole_for_metric | memcmp+starts_with |
|-------|----------|----------------------|---------------------|--------------------|
| -O0   | 2.68 G   | 89.0 %               | 21.3 %              | —                  |
| -O1   | 6.10 G   | 90.9 %               | 44.8 %              | 24.7 %             |
| -O2   | 1.64 G   | 65.2 %               | (below top-5)       | 20.3 %             |

Findings, in attack order (now BACKLOG MS-20):

1. `peephole_for_metric`: the dual-layout explorer in
   `backend/generation.rs` emits every chain-fired function twice and runs
   the FULL text peephole on both arms just to count instruction lines,
   then emits the winner a third time. At -O1 that is 44.8 % of the whole
   compile. The A/B design intent is documented and measured (zstd_count),
   so the fix is memoization/cheapening, not removal.
2. Text-based opcode matching: `memcmp` + `<[u8]>::starts_with` are ~25 %
   of -O1. The peephole re-scans instruction text per pass; a one-time
   parse into opcode id + operand slices would cut this class.
3. `-O1` costing 3.7× `-O2` falsifies the imported audit's "-O0 slower
   than -O1" claim; the outlier is -O1 (more emitted text + more chain
   candidates reach the metric loop than at -O2, which shrinks IR first).

## 3. Backlog rewrite (user-ordered)

`engineering/agent/BACKLOG.md` rewritten from the ground up; the previous
catalog is archived verbatim at `engineering/archive/BACKLOG-2026-10-16.md`.
Every claim of the external "MB" audit was re-probed before admission;
the new §3 of the backlog records rejected/stale claims so they are not
re-imported. Highlights:

- **admitted with fresh probes:** MS-20 (above); DW-01 (`-g` emits zero
  `.debug_*` sections — verified); FE-26 (C23: `nullptr`, `constexpr`,
  `bool` keyword all rejected under `-std=c23` — verified); FE-27 (warning
  parity: zero warnings where GCC 16.2 emits three — verified); LK-31
  (`-z pack-relative-relocs` accepted, no DT_RELR emitted — verified).
- **rejected:** MB-03 — the tree answers `__builtin_cpu_supports` with a
  runtime CPUID probe exactly as GCC 16.2/Clang 23.1/ICX do even under
  `-march` guarantees (AUD-CPU-1); folding to the target feature set would
  diverge from every reference compiler.
- **already fixed:** the "silent no-op options" core (loud warnings +
  `LCCC_STRICT_OPTIONS=1` landed with the #753 port in this PR).

## 4. Landed this session

- Census baseline 503 → 504 (rebase-union fixup, no new knob).
- **`-w` implemented** (FE-27 first slice): measured against GCC 16.2 —
  dominant in both orders, silences `#warning` and kind-less warnings;
  sticky latch in `WarningConfig`; unit tests for both orders plus a
  `-Wall` control; end-to-end output matches gcc-16 byte-for-byte (empty).
  Until now every compile that passed `-w` (benchmark corpus, size gates)
  produced one tolerated-unknown warning line of log noise.

## 5. Open after S05

- MS-20 implementation (the 90 % peephole share) — needs its own session
  with byte-identical-output gating on the whole corpus.
- Fresh post-rebase benchmark scoreboard (PERF-42) — the S04 numbers
  predate #846's generator/IV changes.
- COR-01 corpus triage (128 FAILs) unchanged.
- Hosted CI on the final head of this branch must be green before merge.
