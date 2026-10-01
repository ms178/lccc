# FOLLOWUP — 2026-09-30 — byte-compare epic on top of main @ 8db75621

## 1. Design invariants found during development

Two failure modes were found while developing the transform; both are
fixed and documented in the code as do-not-simplify invariants:

1. **Terminator labels are label-space, not index-space.**
   `loop_info.body` is a set of block *indices*; `Terminator::*` targets
   are `BlockId`s whose numbers follow the frontend's source-order label
   allocation, and later passes reorder the block vector. A block's
   numeric label therefore need not equal its index. Comparing
   `label.0 as usize` against the index set declines silently on any
   reordered function. Every edge comparison in the section is
   label-to-label; the exit *index* is recovered once, by label lookup.
2. **Per-phi phase values.** Rewiring *both* header phis to the *p*
   phase value makes the scalar tail compare A against A (the tail's q
   register is initialised from p; 9600/9600 driver mismatches). The
   fix is `p_phi <- p_v`, `q_phi <- q_v`.

## 2. What landed

`x86: vectorize byte-compare loops (LZ4 extend family) with 32B/16B
windows` — the byte-compare epic, implemented with both invariants above in place:

- Driver arm in `vectorize_with_analysis_mode` before the byte-count
  arm; gate `!neon && !late_minmax_only() && x86_simd_available_pub()`
  (deliberately no FORCE_SSE2/AVX2 exclusion: the transform picks
  32B AVX2 / 16B SSE2 itself).
- Strict 3-block grammar `[Phi,Phi,Cmp]` header / `[Load,(Cast),Load,
  (Cast),Eq]` compare / `[Add(p,1),Add(q,1)]` latch, fail-closed on
  every deviation, no-store check, loop-invariant bound check,
  label-space edge chains (header→compare→latch→header, shared exit).
