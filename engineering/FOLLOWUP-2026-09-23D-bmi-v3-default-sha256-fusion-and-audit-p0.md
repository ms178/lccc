# Follow-up S01/S02 — BMI v3 default, sha256 fusion resurrection, and Review-AI audit P0 hardening

Session date: 2026-09-23. Base: upstream `b228995` (main unchanged this
session). Start of turn: the EPYC 9V45 benchmark report showed geomean
0.6945 (LCCC ahead overall) but chacha20_block 2.19× and sha256_transform
1.32× regressions; Review-AI audit of PR #604 in hand (verdict 8.0/10,
findings F1–F10). End of turn: snapshot S02 — BMI v3 default fix plus the
two list-drift fusion fixes measured as a sha256 WIN on Intel (0.966), all
P0 audit items landed with tests, ch-maj gate re-pinned.

## 1. PRIMARY: the EPYC regressions

### chacha20_block 2.1914 — diagnosed, NOT fixed this session (structural)

- PR #604 EXONERATED: the benchmark predates it (added by PR #512); the
  ARX vectorizer path involved shipped before #604 and is not regressed
  by it.
- Root cause: the ARX vectorizer's cost model counts instructions only.
  On Zen5 the vector-int ALU runs at 3 cycles/issue vs 1 cycle scalar,
  so the vectorized chacha quarter-round loses ~2× despite fewer
  instructions. The scalar fallback is ALSO broken: `live_range.rs`
  structural auto-arming + `MAX_SPAN_REMCOST=100` force ~24
  store-forwarded reloads per round (148-insn loop vs GCC's 130).
- Decision: keep the vector default (fixing scalar breaks Raptor Lake
  tuning; flipping the default is not justified). The scalar fix is a
  register-residency rework — multi-session work, not a patch.

### sha256_transform 1.32 — ROOT-CAUSED AND FIXED (the big find)

Symptom under the restored BMI default: the two K/W `addl mem,%r15d`
fusions of the compression loop vanished (paired new/base = 1.088 on
Raptor Lake — a regression introduced BY enabling BMI).

Root cause: **two independent dest-only mnemonic lists had drifted, and
both were missing `rorx`:**

1. `peephole/passes/helpers.rs` — `is_read_modify_write` (fix alone
   insufficient: still 1.017, zero fused adds).
2. `peephole/passes/liveness.rs` — `is_pure_write_mnemonic` (the
   decisive one). Its unknown-mnemonic model is *read-all-mentioned,
   write-nothing*, so every `rorxl` was treated as a phantom READ of its
   own destination; the K/W register family stayed "live", the fusion
   gate `live_after == Some(false)` failed, and load-op fusion declined.

Fix: `rorx` added to both lists; the liveness list also gained the rest
of the BMI dest-only class (`andn bzhi bextr pext pdep blsr blsi
blsmsk`). `mulx` is deliberately OUT (implicit %rdx read + two dests).
Cross-reference comments now tie the two lists together — this is the
exact bug class the reviewer's F3 drift warning described.

Measured (paired interleaved median, Raptor Lake, new/base):
sha256_transform **0.9658** (was 1.088), chacha20_block 0.9609,
sqlite_varint 0.8800, expat_xml_scan 0.9804, hash_table 1.005 — no
regressions anywhere; 3290 unit tests green before the wipe.

## 2. Review-AI audit adjudication (treated critically, per standing order)

| # | Finding | Verdict / action |
|---|---------|------------------|
| F1 | No unit tests for `machinst_window_rax_free`/`machinst_window_defs` | **CONFIRMED, fixed** — new `machinst_window_predicate_tests` (4 tests: implicit-clobber rejection, operand-position rax scan incl. MemIndex, def-set extraction per arm, FMov/Mov128 pinning) |
| F2 | Multi-block majority-rank branch untested (fixture hardcodes block 0) | **CONFIRMED, fixed** — `maj_kept_term_rank_prefers_entry_and_cross_block_defs`: same-block order, cross-block ⇒ rank (0,0), None ⇒ (0,0), plus the fail-closed repurposed-slot gate (two location-less Ands decline). Discovered en route: the strict `<` tie-break can never see equal (0,0) candidates — documented, not dead code |
| F3 | andn predicate triplicated (fold-time / emission / fusion) | **CONFIRMED** — no mechanical merge (the three sites live in different layers with different inputs); instead an 8-row AGREEMENT TABLE test (`mux_ch_andn_defer_matches_backend_fusion_criteria`) asserts middle-end defer ⟺ backend-fusion transcription across BMI on/off, both type-mismatch directions, width domain, multi-use Not, adjacency gap. The sha256 rorx drift above is the bug class this table guards |
| F4 | `check_store_alu_cross_join.sh` weakened to registry grep | **DISAGREE (deferred)** — the registry grep still fails if the pass is deregistered; a full cross-join re-strengthening is P2, tracked below |
| F5 | rbtree derived-IV gate removed without pinning default-off | **CONFIRMED, fixed** — gate extracted to `ivsr_scalar_derived_enabled`; two EnvGuard tests pin unset⇒off, opt-in⇒on, IVSR-off⇒off. The gate script's REMOVED comment already carried the measurement provenance |
| F6 | andn census `grep -c 'andn'` unanchored | **CONFIRMED, fixed** — anchored mnemonic regex `^[[:space:]]*andn[lq][[:space:]]` in `check_ch_maj_codegen.sh` |
| F7 | acc substitution register-class-blind; `machinst_window_defs` ends `_ => {}` unlike exhaustive sibling | **PARTIALLY CONFIRMED, pinned** — the new tests pin the asymmetry with the fail-safe argument (Mov128/FAlu vregs never enter the integer acc cache, so the extractor fall-through cannot enable an unsound acc substitution). Full register-class threading stays P2 |
| F8 | `CCC_IVSR_SCALAR_DERIVED`/`CCC_NO_ANDN_FUSION` undocumented | **CONFIRMED, fixed** — new "Codegen kill switches and opt-ins" section in scripts/README.md (also covers `CCC_NO_BOOL_ALGEBRA`, `CCC_NO_RORX`) |
| F9 | Unguarded `next()` in gate checks C/D | **CONFIRMED, fixed** — check C now reports "compression loop bounds not found" instead of a StopIteration traceback (D was already guarded) |
| F10 | Wording overclaims / stale names | **PARTIALLY CONFIRMED** — `check_nbody_rbtree_perf_shapes.sh` keeps its name (renaming touches ci_local.sh + ci.yml for zero functional gain; the header documents the removed rbtree check and the measurement provenance); overclaim sweep done in this doc |

Re-litigated per standing order: NOTHING. The two verified-deliberate
decisions (Sub excluded only from the rhs-resident path; (Or,Xor)
majority rejection) stand untouched.

## 3. Gate re-pinning after the BMI default (evidence-based, no reverts)

`check_ch_maj_codegen.sh` section B previously pinned "baseline ⇒ 0 andn".
With BMI now the v3 default that pin is obsolete BY DESIGN. Updated with
measurement: bare `-O2` must carry exactly 1 anchored `andnl` (the CH
fold), `-mbmi` likewise, and only the sticky `-mno-bmi` denial may drop
to 0 (verified all three against the current binary before pinning).
Gate PASS confirmed pre-wipe. The golden asm-shape gates elsewhere were
NOT swept this session — see TODO.

## 4. Harness wipe survival incident (process note)

Mid-session the harness wiped `~/.cargo`, `target/`, swap, AND
`lccc/.git` (and ~90 source files including all three files edited this
session) between tool calls. Recovery that worked:
`artifacts/lccc.bundle` (S01) → pack extracted with `git index-pack`
after stripping the bundle header (plain `git bundle unbundle` failed on
truncated history) → S01 tree materialized with `git archive` → missing
files backfilled with `cp -a --no-clobber` → the three lost files
reconstructed verbatim from session context → repo rebuilt (`git init` +
pack copy + `reset --mixed e25fd3e`) → S02 committed and
`/home/user/ms178-1.patch` (42 465 B, 9 files) published APPLIES-CLEAN
BEFORE the toolchain finished restoring. Lesson re-confirmed: snapshot
after EACH validated unit, never batch.

## 5. TODO (next session, priority order)

1. **RE-VERIFY S02 after the wipe rebuild**: cargo test (3290+ expected),
   clippy, rustfmt, ch-maj gate, and a sha256 paired A/B — the
   reconstructed emit.rs test module was written post-wipe and compiled
   only in-session pending this check.
2. Sweep remaining golden asm-shape gates for BMI-default pin drift
   (ci_local.sh --fast full run; update pins only with measured evidence).
3. F4: re-strengthen `check_store_alu_cross_join.sh` beyond the registry
   grep (P2).
4. F7: thread register class through the acc-substitution path instead of
   relying on the fail-safe argument (P2, needs an XMM-in-window repro).
5. chacha20 scalar RA rework (multi-session): kill the structural
   auto-arm for ARX loops or cost rematerialization honestly; target the
   130-insn GCC loop on Zen5.
6. linux_find_bit 1.42, sieve 1.25, glibc_strstr 1.11, fannkuch 1.11 —
   next oracle-distillation candidates (godbolt GCC 16.2 / Clang 23.1).
7. Rebase onto latest ms178/lccc main before final submission (main
   unchanged this session; base ref still b228995).
