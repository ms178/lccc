# Follow-up: FP min/max/compare/blendv vectorization (Fable #2, S06 → S11-ULTIMATE)

Session S06 (base `f41363a`, deliverable `ms178-1.S06-fp-minmax-select-vectorization.patch`).
Session S09 (base `694f6fa` = f41363a + PR #408; the audit revision — R1/R2/R4/harness
fixes + restored regressions; delivered as `ms178-1.patch`, md5 e3f088af).
Session S10 (base `2574c30` = 694f6fa + PR #409 "x86: promote legacy SSE to VEX";
competing "Agent A v2" patch — vex_promote audit/refinement + reduction-tail coalescing).
PR #410 upstream merged a 4148-line subset of v2 (vex_promote refinement and the
reduction-tail work were dropped); the v2 doc's audit narrative was merged without the
code fixes it describes.  **This revision (base 55051627 = main) is the reconciliation:
the absolute best of S09 and S10, with every claim re-verified against the code.**

## RED-TEAM AUDIT — S09 revision vs Agent A v2 (which is better, and why)

**Verdict: the S09 revision is the correct base; Agent A v2's S10 feature work is the
correct extension.  The ultimate revision = S09's soundness fixes + v2's S10 work,
with one soundness correction applied to the latter.**

Evidence, per item (all executable or line-level, none assumed):

