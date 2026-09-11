# Follow-up: rebase to latest main, S05 inline fix, S06 cmp-fusion hardening, S07 ifcombine guard, worst-10 triage

Date: 2026-09-11 · Base: `ms178/lccc` main `@ 417951a4` (PR #490, verified
fresh at snapshot time) · Deliverable: `/home/user/ms178-1.patch` (ledger
`S04`–`S07` vs `417951a4`, cumulative 57,656 bytes, APPLIES-CLEAN, independently
verified) · Series: `/home/user/artifacts/series/0001..0004` · Bundle/tarball/
ledger refreshed and verified (bundle test-cloned).

Numbering note: this session's working shorthand (`S04-inline`, `S05-cmpfusion`,
`S06-ifcombine`) predates the rebase. The ledger already held `S01`–`S03`, so
the landed numbers are **ledger-S05 = inline**, **ledger-S06 = cmp-fusion**,
**ledger-S07 = ifcombine** (ledger-S04 = oracle-label-fix re-based). The text
below uses ledger numbers.

## 1. Verdicts up front

| Item | Verdict |
|---|---|
| Rebase onto `417951a4` + environment recovery (`.git` lost, bundle corrupt, toolchain wiped, exec bits stripped) | **Complete, fully accounted.** 25-tree-file diff vs pre-rebase backup = 21 pure-upstream + 4 ours; zero local-only files lost. Toolchain reinstalled (rustc 1.98.1 stable), full rebuild green. |
| S05 inline-single-site-static (`src/passes/inline.rs` + check script + baseline) | **Validated correctness fix, perf-neutral.** Full-corpus A/B geomean 0.7777 → 0.7857 (+1.0%, within run-to-run noise; paired runner A/B +0.6%, CV 8%). zstd_count insns 131 → 135 (more inlining) with runtime **improved** 1.1597 → 1.1371. Codegen gate re-baselined surgically (12 values, 5 workloads). |
| S06 cmp-branch-fusion-generalize (`compare_branch.rs`, 16 unit tests) | **Soundness hardening + generalization, zero corpus fire (proven).** Closes 4 REAL latent-miscompile classes in the old scan (unchecked relay gate, foreign-load deletion, foreign-movslq deletion, slot-observer deletion), each with a regression test. 16/16 tests green. All 39 corpus files byte-identical; all 16 true setCC sites audited by hand and proven correctly-unfused (13 by shape, nbody P2 by sound-conservative liveness, 1 FP-anchor, 1 cross-block). |
| S07 ifcombine-profitability-guard (`if_convert.rs`, 46 lines) | **Real measured win: lz4 +4.7%, surgical.** Implements the profitability guard the module docs promised but never implemented (doc-code mismatch — simulation/rollback described, only 3 pre-filters present). Only 1/39 corpus files changes; classifier loops still combine and vectorize (k_alpha: 43 SIMD insns). |
| Worst-10 triage (lz4, mandelbrot, find_bit) | **lz4's true gap is ~10x, not 1.94x** (A/B startup-overhead artifact — see §6). Root cause: byte-at-a-time match-extend (9 insns/byte) and literal-copy (7 insns/byte) loops; GCC uses word-compare + `memcpy`. Fix = loop-idiom recognition (big project, spec'd in backlog). mandelbrot = vectorizer gap (GCC 9 packed-double vs our 1). find_bit needs deeper analysis (our `bsfq` present, still 1.40x). |
| `ci_local.sh --fast` + clippy | **ALL GREEN on rebased tree** (17 passed, 0 failed, 3 skipped-slow; clippy `-D warnings` clean). |

## 2. Rebase and environment recovery (do not skip — durability lessons)

Mid-session the workspace lost `/home/user/lccc/.git` entirely, the Rust
toolchain (`.rustup`), the `target/` tree, and all exec bits; the archived
`lccc.bundle` proved corrupt (`Failed to traverse parents`). Network in bash
worked. Recovery, in order:

1. All source work verified intact in the working tree; 4 changed files backed
   up to `/home/user/recovery/`.
