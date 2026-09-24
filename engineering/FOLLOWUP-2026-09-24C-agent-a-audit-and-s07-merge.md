# FOLLOWUP 2026-09-24C — Agent A red-team audit + S07 merge (both-work union)

Session: deep red-team audit of `ms178-1-AgentA.patch.txt` (S49 lineage: chacha20
ARX + integer-v3 completion + Review-AI work order) against my S06, then a full
merge of both works into one tree. Everything below is empirically verified on
the merged tree unless explicitly marked "claim, re-measure on EPYC".

## Verdict (quantified)

| Axis | Winner | Evidence |
|------|--------|----------|
| chacha20-block perf design | **Agent A** (adopted) | frame 0xd0→0x60 (`subq $96`), `.LCVEC` const-pool rotate masks (4 refs in default -O2 asm), chacha self-check `ff1bc75f6884e79f` identical to S06; their claim 1.036× default / 0.842× `-march=native` vs gcc-14.2 (EPYC re-measure outstanding) |
| integer-v3 architecture | **Agent A** (adopted) | `resolved_*` ×5 (BMI1/BMI2/LZCNT/POPCNT/**MOVBE** — S06 missed MOVBE entirely) with the `Target::X86_64` capability lock; last-flag-wins clearing; `-march=native` CPUID permission incl. avx512vl (KNL-correct) |
| i686 andn-defer | **Agent A fixed my bug (FIND-A)** | S06 `bmi1_effective()` armed the andn fold-DEFER under i686 + `-mbmi`, where the i686 backend can never emit andn → infinite defer cliff. `resolved_bmi1()`'s X86_64 lock is the correct gate |
| predefined macros | Agent A complete, S06 incomplete | S06 left `__POPCNT__` and the macro wiring open; A wired the full set (FIND-C). Their macros=capability choice (i686 `-mbmi` does **not** define `__BMI__`) diverges from GCC but is the fail-closed house policy (matches `set_sse_macros(no_sse)` precedent); pinned by `test_i686_bmi_macro_follows_codegen_capability`. Flip to GCC parity when the i686 backend gains BMI/ABM emission |
| gate hygiene | **Agent A fixed my defect (FIND-D)** | my `check_clz_ctz_target_isa.sh` still contracted the pre-S05 default ("NO lzcnt/tzcnt/popcnt") AND was not wired into `ci_local.sh` — my 64-gate green never executed it. Their rewrite + wiring is correct and tested |
| andn agreement contract | **A deeper, mine wider — fused** | A: 12-row table against a real `backend_andn_fusion_reference` transcription of `detect_and_not_fusions` + cross-block pin + `has_scalar_andn` layer-3. Mine: 8-row canned-bool table with the I8-width, consumer-wider-than-Not, multi-reader rows. A's architecture adopted; my position/width rows expressed in the fused suite |
| window predicate tests | **fused (union)** | A pins every MachInst defining variant (16) + xmm-omission fail-safe + FAlu rax-scan sibling semantics; mine pins the Mov-src over-approx contract, source/base omissions (Movzx src, ShiftX count/src, Lea base), CallArgMove + ShiftX-count `%rax` poisons. All in `machinst_window_tests` (6 tests) |
| store_alu inert-pass gate | **Agent A (adopted)** | F4 completion template: `fold_store_alu_memop` call count ≥ 2, sk() gate parity, no-default-disable scan, residual hole stated. The right shape for any deliberately-inert pass |
| ch-maj census | **Agent A (adopted)** | F6 fix: region-scoped to `sha256_transform` AND mnemonic-anchored, 4 configs (default, `-mbmi`, `-march=x86-64`, `-mno-bmi`) |
| regalloc chain collector | **Agent A (adopted, verified)** | their copy-prop fixpoint feeds MY `allocate_vector_registers_destructive`; proposals are interference-checked, so the tightened set is sound. Verified: suite 3334/0, S06 kernel shapes unchanged (sieve 0 movsbq, strstr 173, fannkuch 138), chacha frame 0x60. Second consumer at regalloc.rs `sse_destructive_tests::run` is test-only |
| my S06 peephole work | **mine preserved** | narrow-copy identity N1–N4, imul-NDD phantom-RMW fix, orphan retirement (`retire_dead_pure_writes` ×4), popcnt default: sieve movsbq 0, strstr 176→173, fannkuch 138 all hold on the merged tree (Agent A's tree had sieve movsbq 2) |

## Merge record

- 13 files stacked clean (regalloc, intrinsics, predefined_macros, arx_vectorize,
  vec_arx, ivsr, passes/mod, vectorize, backend/mod, clz_ctz gate, store_alu gate,
  nbody rename, README + their two engineering docs).
- 8 files 3-way-merged (`git apply --3way`): ci.yml + ci_local.sh auto-merged;
  25 conflict blocks in ch_maj/vectorize_isa/cli/pipeline/bit_idioms/emit resolved
  to the union. Naming unified on `resolved_*` + `*_explicitly_disabled` (theirs —
  the clean-applied files referenced it); my soundness docs (tzcnt ≥ bsf argument,
  popcnt trivial-equivalence) folded into `resolved_lzcnt`/`resolved_popcnt`.
- Duplicate helpers unified: `slot_width_fits` (mine) merged into `slot_fits_width`
  (theirs, P0-4 single-source doc); duplicate width-rule doc kept theirs + my intro.
- FIND-G fixed: "A dead `iv * C` earns no recurrence." re-attached to
  `test_scalar_derived_iv_dead_mul_untouched`; the default-off pin test now carries
  its own accurate doc.
- FIND-B adjudicated (documented above): kept their macros=capability policy.

## Final state (all on the merged tree)

- `cargo test`: **3334 passed / 0 failed / 0 ignored** (both test families fused).
- `ci_local.sh --fast`: **64 passed / 0 failed / 3 skipped — ALL GATES GREEN**
  (63 = A's wiring incl. rewritten clz_ctz + renamed nbody; +1 my findbit gate).
- clippy `--all-targets`: 0 warnings/errors. `cargo fmt --check`: clean.
- chacha20_block -O2 default: frame `subq $96` (0x60) + `.LCVEC`×4; self-check
  `ff1bc75f6884e79f` = the S06 invariant (byte-output identity across both works).
- S06 kernels preserved: sieve `movsbq=0`, strstr 173 insns, fannkuch 138 insns.
- bitops invariant `pop=800020159 clz=49986357 rev=6375100514 pow2=2184481749767`.
- Snapshot: head `8cd4f1fc` lineage, deliverable `ms178-1.patch` **APPLIES-CLEAN**.

## TODO bank (priority order; B1–B8 diagnoses adopted from Agent A §B, verified shapes)

1. **B1 sieve redundant multiply** (HIGH/med): `i*i` computed 3×/outer-iter
   (LBB1 bound, LBB3 inner-start, LBB6 backedge; gcc 34 vs lccc 47 insns in
   `count_primes`) — global CSE/GVN miss across the `if (sieve[i])` diamond.
   Generalises to every search-then-squared-index loop.
2. **B2 callee-saved homing for call-live values** (HIGH/high): expat scan hash
   spilled per-char; gcc homes %r12 across the call (gcc 97 vs lccc 156 insns).
   RA policy: live-across-call ⇒ prefer callee-saved home, cost = call freq ×
   spill traffic. Must not pessimize leaves.
3. **B3 chacha → ICX structure** (HIGH/high): (W5a) copy-in forwarding — replace
   `x[i]=in[i]` preheader copies with VecLoad lane groups; (W2b) latch-copy
   elimination via destructive-chain header-phi homing (highest risk — full
   battery + ARX latch gate); (W5b) exit `out[i]=x[i]+in[i]` feed-loop →
   VecLoad/VecAdd/VecStore. ICX 74 vs lccc 132 insns; rotates already best.
4. **B4 andn memory-operand fold** (med/low): find_bit 5→4 insns/word via r/m
   inversion + operand swap in `emit_and_not_impl` + fusion operand prep. Extend
   the ch-maj andn census, never weaken.
5. **B5 IV widening follow-through** (med/med): sieve outer loop keeps `movslq`;
   find why `CCC_NO_IV_WIDEN`'s pass vetoes (legality vs inner 64-bit use?).
6. **B6 mem-operand compare** (low/low): expat `cmpb (%rax),%r8b` selection for
   single-use loads feeding compares + census of sibling misses.
7. **B7 count-loop sbb idiom** (low/low): `cnt += (x == 1)` → `cmpb/sbbl $-1`,
   after B1/B5 (tail is cold).
8. **B8 sha256 control-flow gap** (med/TBD): clang 126 / gcc 154 / lccc 159 insns;
   lccc ~2× branches (4× jl + 4× jge + 5× cmpq + 4× cmpl vs clang 2× jne + 2×
   cmpq). lccc already wins rotate selection (6× `rorx` 3-op). Loop-by-loop study
   against the committed oracle artifacts; do not guess from histograms.
9. **FIND-B flip**: i686 macros → GCC parity (`-mbmi` ⇒ `__BMI__`) when the i686
   backend gains BMI/ABM emission or builtin support; until then the fail-closed
   capability-macro policy stands (documented divergence).
10. **`supports_and_not` single-spelling** (Agent A §D debt): the env
    `CCC_NO_ANDN_FUSION` read is gate-equivalent to `scalar_andn_available()` but
    still a dual spelling — fold the env check into the helper.
11. **`machinst_def_use` helper** if a third def/use consumer appears (both
    sides flag the two duplicated matches).
12. **EPYC re-measure**: chacha 1.036×/0.842× and bitops −25.0% claims (2-vCPU
    source host; interleaved-idle A/B protocol per PERF-PROVENANCE-S49 §3),
    plus the full 39-corpus run of the merged tree.
13. **Godbolt manifests**: commit the exact API payload the oracles consume
    (Agent A §D).

## Method notes (reusable)

- Harness wipes destroy every `.git` dir at any depth and the toolchain; keep
  backups OUTSIDE any `lccc*` path, re-constitute via
  `git clone https://github.com/ms178/lccc` + `git apply ms178-1.patch` (patch is
  the canonical artifact; hashes may renumber), re-run `setup_swap.sh`, rebuild
  rustup + `pip3 install cmake ninja`.
- Both-agent patches on one base: `git apply --3way` reconstructs real conflict
  blocks even when the subjects' trees are gone (base blobs in the object DB
  suffice); fuse "union of strengths", never one side wholesale.
- When both sessions fix the same bug independently (slot width rule, andn
  agreement table, F1 window tests), expect parallel names and near-identical
  kernel evidence (the zstd `ZSTD_decodeLiteralsBlock` case appears in both) —
  that is corroboration, not copying.