1. **CRITICAL — unsound arm-load speculation** (`if_convert.rs`).
   *S09*: replaced with the path-coverage rule.  *v2*: doc claims the replacement
   ("Replacement: the exact path-coverage rule … Regression:
   `tests/regression/ifconv_speculative_load_coverage.c`") but the code still ships
   the unsound IV gate AND the cited regression file is absent from the patch —
   a doc/code mismatch.  *Executable proof (re-verified against main 55051627 this
   session)*: with `c` backed by 16 bytes before a PROT_NONE guard page and the false
   arm taken only for `i < 4`, a compiler with the IV gate **SIGSEGVs** on a program
   whose C semantics cannot fault, while GCC 14 (-O3, x86-64-v3) runs it clean and
   lowers `c[i]` behind `vmaskmovps` — masked, not speculated (measured, not assumed).
   This revision ships the path-coverage rule: an arm load of canonical key K is
   speculatable iff K is dereferenced on every path pred→merge (`K ∈ derefs(P) ∪
   derefs(other arm)` for diamonds, `K ∈ derefs(P)` for triangles) — trap-for-trap
   equivalent.  Regression: `tests/regression/ifconv_speculative_load_coverage.c`.
2. **CRITICAL — non-injective canonical address key** (`if_convert.rs`).
   *S09*: shift constants recorded in the key (`gep(base@root<<k…)`).  *v2*: doc claims
   the fix; the code still collapses through `Shl` without recording k.  With the old
   key, `d[i]` (off `i<<2`) and `d[2*i]` (off `i<<3`) share one key:
   `sink_conditional_stores` merged stores to two different addresses into one store,
   and `rewrite_covered_arm_loads` replaced an arm's `d[2*i]` load with the pred's
   `d[i]` value.  Regressions: `tests/regression/ifconv_store_sink_distinct_scales.c`
   (both consumers) + unit test `canonical_addr_key_distinguishes_shift_scales`.
3. **PERF — blendv deferred-mask flush** (`intrinsics.rs`): the
   `(mask not homed, true homed)` arm loaded the false vector before the mask,
   flushing the deferred cmp result to its never-written slot and re-reading it
   (one dead store + one stack load per iteration).  *v2* kept the flushing order in
   this arm (only the `(None, None)` arm is mask-first).  Both orders are sound; this
   revision resolves the mask first in both arms.
4. **Harness determinism** (pre-existing, exposed by S06's 10 new tests): the
   `kill_switch` tests in `vec_interleave.rs`/`vec_load_sink.rs` mutated the
   process-global `CCC_NO_VEC_*` env around `run()`; under cargo's default test
   parallelism the window raced with every other test's env read and randomly
   disabled the pass mid-suite (`interleave_dot` failed only in full runs).  Both
   modules serialize through an `ENV_LOCK` mutex held across the whole
   set/run/remove window.  *v2 did not include this fix.*
5. **Restored session-8 regression files** (`peephole_cascaded_shift_far_use.c`,
   `peephole_recurrence_3op_imul.c`): merged upstream as code fixes without their
   test files; re-added here.  *v2 did not include them either.*
6. **v2's S10 work — audited and adopted, with one soundness correction**:
   - `vex_promote.rs` refinement (PR #409 red-team): reg-reg `movd`/`movq`
     hard-blocked (legacy merges dst[127:32/64], every VEX spelling zeroes them, no
     three-operand encoding exists — a latent upstream soundness hole, unexercised);
     SSE2 immediate-less `pextrw` guarded (no VEX spelling); `movntdq/movntdqa/
     movntps/movntpd/aeskeygenassist` added to the table; the whole-function
     `%ymmN`-nowhere gate upgraded to per-line reachability masks (reverse BFS over
     the emitted text: labels/fall-through/jmp/jcc resolved, indirect/unresolved
     fail closed to all lines; write-aware read detection with dest-position
     exclusion; kill-aware absorption — VEX.128/VEX.256 dest writes and
     `vzeroupper` kill, masked/EVEX lines never kill).  Audited line-by-line this
     session: **sound** (fail-closed on every unresolved edge; read+kill lines
     propagate; masked lines neither read-block nor kill).  Adopted verbatim with
     its 17 unit tests.
   - Reduction-tail coalescing (follow-up #3): horizontal-reduce ops admitted to
     `collect_non_gpr_values`; new `fp_copy_web_groups` (union-find over Copy edges
     among scan-eligible FP values; merged leader interval; members filtered out of
     the scan inherit the leader's register — call-spanning, zero-length and
     already-assigned members never inherit); direct-emit for
     `VecHorizontalAddF64x4/F32x8` writing the reduction steps straight into the
     destination register.  **One soundness defect found and fixed here**: the
     direct-emit's dest==`%xmm1` fallback used `%xmm2` as scratch, but `%xmm2` IS in
     the F64 scan pool (`regalloc_helpers.rs`: pool = xmm2–xmm7) whenever the
     function lacks the listed xmm2-clobbering emitters — a different value homed in
     `%xmm2` live across the intrinsic would be corrupted, and the dest==xmm1 case
     is only knowable AFTER allocation (so the function-level `clobbers_xmm2` gate
     cannot cover it).  Fix: dest==`%xmm1` falls back to the legacy sequence +
     `store_xmm_to` (one move, provably safe — `%xmm1` is scratch-only and never in
     any pool); the direct-emit path now only fires for pool-homed dests
     (`%xmm2`–`%xmm7`, `%xmm0` retval homes) where `%xmm1` scratch is safe.
   - Honest perf note kept from S09: 9/12 kernels in `vectorize_fp_minmax_select.c`
     keep packed lowering (asm-verified `vminps`-memfold / `vblendvps`);
     k_select3/k_select_thr/p28_select stay branchy until masked loads (follow-up 0);
     the "p28 beats GCC 16.2x" oracle line measured the unsound build and stays
     retracted until then.

Honest consequence of 1: `k_select3`, `k_select_thr` and p28_select keep the branchy
scalar lowering until masked loads exist (follow-up 0 below).

## S11: re-base on PR #411 (4927a774) — audit, fixes, full battery

PR #411 (`fba657d4`, "Fix phi-latch coalescing miscompile hidden by folded SIB
indices", co-authored by arena-agent) was audited on merge.  The veto concept is
sound (a destructive coalesce must prove the phi's OLD value unread inside the
update window); three defects were found and fixed, one of them in PR #411's own
regression suite as merged:

1. **PR #411's `accepts_materialized_binop_escaping_source_block` unit test
   FAILED as merged** (1911/1 on pristine upstream).  The syntactic peel closure
   treated every constant-operand `Shl/Mul/Add/Sub` derived from the phi as a
   hidden SIB read, but the backend only absorbs such chains when they FEED a
   dereferenced GEP.  Fix: an `addr_fed` precompute (backward walk from every
   Load/Store pointer through the peelable def links) gates the binop arm;
   materialized binops (`v3 = n1 * 2; return v3`) no longer veto.  New
   mirror-negative unit test `rejects_const_binop_chain_folding_into_dereferenced_gep`
   guards the address-fed veto; #411's C regressions still pass.
2. **`VecMaskedAddI32x8` was classified 128-bit** by `vector_result_width()`
   (grouped with the widening ops by name similarity; the emitter is 256-bit:
   vpcmpgtd/vpand/vpaddd in YMM).  Every guarded equal-width I32 reduction
   (`vector_guard_sum`: `nat=279` vs GCC `651`) lost lanes 4–7 per iteration
   through a `movdqa`-spelled latch copy of a 256-bit accumulator.
   Fixed by moving the variant to the 256-bit arm.
3. **#411's liveness-model veto over-triggered on same-block latches.**  A phi
   that is live-in (used early in the body block) and live-out (redefined by the
   latch copy) gets a whole-block segment from the block-granular builder, which
   overlaps every interior update window — every tight-loop accumulator web was
   vetoed (`check_integer_reduction_vecreg_codegen`: four accumulator homes →
   `[]`), a silent #411-introduced codegen regression.  Fix: the liveness walk
   now records the EXACT hidden-read points (`LivenessResult::folded_read_points`,
   value id → folded-access program points, including copy-chain members), and
   the veto consults those points plus the IR-visible scan
   (`phi_used_in_window`) instead of the segment cover.  #411's miscompile class
   (`p = &a[i]; next = i + 1; *p = ...; i = next` — GEP outside the window, SIB
   re-read inside) is still caught exactly; `check_multiple_fp_reductions_codegen`
   returned to green.
4. **Harness integrity**: seven regression entries reported GCC-output MISMATCH
   where GCC could not build the reference at all (missing `-lm`:
   `vectorize_map_expr_tree`, `scalar_fp_builtins`; GCC 14 rejects the construct:
   `builtin_apply_return` (`__builtin_apply` type rules),
   `regress_tentative_incomplete_global_array_size` (tentative incomplete array),
   `segfs_pointer_declarator_matrix` / `vzeroupper_after_ymm` (implicit
   declarations)).  The outputs are byte-identical when GCC does build.  Fixed
   honestly: `-lm` added to the two `.flags` sidecars, `LCCC_NO_COMPARE=1`
   markers added for the four defective-oracle cases (the runner's documented
   mechanism).
5. **Environment limitations classified** (not compiler defects): the six i686
   `rc=159` runs and `check_i686_overalign_interop` need a 32-bit ELF
   interpreter this host image lacks (`/lib/ld-linux.so.2` absent);
   `check_arm_csinc_select` needs `aarch64-linux-gnu-gcc`, also absent.
6. **Stress harness note**: the `abi` family raised `FileNotFoundError` when
   `--lccc` was given a relative path (the generator shells out with `cwd=` the
   case directory); an absolute compiler path is required.

### Remaining open items (pre-existing codegen-quality gates, precisely scoped)

- `check_integer_reduction_vecreg_codegen`: 2 of 4 accumulator homes
  (`ymm6`/`ymm7`) — the state before PR #411; the residual pair predates it.
- `check_reduction_vecreg_codegen` (p15_sum_f32 accumulator not in place),
  `check_vector_dot_fma_codegen` (p17_dot_f32 transient spill),
  `check_affine_map_vectorization_codegen` (affine_f64 missing `vmulpd`),
  `check_fma_dest_coalesce_codegen`, `check_bitop_nonneg_zext`,
  `check_global_addr_remat`, `check_gvn_disjoint_epochs`,
  `check_licm_alias_disjoint`, `check_machinst_fallback_replay` — all predate
  PRs #409–#411 (verified identical on 55051627).
- **NEW isolated upstream miscompile (top priority for the next session)**: a
  `for`-loop counter's init copy inside `main` vanishes when an intervening
  inlined loop follows it.  Repro (outside the corpus): fill loop + inlined
  vectorizable kernel + a subsequent `for (i = 0; i < 257; i++) printf(...)`
  loop — the `%i = Copy(0)` init exists in the pre-codegen IR but no register
  (or slot, under `CCC_NO_REGALLOC`) write is emitted; the loop then runs from
  stale register/slot contents (SIGSEGV, or exactly 193 = 257−64 lines with a
  stale 64 start under `CCC_NO_REGALLOC`).  Independent of RA and of
  phi-coalescing (`CCC_NO_PHI_COALESCE=1` unchanged).  Next step: trace the
  preheader-init emission path through isel for a counter whose single use is
  a fused compare at the loop head.

### Validation (S11, final, on 4927a774 + S09/S10 fixes + items 1–4)

- cargo test --lib: **1925/1925** (upstream as merged: 1911/1); clippy **0 warnings**.
- Regression corpus: **678 pass / 0 real fail / 18 accounted** (8 environment +
  10 pre-existing quality checks listed above) vs upstream-as-merged 666/26.
- Stress sweep seeds 0:4 × 40 cases × O0,O1,O2,O3,Os × rt/cf: **700/700**.
- Peephole families (narrow/shiftchain/flags) seeds 8:14 × 60 cases × 4 levels:
  **all clean**.
- Benchmark output gate: **156/156**.
- Executable soundness proofs kept green: guard-page speculation
  (`ifconv_speculative_load_coverage.c`), distinct-scale store sinking
  (`ifconv_store_sink_distinct_scales.c`), FP web interference
  (`fp_copy_web_interference.c`, byte-identical to GCC), guarded reduction
  (`vector_guard_sum.c`, byte-identical to GCC).

## Accomplished

### Intrinsics + backend
- 16 new intrinsics: `VecMin/VecMax/VecCmp/VecBlendv` × {F32x8, F32x4, F64x4, F64x2} with the
  exact x86 operand contract documented in the enum (`min = a<b?a:b`, SECOND operand returned
  on unordered/both-zero; cmp `[lhs, rhs, imm]`, imm ∈ {EQ_OQ=0, LT_OS=1, LE_OS=2, NEQ_UQ=4};
  blendv `[false, true, mask]`).
- x86 emitters: AVX `vminps/vmaxps/vminpd/vmaxpd` (non-commutative order preserved:
  `op args[1], args[0], dst`), `vcmpps/vcmppd` + 128-bit `cmpps/cmppd` with imm,
  `vblendvps/vblendvpd` + SSE2-baseline `andps/andnps/orps` 128-bit triple (no SSE4.1 dep).
  Mask-first resolution in the AVX blend emitter keeps the deferred-cmp cache hit —
  now in BOTH the `(None, None)` and `(mask not homed, true homed)` arms (R4).
- Cohesion wiring (`copy_coalescing.rs`): memfold consumers, two-operand binary,
  SSA producers, cache-aware 3-op classification, VDEFER positions.  The 16 ops are
  NOT in `is_raw_reader_intrinsic` (their emitters read through the register cache,
  never raw slots — a forced dead store + reload per consumer was removed).
- Peephole: FP-neutral classification for cmpps/cmppd/blendvps/blendvpd.
- All four FP `VecStore` arms consult the store SOURCE's register home first
  (`vec_store_source_256/128`: `reg_assignments` XMM home →
  `phys_reg_name_256`/`phys_reg_name`, then block-local `vec_live_regs`, else the
  canonical `%ymm0`/`%xmm0` slot/peephole path) — a register-homed store source no
  longer bounces through its slot.

### Vectorizer (late pass map analyzer)
- `MapExpr` extended with `Cmp`, `Select`, `MinMax` (+ `node_count`, `PartialEq`).
- `parse_map_expr`: Cmp arm (FP-only, GT/GE operand swap, ordered EQ/LT/LE/NE subset),
  Select arm (mask must be a loop-local FP Cmp), and three exact min/max folds:
  1. `l < r ? l : r` → `min(l, r)`;  2. `l > r ? l : r` → `max(l, r)`;
  3. `l > r ? r : l` → `min(r, l)` (the clamp tail `a > 1 ? 1 : a`).
  Swapped-arm `l < r ? r : l` and `<=`/`>=` forms deliberately do NOT fold (MINPS/MAXPS
  ±0 semantics differ) — they lower to cmp + blendv.  All folds are IEEE-exact
  (second-source-on-unordered matches the ternary lane-for-lane).
- `MapEmitCtx`: cmp/blendv/minmax arms; whole-tree CSE (free: one `a` load per clamp);
  blendv emits false, true, then the mask LAST so the deferred cmp result stays in ymm0.
- `map_tree_ops_available` + `expr_uses_stream` extended; per-target cmp/blendv/minmax
  op tables in `transform_map_vector`; scalar remainder mirror for Cmp/Select/MinMax.
- `find_reduction_byte_iv` resolves Copy/Cast chains between the GEP offset and the
  shl/mul scale (store sinking / late passes introduce per-block copies).
- Failure-path diagnostics: every silent `analyze_map_pattern` bail is tagged
  `[VEC-MAP] BAIL: <reason>` under `LCCC_DEBUG_VECTORIZE`.

### if_convert (new capabilities)
- **Store sinking** (`sink_conditional_stores`): all-predecessor stores to the same
  canonical address merge into one phi-driven store in the common successor (fresh
  address chain cloned into the merge block; every SSA def dominates).  Value-exact
  in every context (no loop precondition); address identity is the INJECTIVE key (R2).
- **Partial-merge conversions**: diamonds whose merge has >2 predecessors convert their
  own phis partially (fresh Select + rewritten surviving phi).  This is what lets nested
  conditionals convert inside-out (p27's outer diamond only became a 2-pred merge after
  the inner triangle converted).
- **Arm-load speculation under the path-coverage rule** (`speculative_load_ok`): a load
  in an arm may be hoisted iff its canonical key is dereferenced on every path
  pred→merge — diamond: `K ∈ derefs(P) ∪ derefs(other arm)`; triangle: `K ∈ derefs(P)`.
  Trap-for-trap equivalent; everything else stays behind the branch (fail closed).

### FP horizontal-adds (lost-work adoption)
- All 6 FP horizontal-add emitters (legacy + Vec families) store their scalar result
  via `store_xmm_to`.  Admitted the 6 ops to `collect_f64_values`.

### Allocator vec-reg homes (S09: follow-ups #1 + #2 DONE)
- **`collect_x86_map_intermediate_values`** (new, regalloc.rs): admits map-loop
  intermediates — stream loads, packed compares, blends, min/max and arithmetic
  results — whose EVERY consumer reads through a home-aware path (emitter register
  cache, `vec_home_256/128`, or the home-aware `VecStore` source path).  Fail-closed:
  one non-listed consumer, an address-side use (intrinsic `dest_ptr` / Store-Load ptr /
  GEP base / terminator) strands the value on the stack.  Multi-use webs (the clamp
  body's load feeding a min AND a compare) are exactly what this collector exists for.
- **Broadcast legal consumers widened**: `collect_x86_map_broadcast_values` now admits
  the 16 new ops (Cmp/Min/Max/Blendv, both widths, plus Sub/Div/Sqrt for the FP
  classes), so loop-invariant broadcasts get YMM homes even when they feed the new
  select/clamp ops (follow-up #2).
- **Address-side hardening (both collectors)**: any use reached through
  `for_each_value_use_in_instruction` (intrinsic dest_ptr, ptr positions) is
  unconditionally rejected.  6 unit tests cover admission + all three rejection classes.
- **Store side**: see the `VecStore` home consultation above.

### Reduction-tail coalescing (S10: follow-up #3 DONE)
- Horizontal-reduce ops admitted to `collect_non_gpr_values` (their filter runs FIRST —
  without this the combine value never reached the XMM scan and was slot-homed).
- `fp_copy_web_groups` (regalloc.rs): union-find over Copy edges among
  `f64_value_set ∩ real_use` scan-eligible values; one merged interval per web; all
  members share the leader's register (members filtered out of the scan —
  call-spanning, zero-length, already-assigned — never inherit it).
- Direct-emit: `VecHorizontalAddF64x4`/`VecHorizontalAddF32x8` write the reduction
  steps straight into the destination register with 3-operand VEX (%xmm1 scratch;
  dest==%xmm1 routes through the legacy sequence + one move — see audit item 6).
  The unpack step reads the SUMMED halves (the pre-sum-input variant silently dropped
  lane 3, caught by the p16 differential).
- Result: p16/p23 tails have ZERO transport moves (combine → loop register-direct).

### vex_promote refinement (S10: PR #409 red-team, adopted)
- Reg-reg `movd`/`movq` never rewritten (no safe VEX spelling); SSE2 immediate-less
  `pextrw` guarded; `movntdq/movntdqa/movntps/movntpd/aeskeygenassist` added.
- Per-line reachability masks replace the whole-function gate (reverse BFS over the
  emitted text; indirect/unresolved branches fail closed; write-aware read detection;
  kill-aware absorption).  17 unit tests.  Only reachability-gated sites stay legacy
  (live upper-half data across spill/fill, e.g. clmul256).

### Tests & measurements
- `tests/regression/vectorize_fp_minmax_select.c` (+ `.flags`): 12 kernels hashed over
  NaN/±0/∞/subnormal inputs; differential vs GCC under the same flags; runner also arms
  the IR verifier and the A/B small-slot pass.  Byte-identical output.
- `tests/benchmark/patterns/simd_fp_oracle.c`: six patterns separating the shapes that
  must not fold (`a < b ? b : a`, `a <= b ? a : b`) from those that must (nested
  ternary clamp, min/max chain, and their F64 twins) — FNV-1a output hashing.
- if-conversion soundness regressions: `ifconv_speculative_load_coverage.c` (guard
  page, -O3 -march=x86-64-v3), `ifconv_store_sink_distinct_scales.c` (-O2).
- Restored: `peephole_cascaded_shift_far_use.c` (-O1), `peephole_recurrence_3op_imul.c`
  (-Os, 27-case matrix).
- Unit tests: 16 map/if-conversion/collector tests (S09) + 17 vex_promote tests +
  web-group and direct-emit tests (S10).

## TODO / open follow-ups (next sessions)

0. **Masked loads (GCC parity for the uncovered select shapes) — the top priority.**
   After the soundness fixes, `p28_select`/`k_select3`/`k_select_thr` keep the branchy
   lowering until the vectorizer can if-convert diamonds itself with
   `vmaskmovps/vmaskmovpd` (AVX only; SSE2 targets keep the branchy lowering): (a) 4
   new intrinsics `VecMaskLoad{F32x8,F64x4,F32x4,F64x2}`, (b) emitters consuming the
   deferred cmp result as the mask register (join `is_cache_aware_3op`), (c) the late
   map analyzer learning to parse the still-branchy diamond itself and emit
   `Select + masked stream loads`, (d) a branchy or masked scalar remainder.  This is
   vectorizer-side if-conversion — the GCC/LLVM architecture — and needs its own
   validation battery.
1. **Vector register homes for multi-use intermediates** (p24/p25, p27 residual
   dead-store traffic) — the defer-overflow promotion path; allocator work.
2. **VecCmp as memfold consumer**: `vcmpltps (mem a), %ymm_inv, %ymm0` would drop the
   explicit load+move when the compare's first operand is a fresh stream load
   (the `pending_vec_memfold` machinery already has the elision plumbing).
3. **F32 128-bit path verification**: `LCCC_FORCE_MAP_SSE` / SSE2-only targets exercise
   the 128-bit cmpps/blendv triple; add an oracle run with `-mno-avx` for p24b/p27b.

## Procedures honored
- fastbuild preset, -O1, -j2; swap recreated after wipe; re-based on latest main
  (f41363a → 694f6fa → 2574c30 → 55051627); harness wipe recovery via fresh clone +
  file overlay; probe cleanliness (no CCC_DUMP_IR_* raw dumps left; all diagnostics
  env-gated); no shortcuts: every fold proven lane-exact against MINPS/MAXPS semantics
  and the full NaN/±0 differential; every soundness claim in this file is backed by an
  executable regression or a line-level citation.