2. `git ls-remote` → latest main `417951a4` (PR #490). Fresh clone to
   `lccc-new`.
3. Tarball `lccc-src.tar.gz` (base-era, no `.git`) served as base content for
   our files: `compare_branch.rs` tarball == base commit (md5 `2e5d8e50`);
   `inline.rs` tarball == base + 4 `regparm: None` lines == **current main
   byte-for-byte** (md5 `f0213373`), so both files applied directly. Our S04
   inline hunks confirmed pure (46 diff lines, exactly the fix).
4. Upstream changes to our files since base: `inline.rs` +4 regparm lines only
   (already in our parent), `ci-codegen-baseline.json` key reorder (values
   identical), `compare_branch.rs` untouched, `codegen_oracle.py` untouched
   (S03 still applies).
5. Safe swap: full-tree backup (40 MB, minus `target/`) → moved fresh `.git`
   in → `checkout -f main` → restored our files → `diff -rq` accounting: 25
   differs = 21 pure-upstream + 4 ours; zero `Only in backup` (no local files
   lost); untracked check script survived.
6. Toolchain: `rustup` binary survived without +x (`chmod +x`), `rustup
   toolchain install stable -c rustfmt -c clippy` → rustc 1.98.1. Full
   `build_lccc_fast.sh` rebuild green (2m27s). `lccc-s04-inline` "before"
   binary survived (re-`chmod`ed, verified working).
7. Baseline re-measured on rebased main: identical 12-value S05 footprint →
   surgical update (format-preserving), gate green.

Durability lessons (applied): the snapshot bundle/tarball/ledger were
regenerated (S04–S07) and the new bundle **test-cloned**; exec-bit stripping
and `.git` loss must be assumed after every wipe — verify `git status` +
`cargo --version` + key binaries' +x at session start. The snapshot script's
structural guards (no deletions/mode changes) and merge-base guard all passed.

One process failure, owned: the first S04 snapshot run committed a dirty tree
containing all four fixes under the oracle tag (57 KB mislabeled). Fixed by
ledger surgery within the session (removed the botched entry, reset `.seq`
to 3) and redone as 4 clean per-fix cycles via stash + per-path checkout. The
final deliverable sha (`a86bae…`) matches the botched run's — same base, same
content — confirming the redo changed nothing semantically.

## 3. S05: inline single-site statics (correctness fix, neutral perf)

`!func.is_inline` exclusion removed; `is_single_call_site_static` exempted
from size caps (GCC `-finline-functions-called-once` parity). Plus regression
check script and surgical baseline refresh.

- Full-corpus A/B (39/39 correct, 9 reps): geomean 0.7777 → 0.7857. Neutral
  within noise (paired runner A/B +0.6%, runner CV 8%).
- zstd_count: 131 → 135 insns (+4 from inlining the 10-block helper), runtime
  1.1597 → 1.1371 (call elimination beats spill cost). Baseline update is
  unambiguously correct (faster + correct output).
- Incidental improvements from more inlining: gzip 77 → 75, zlib-ng 233 →
  230, expat 203 → 198, hash 145 → 147… (hash 147 → 145) insns.

## 4. S06: compare/branch fusion — hardening + generalization, zero fire

Rewrote the peephole scan: all 16 GP families (not just `%rax`), XMM-hoist
tolerance (nbody P2 shape), real `FileLiveness` gate for every NOPed family,
exact-spelling matching (`%ah` can never match), carrier-clobber refusal,
foreign-def refusal, slot-touch refusal, transparent-line preservation (old
code NOPed unrelated reloads).

- 16/16 new unit tests green (`compare_branch_fusion_tests`), full lib suite
  2361 green, no existing test changed behavior (zlib-ng pin intact).
- 4 negatives close REAL old-code holes (verified against `HEAD` version:
  old code NOPed everything `1..=test_scan` including skipped foreign loads/
  movslq, and the relay gate mapped `%-less` text to `REG_NONE` and never
  fired).
- nbody P2 root-caused: gate refuses with `Some(true)` — `%r9` genuinely
  live via exit-path `call printf@PLT` (`CALL_READS` ∋ `%r9`, the 6th arg
  register). Sound-conservative; arity-sharpening from asm text is unsound
  in general (documented, not attempted).
- Full-corpus audit: 39/39 files byte-identical; the 15 (then 16 with nbody)
  true setCC sites hand-audited — returned/stored/ANDed/arithmetized booleans,
  cmov consumers with dead flags (`subl`/`xorq` between set and test), one FP
  anchor (`ucomisd`, correctly excluded), one cross-block spill. cmov-fusion
  investigated and correctly NOT built (flags genuinely dead at every
  candidate site).

## 5. S07: ifcombine profitability guard — lz4 +4.7%

LCCC's `while (ip < mflimit)` skip loop materialized
`ref < ip && ref >= src` as `setb/setae/andl/je` (6 insns) where GCC emits
two short-circuit branches. `CCC_NO_IFCOMBINE=1` restored short-circuit and
gained 1.023x at scale. Root cause: `try_if_combine_loop`'s docs promise
"simulation + rollback" (combine iff the body becomes branch-free), but the
code contains only 3 cheap pre-filters — the simulation was never implemented.
lz4's main loop (containing nested loops + a giant match/skip diamond) passed
all 3 and combined at a pure loss.

Fix (46 lines, fail-closed): pre-filter 4 rejects the loop unless every
non-exiting conditional branch either belongs to this round's chain (folds
now) or is taken by the existing diamond/triangle detectors (same functions,
same CPU-model budgets — no heuristic drift). Rejection keeps the short
circuit, which is always safe (the pipeline's common case).

Validation:
- lz4 main loop: `reject (branch in block 23 survives)`; `fill_source`'s
  classifier loop still `accept`s (selectivity proven — better than blanket
  disable: 1.047x vs 1.023x).
- Only 1/39 corpus files changes (`lz4_compress.s`); `ascii_case_fold` still
  accepts on first visit; `k_alpha` still vectorizes (43 SIMD insns);
  `vec_class_union_ifcombine` regression green; expat already rejected via
  pre-filters 1–2 (unchanged).
- Fixpoint interaction: `k_alpha` rejects on iteration 1 (outer `||` branch
  not yet convertible) and accepts on iteration 2 — end state verified firing.
  Deadlock analysis: worst case is a missed optimization (short-circuit kept),
  never a miscompile or a regression-vs-`CCC_NO_IFCOMBINE`.

## 6. Worst-10 findings (the honest perf picture)

Measured worst-12 (lccc/gcc median, A/B config): lz4 1.941, mandelbrot 1.673,
find_bit 1.404, expat 1.275, sqlite_varint 1.244, sha256 1.210, spectral_norm
1.193, zstd 1.137, sieve 1.123, switch_dispatch 1.102, binary_trees 1.094,
nbody 1.089.

Three findings matter more than the ranking:

1. **lz4's true gap is ~10x.** Scaling PASSES 24 → 240 → 2400: ratio 2.75 →
   7.6 → 10.2. Fixed-cost subtraction (startup + `fill_source` ≈ 2 ms) shows
   work ratios of 8–11x throughout — the A/B's 1.94x is a startup-dominated
   artifact. Root cause (asm-constructed): match-extend at 9 insns/byte and
   literal-copy at 7 insns/byte vs GCC's word-compare + `memcpy@PLT`. LCCC has
   no loop-idiom recognition. **This is the #1 perf project** (backlog
   `PF-LZ4-1`, spec'd: byte-copy → `memcpy`, byte-compare → word-compare).
2. **mandelbrot is a vectorizer gap** (same insn count, GCC 9 packed-double
   vs our 1). Backlog `PF-MB-1`.
3. **Harness methodology**: short benchmarks understate gaps (startup
   compression). Backlog `INF-HARNESS-1`: subtract startup / scale workloads
   so work dominates (target: fixed cost < 10% of measured time).

Also note for `linux_find_bit` triage: our `bsfq` idiom IS present, yet 1.40x
with 24% more insns than GCC (176 vs 142) — needs a hot-loop diff, not an
idiom fix. Backlog `PF-FB-1`.

## 7. Snapshots and verification

| Ledger | Content | Cumulative bytes | Verdict |
|---|---|---|---|
| S04-oracle-label-fix | oracle script fix, re-based | 1,148 | APPLIES-CLEAN |
| S05-inline-single-site-static | inline.rs + check + baseline | 11,679 | APPLIES-CLEAN |
| S06-cmp-branch-fusion-generalize | compare_branch.rs + 16 tests | 55,249 | APPLIES-CLEAN |
| S07-ifcombine-profitability-guard | if_convert.rs +46 | 57,656 | APPLIES-CLEAN |

Base `417951a4` (fresh at snapshot time), `origin/main` verified equal.
Independent verification: patch `apply --check` in a fresh worktree ✓; bundle
test-cloned with all 4 commits ✓; series 0001–0004 ✓; ledger entries 4–7 ✓;
`.seq` = 7. Final `ci_local.sh --fast`: 17 passed, 0 failed, 3 skipped;
clippy `-D warnings` clean; rustfmt clean.

## 8. Backlog and next session

Backlog updated (`backlog.md`, session entry 2026-09-11): `PF-LZ4-1`
(loop idioms — the whale), `PF-MB-1` (mandelbrot vectorizer), `PF-FB-1`
(find_bit hot-loop diff), `INF-HARNESS-1` (startup subtraction), plus
carried-forward S06/S07 notes. Next session: (1) confirm `git`+toolchain
health first (wipe drill); (2) `PF-LZ4-1` design (safety preconditions for
byte-copy → `memcpy`: aliasing, overlap, trip-count forms in this IR);
(3) targeted A/B harness for single-workload iteration (the full A/B is too
slow for tuning loops).
