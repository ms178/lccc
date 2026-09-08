# S03 supreme merge: red-team audit of PR #446 + unified rebase on `1a606ff`

Base: upstream main `1a606ff` (Merge PR #446). Deliverable: that base plus
the S02 lineage (`b6d6e7bd` + #442-port + #445-port + red-team fixes +
cold-attr + tests + RA valve + ChaCha roadmap), merged file-by-file with
judged conflict resolutions, plus new contract tests and CI gates. Every
verdict below was verified programmatically: line-set containment checks,
A/B binaries of `1a606ff` and the merged tree, the GCC-differential
correctness battery, `check_loop_alignment.py`, and the full CI-equivalent
gate list.

## Archaeology (established from the object store, not from memory)

- PR #442 (`4e1e4837`, base `20261722`): ARX register pressure, rotate
  soundness, narrow rotates, SSA dominance verifier. NEVER MERGED.
- PR #445 (`1aba0bd8`, base `20261722`): conditional-map vectorization,
  hot-loop alignment, vector stack-slot fixes. NEVER MERGED (CI failed on
  a missing `cargo fmt` run and the machinst temp-dir race; both
  root-caused in `AUDIT-2026-09-08-PR444-redteam.md`).
- PR #446 (`b2ac3b9` on `b6d6e7bd`, merged as `1a606ff`): the #445
  session's rebase — #445's content carried onto the #444-containing
  main, #444's `src/passes/loop_align` adopted over the #445 backend
  hook, plus policy hardening (bounded vector cascade, terminator-fed
  tiny-trip skip), the probe-dir race fix, and CI repairs. Merged GREEN
  (Clippy, Test Suite, LCCC-vs-GCC all success).
- S02 (`46b29c90`, this lineage's snapshot): `b6d6e7bd` + #442-port
  (`d6e54908`) + #445-port/red-team/cold-attr (`cee3d6a3`) + RA valve
  (`46b29c90`).

So #446 upstreamed the equivalent of my #445-port but NOT the #442-port,
the red-team fixes, the cold-attribute wiring, or the S02 diagnostics —
and its rebase silently DROPPED the abs-fold fix (finding F1).

## Red-team findings against #446 (all reproduced on a `1a606ff` build)

F1 — LIVE MISCOMPILE, abs fold ignores the subtrahend. `fold_int_minmax`
matches the true arm with `Sub(0, _)` by its lhs alone, so
`v<0 ? 0-(v+1) : v` folds to `max(v,-v)` = abs(v). Exhibit (N=2051,
`-O3 -march=x86-64-v3`, FNV hashes vs GCC 14.2): `negsucc` lccc
`4b12e88f` == the pure-`abs` hash, GCC `3879cdd2`; `negother` likewise;
all 34 small-n sweeps mismatch. The positive shapes (`abs`/`absle`/
`absadd`, INT_MIN included) match — the fold is sound, only the guard
was wrong. S02's checked `is_neg_of(t, l)` fixes it; the merged tree
passes `vectorize_abs_fold_subtrahend`. WHY IT SURVIVED: no suite in CI
compiled that shape at vectorization flags — fixed by gating
`run_correctness.py` in CI (see below).

F2 — dead `mut` (micro). #446 wrote `let mut aligned_nls` where the
binding is never mutated (5 use sites, all reads; no `#[allow]`). Rust
1.98.1 `cargo clippy -- -D warnings` stays green either way (mechanism
unexplained — recorded, not chased), so this is hygiene only. S03
removes it; removal verified by rebuild + clippy.

F3 — stale/factually wrong lane-const comments in `emit.rs` (docs only,
code was right). #446 claims lanes emit BEFORE the scalars (the code
emits them after), that legacy `pxor mem,%xmm` "accepts unaligned
memory" (it faults — only VEX forms tolerate it), and that `vpxorpd`
"requires 32-byte alignment" (no VEX op requires alignment). All three
verified against the emitter source; S03 takes the corrected wording.

F4 — perf gap: ChaCha 4.66x. `chacha20_block` at `-O2`, lccc-vs-GCC
geometric mean over 9 paired rounds: **4.664 on #446** vs 1.601 on the
S02-lineage tree (same GCC 14.2.0-19, same Xeon 2.60 GHz box, same
runner). Attributed to the missing #442 content: RA loop-span work,
the SROA fold→copyprop→split reorder (which unsplits chacha20's
`u32 x[16]` — "the whole ARX state lived in stack slots (10x vs GCC)"),
and unroll-at-O2. PREDICTION (falsifiable): S03 recovers ~1.6x; the S03
ChaCha run below either confirms the merge or exposes a broken RA path.

Adopted FROM #446 (kept verbatim): the bounded vector cascade
(`.p2align 5,,15` + `.p2align 4`), the terminator-fed trip check
structure, the `probe_dir` race fix, the `check_loop_alignment.py`
contracts + `tiny_trip_call` corpus case, the bench.yml fastbuild
shape, and the PR444 red-team audit doc.

## Merge table (33 differing files, base `b6d6e7bd`, judged 2026-09-08)

- TAKE S02 (26 files #446 never touched, incl. `regalloc.rs` whose
  three-way merge is byte-identical to S02): ci.yml, .gitignore,
  backlog.md, register-allocator.md, i686-alu, live_range, liveness,
  regalloc, x86-alu, isel, machinst, identical_blocks, prologue, parser
  ast/declarations/parse, func_lowering, aggregate_sroa/split,
  bit_idioms, iv_widen, loop_unroll, passes/mod, vec_interleave,
  verify, verify/tests.
- KEEP #446 (S02 never touched): machinst_tests.rs (race fix),
  check_loop_alignment.py (contracts — then EXTENDED, see below).
- KEEP #446 + 1-word fix: slot_assignment.rs (F2 `mut` removal).
- JUDGED three-way merges (5 files, 9 conflict hunks, all resolved):
  - `loop_align.rs` (2 hunks): call site takes #446's form
    (semantically identical to S02's `is_some_and`); the trip-bound
    function is a SUPREME fusion — #446's terminator-fed + mirrored
    structure, S02's `Sle/Ule` (+1 bound) and `Ne` arms, S02's
    `limit >= 0` guard (a negative limit from an unknown start proves
    nothing — pad as usual), `Sge/Uge`-flipped mirrors. Lower-bound
    shapes (`iv > n`, …) prove nothing and pad. Verified shape by
    shape on the merged binary: `< 3`, `<= 3`, `!= 3`, `3 > j` all
    reach codegen unpadded; `< 100` and dynamic-bound loops keep the
    scalar cascade.
  - `vectorize.rs` (2 hunks): S02's checked `is_neg_of` + a fused
    comment (INT_MIN soundness note + subtrahend warning).
  - `emit.rs` (3 hunks): S02's corrected comments (F3); no code delta.
  - `bench.yml` (1 hunk): fused comment (shared-profile rationale +
    determinism/byte-identical rationale).
  - `run_correctness.py` (1 hunk): union — #446's +83 ⊆ S02's +121
    byte-identically; the 38-line delta is exactly the abs-fold test.
- KEEP (deliberate deviation from S02): repo-root `.base_ref` (dead
  9-byte marker S02 deleted; the snapshot guard fails closed on
  deletions, and the operative base file is `artifacts/.base_ref`).

## New work in S03 beyond the merge

1. `check_loop_alignment.py` corpus: `tiny_trip_le` (`j <= 3`),
   `tiny_trip_ne` (`j != 3`), `tiny_trip_mirror` (`3 > j`) — each
   asserted in both directions (loop survives, stays unpadded) via a
   shared `TINY_TRIP_NO_PAD` set; docstring contract updated. Audit:
   PASS (8 shapes).
2. CI gates wired (validated verbatim locally): the differential
   `run_correctness.py` gate (53/53) and the `check_loop_alignment.py`
   gate (PASS) join the Test Suite job. The abs-fold class can no
   longer land without tripping CI.

## Validation battery (merged tree, fastbuild, `-j 2`)

- build: GREEN.
- `cargo fmt --all -- --check`: CLEAN (first try).
- `cargo clippy --all-targets --profile fastbuild --locked -j 2
  -- -D warnings`: GREEN.
- correctness suite: 53/53 (abs test FAIL→PASS across the merge).
- loop-alignment audit (extended): PASS.
- benchmark-output gate: 180/180.
- `cargo test --profile fastbuild --all-targets --locked -j 2`: GREEN —
  exit 0, 7/7 binaries ok, 2124 passed / 0 failed / 6 ignored (twice:
  full build+run, then authoritative `pipefail` re-run).
- regression corpus (exact CI command, `CCC_VALIDATE_SSA=1`, `-j 2`):
  GREEN — 688 passed / 0 failed / 14 skipped-compare / 702 total
  (`failures: []` in JSON). All 14 skips are GCC-side link failures
  (`collect2: ld returned 1`) with lccc compile+run OK — environment,
  not compiler.
- ChaCha20 S03 (`-O2`, 9 paired rounds vs GCC 14.2.0-19): **1.603x** —
  prediction CONFIRMED (S02-lineage 1.6015x, #446 baseline 4.664x). The
  merge recovered the RA/SROA/unroll performance exactly (0.1% = noise);
  the supreme additions cost nothing here.

## Follow-up worked this turn: span-reserve recalibration

The S02 `CCC_RA_LOOP_SPAN_RESERVE=3` claim (+29% chacha20) does NOT
replicate on S03: 1.626x vs 1.603x default (noise) over 9 paired `-O2`
rounds. Engagement verified (313 assembly lines differ with the knob
on); the mov gap persists (232 vs GCC's 90 whole-file), so the cap
presses the wrong pressure point on this tree — the SROA/valve changes
moved the starvation elsewhere. Default stays off (unchanged); the
knob's doc comment now records the non-replication, and RA-PRESSURE-1
must re-measure its census from this tree. Evidence:
`chacha_s03.json`, `chacha_446_baseline.json` (workspace).

## Follow-ups (S02's roadmap, updated)

RA-PRESSURE-1 census + `CCC_RA_LOOP_SPAN_RESERVE` calibration
(`FOLLOWUP-2026-09-07-ARX-CHACHA-RA.md`); ChaCha-vs-GCC 1.6x→1.0x
roadmap (`FOLLOWUP-2026-09-08-UNIFIED-442-445.md`); `!=`-trip-bound
precision census on the benchmark corpus (the exclusion is
padding-only, but a hot `while(x != CONST)` loop would lose its pad).