- Transform prepends `vh`/`vbody`/`vstep` (room test reuses the
  frontend's original bound op — signedness preserved), keeps the
  original scalar loop intact as the fallback tail; pre-mutation id
  allocation; per-phi phase values; 2-arg `VecLoadI8xNN (base,0)`,
  3-arg `VecCmpI8xNN (a,b,0)`, 1-arg `Pmovmskb`, full mask
  `-1i32`/`0xFFFFi32`.
- Kill switch `LCCC_NO_BYTECMP_VEC`; decline tracing under
  `LCCC_DEBUG_VECTORIZE`.

## 3. Hard data

| Check | Result |
|---|---|
| Execution driver, 9600 cases (len 0..399 × 8 mismatch positions × 3 seeds), x86-64 baseline (16B) | **OK, 0 failures** (gcc parity) |
| same, x86-64-v2 (16B) | **OK, 0 failures** |
| same, x86-64-v3 (32B AVX2) | **OK, 0 failures** |
| Kill switch (scalar path) | OK, 0 failures |
| Emitted loop | byte-idiom identical to GCC 16.2: `leaq 32B / cmpq / jae tail; vmovdqu×2; vpcmpeqb; vpmovmskb; cmpl $-1; jne tail; addq $32×2; retest` with the original scalar tail (`movq %rdi,%r10; movq %rdx,%r9` — q from its own register, the per-phi fix) |
| Regression corpus, final rebased tree (`CCC_VALIDATE_SSA=1`, 862 cases) | **849 passed, 0 failed**, 13 skipped-compare (gcc cannot compile them), 0 skipped-run (pre-rebase tree: 847/847 of 860) |
| LZ4 (16 MiB patterned, best-of-5, this box): lzc_ll / lzc_hc / lzdec | gcc: 4/392/2 ms · lccc bytecmp-ON: 3/529/4 ms · lccc bytecmp-OFF: 3/539/4 ms |
| `cargo test` (unit + doctests) | **3882 unit tests passed, 0 failed** (also re-run green inside `ci_local.sh`); one pre-existing upstream doctest failure (unfenced prose in `late_vectorize_entry` docs, commit 67cabbee) fixed with a text fence (`905d8943`) |
| 39-benchmark corpus sweep, final rebased tree (lccc vs gcc, reps 15) | geomean lccc/gcc **0.7262** (final binary on 3e0c36cb; 0.7278 on the 7b3958f6 run), 39/39 correct, 0 failed; best `constant_recursion` 0.013 (78× faster than gcc), worst `linux_find_bit_scaled` 1.415; in-corpus transform hits: `lz4_compress` 1.009, `zstd_count` 1.086, `lz4_match_extend` 1.176 |

**In-corpus A/B (the transform's actual effect, hard data):** a
per-program probe with `LCCC_DEBUG_VECTORIZE=1` shows the arm fires on
exactly two corpus programs — `lz4_compress` and `zstd_count` (and, via
the repeat-rich input, `lz4_match_extend` runs the same source). Paired
15-rep sweep of those three, same binary, `LCCC_NO_BYTECMP_VEC` ON vs
OFF (child CPU median):

| bench | ON (ms) | OFF (ms) | OFF/ON |
|---|---|---|---|
| lz4_compress | 3.62 | 3.73 | 1.031 |
| **lz4_match_extend** | **1271.03** | **1786.99** | **1.406** |
| zstd_count | 11.21 | 11.34 | 1.011 |

The transform makes `lz4_match_extend` **40.6 % faster**; without it
that benchmark is 1.62× the gcc time (1787 vs 1104 ms), with it
1.15× (1271 vs 1104 ms). No other corpus program is touched (all
others decline fail-closed), so the rest of the corpus is byte-identical
to the OFF build by construction.

**LZ4 library note (honest read):** the current LZ4 master's extend
path is `LZ4_count`/`LZ4HC_countPattern` — *word* (8-byte) loops with
`NbCommonBytes` refinement, hand-unrolled in source. The byte-compare
shape (`while (p<end && *p==*q)`) is not present in current LZ4, so the
transform does not fire in lz4.c/lz4hc.c; the ON/OFF delta above the
line (529 vs 539 ms HC, within this box's noise) confirms no regression
and no gain there. The transform's home is the shape itself (the
corpus's `lz4_match_extend` is the real instance; older-lz4
`countReplica` / memmem-style two-stream scans are the family). The
remaining lccc-vs-gcc HC gap on the library bench (529 vs 392 ms;
dynamic instruction count ratio 1.72× via callgrind summaries) is
pre-existing and pre-dates this change — it needs hardware counters to
root-cause on this 1 GiB box (see §5, item 1).

## 4. Red-team of the byte-compare transform (self, per directive)

Attack list, verdicts:

1. **Room-add overflow near top of address space** (`p_v + 32` wraps,
   room test true, OOB window load). — *Accepted risk, documented.*
   GCC 16.2's idiom has the identical property (`leaq 32(%rdi)`);
   signed pointer overflow is UB in C, so parity with every shipping
   vectorizer is the defensible position. The section epic states it.
2. **Preheader with p_pre == end / > end (empty run).** — *Safe.* Room
   test false (modulo #1), header test false, exit; no load issued.
   Covered by the driver (len 0 cases, all pass).
3. **vbody false-edge → header re-entry:** does the tail re-scan from
   the window start? — *Yes by construction*: the header phi takes
   `p_v`/`q_v` (window start) on that edge and the original compare
   block re-tests byte 0. Driver mismatches at *every* position in the
   window pass (rel 0..7 × len 0..399).
4. **Phi with 3 incomings (vh, vbody, latch) confusing later passes.**
   — *Covered*: SSA validation is on in the regression gate (847/847);
   all three edges are true predecessor edges.
5. **Multiple preheaders / second non-loop edge into the header.** —
   *Fail-closed*: the phi incoming check requires exactly
   {preheader, latch}.
6. **Aliasing q into [p, end).** — *Preserved*: the window loads read
   the same addresses in the same order as the scalar iterations;
   volatile loads are excluded by the grammar.
7. **end_val modified inside the loop / loop-invariant breach.** —
   *Checked*: `defined_in_loop(end_val)` decline.
8. **32-bit pointers (i686).** — *Deliberately unsupported* (header phi
   type gate is `Ptr|I64`); the arm is fail-closed there. i686 keeps
   its scalar loop. Documented in the red-team record, not a bug.
9. **Kill switch coverage.** — *Verified*: `LCCC_NO_BYTECMP_VEC=1`
   disables only this arm; 9600/9600 on the scalar path.
10. **`changes += 11` bookkeeping.** — cosmetic (debug reporting only);
    matches the house style of the neighbouring arms.

**Verdict: agree with the design.** The two failure modes found during development
(label/index conflation, shared phase value) are
documented as hard-won invariants with the exact regression symptom
each one would produce, so a future "simplification" that re-introduces
them fails loudly instead of silently.

**Disagreement (self-correction of an earlier assumption):** the
transform was initially framed this transform as "the LZ4 extend-loop fix".
Current LZ4 does not contain the shape (word loops instead), so the
LZ4 performance case rests on the corpus benchmark, not on LZ4 itself.
The claim is corrected in §3 above.

## 5. Open items (ordered)

1. **lccc-vs-gcc LZ4 HC gap (1.35× wall, 1.72× instructions)** —
   pre-existing. Needs hardware counters (perf on bare metal) for
   per-instruction attribution; valgrind's line attribution is
   unreliable on PIE/`-g` mixes here, and `perf` is not in the
   Debian repos. Static diff done: gcc's inner HC loop is ~1.7× wider
   (more unrolling per iteration); no per-iteration global-address
   rematerialization found in the current build (the earlier
   "2 leaqs/iteration" evidence predates the current codegen —
   re-derivation required before touching `machinst_alloc`).
2. **SSE2 path codegen quality:** the 16B path is correct but the vec
   load/store state machine spills both vectors to the stack
   (`movdqu %xmm0, 80(%rsp)` round-trips) in `cmp_ext`; a
   register-home improvement in the vec state machine would shave the
   tail. Cosmetic; measured, correct.

## 6. Landing gate fixes (recorded for the review)

The first full `scripts/ci_local.sh` run on the final tree exposed three
gate findings, all fixed in the delivery commit:

1. **rustfmt** — two of the new call sites in the epic needed canonical
   formatting; `cargo fmt` touched only `vectorize.rs` (the rest of the
   tree was already fmt-clean).
2. **env-test-hygiene** (the pipeline env-read ratchet, 157) — the new
   `LCCC_NO_BYTECMP_VEC` kill switch is the one new read site; budget
   raised to 158 with the measured count and rationale in
   `tests/regression/check_env_test_hygiene.sh` (house style: every
   codegen epic carries a kill switch; the arm's debug trace reuses the
   already-counted `LCCC_DEBUG_VECTORIZE`).
3. **codegen-quality-gate** (code-size ratchet) — `zstd_count` gains
   13 insns / 3 moves (+10.6% / +11.1%) from the vector phase. The
   firing is correct: that program's line-56 loop
   `while ((pIn < pInLimit) && (*pIn == *pMatch))` is the genuine
   byte-compare shape (ZSTD_count tail path); trip count is
   data-dependent, so no static guard can distinguish it from the 40.6%
   winner in `lz4_match_extend`. The gate's sanctioned path for an
   intentional landing change is `--update-baseline`; the baseline was
   refreshed and the A/B data above documents the trade (a 12 ms bench
   pays ~1% runtime for the coverage; the win is elsewhere).

## 7. Verification at base 8db75621

Measured locally on the final tree (not re-run by CI at authoring time):

- 9600-case byte-compare driver: 0 failures (16B, v2 16B, v3 32B, kill
  switch).
- Regression corpus: 849 passed, 0 failed (862 total).
- Codegen-quality gate: all golden workloads within tolerance against
  the refreshed baseline.
- env-test-hygiene PASS, rustfmt clean, clippy `-D warnings` clean.
- Canonical 39-benchmark sweep: geomean lccc/gcc 0.7270 (arithmetic
  0.9433), 39/39 correct; `zstd_count` 1.087, `lz4_compress` 1.012,
  `lz4_match_extend` 1.150.
