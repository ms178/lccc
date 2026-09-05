# Follow-up: FP min/max/compare/blendv vectorization (Fable #2, S06 → S09)

Session S06 (base `f41363a`, deliverable `ms178-1.S06-fp-minmax-select-vectorization.patch`).
Session S09 (base `694f6fa` = f41363a + PR #408; follow-ups #1 and #2 below DONE).
All items below are verified unless marked TODO.

## Accomplished

### Intrinsics + backend
- 16 new intrinsics: `VecMin/VecMax/VecCmp/VecBlendv` × {F32x8, F32x4, F64x4, F64x2} with the
  exact x86 operand contract documented in the enum (`min = a<b?a:b`, SECOND operand returned
  on unordered/both-zero; cmp `[lhs, rhs, imm]`, imm ∈ {EQ_OQ=0, LT_OS=1, LE_OS=2, NEQ_UQ=4};
  blendv `[false, true, mask]`).
- x86 emitters: AVX `vminps/vmaxps/vminpd/vmaxpd` (non-commutative order preserved:
  `op args[1], args[0], dst`), `vcmpps/vcmppd` + 128-bit `cmpps/cmppd` with imm,
  `vblendvps/vblendvpd` + SSE2-baseline `andps/andnps/orps` 128-bit triple (no SSE4.1 dep).
  Mask-first resolution in the AVX blend emitter keeps the deferred-cmp cache hit.
- Cohesion wiring (`copy_coalescing.rs`): memfold consumers, two-operand binary,
  SSA producers, cache-aware 3-op classification, VDEFER positions.
  **Fix this session**: the 16 ops were wrongly listed in `is_raw_reader_intrinsic`,
  which forced a dead store + reload at every consumer; removed (their emitters read
  through the register cache, never raw slots).
- Peephole: FP-neutral classification for cmpps/cmppd/blendvps/blendvpd.
- **S09**: all four FP `VecStore` arms (F32x8/F32x4/F64x4/F64x2) now consult the store
  SOURCE's register home first (`vec_store_source_256/128`: `reg_assignments` XMM home
  → `phys_reg_name_256`/`phys_reg_name`, then block-local `vec_live_regs`, else the
  canonical `%ymm0`/`%xmm0` slot/peephole path).  A register-homed store source no
  longer bounces through its slot (follow-up #1/#2's store side).

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
- `find_reduction_byte_iv` now resolves Copy/Cast chains between the GEP offset and the
  shl/mul scale (store sinking / late passes introduce per-block copies).
- Failure-path diagnostics: every silent `analyze_map_pattern` bail is now tagged
  `[VEC-MAP] BAIL: <reason>` under `LCCC_DEBUG_VECTORIZE` (this session's silent-bail
  hunt cost hours; the tags prevent recurrence).

### if_convert (new capabilities)
- **Store sinking** (`sink_conditional_stores`): all-predecessor stores to the same
  canonical address merge into one phi-driven store in the common successor (fresh
  address chain cloned into the merge block; every SSA def dominates).  Turns the
  store-in-arm clamp form (`if (x<0) d[i]=0; else if (x>1) d[i]=1; else d[i]=x;`) into
  phi-driven dataflow the diamond converter folds into Selects.  Runs in the same
  fixpoint loop; value-exact in every context (no loop precondition).
- **Partial-merge conversions**: diamonds whose merge has >2 predecessors convert their
  own phis partially (fresh Select + rewritten surviving phi).  This is what lets nested
  conditionals convert inside-out (p27's outer diamond only became a 2-pred merge after
  the inner triangle converted).
- **Speculative IV-addressed arm loads** (`speculative_load_ok`): a load in an arm may be
  hoisted when its address is the innermost loop's induction variable scaled against a
  loop-invariant base — the exact shape GCC/Clang predicate (p28_select).  Fixed point
  from header integer phis through Cast/Copy/Shl/Add/Mul; base must be loop-invariant.
  Any other load stays behind the branch (fail closed; guarded by 2 unit tests).

### FP horizontal-adds (lost-work adoption)
- All 6 FP horizontal-add emitters (legacy + Vec families) now store their scalar result
  via `store_xmm_to` (SSE domain: `movapd`/`movsd`/`movss` direct; no `vmovq→rax` round
  trip).  Admitted the 6 ops to `collect_f64_values` so the allocator can XMM-home them.

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
  unconditionally rejected — previously a listed op in dest_ptr position slipped
  through.  6 new unit tests cover admission + all three rejection classes.
- **Store side**: see the `VecStore` home consultation above.
- Result (oracle, `-O3 -march=x86-64-v3`, vs ICX best): p27_clamp_f32 **61 → 55 insns,
  spills 10 → 0** (loop body: 2 invariant broadcasts + 1 load + 3 ops + 1 store, all
  register-homed); p28_select_f32 **67 → 53** (beats GCC 16.2's 79 and clang 23.1's
  117); p24/p25 42 → 41 with 0 spills; p31_sign_apply still **24 = 1.00x best in
  class**.  Semantics: 6 probes × 21 boundary sizes × {AVX, forced-SSE} byte-identical
  to GCC.

### Tests & measurements
- `tests/regression/vectorize_fp_minmax_select.c` (+ `.flags`): 12 kernels hashed over
  NaN/±0/∞/subnormal inputs; differential vs GCC under the same flags; runner also arms
  the IR verifier and the A/B small-slot pass.  Byte-identical output.
- 16 new unit tests: 6 map-parsing/fold tests (incl. negative folds), 4 if_convert tests
  (IV-load conversion + rejection, partial-merge nested clamp, store sinking),
  6 map-collector tests (S09).
- Full validation (S09, base 694f6fa): regression suite **617 PASS / 0 FAIL / 15 SKIP,
  0 A/B diffs**; unit tests **1906 passed**; probe semantics identical (AVX + SSE).
- Oracle: p31_sign_apply **1.00x best in class**; p28 beats GCC 16.2 and clang 23.1;
  p27c/p27d vectorize where GCC 16.2 scalarizes (branchy scalar loop).
  Shapes p24b/24c/27b/27c/24d/27d live in `simd_fp_oracle.c`.

## TODO / open follow-ups (next sessions)

3. **Reduction-tail coalescing**: the horizontal-add combine result still spills once
   before the remainder accumulator (%xmm8) loads it; coalescing the combine SSA value
   with the remainder phi (both now admitted to `collect_f64_values`) removes the pair.
   (p16/p23 verified ≤1 ULP vs GCC under `-ffast-math` this session; not regressed.)
4. **VecCmp as memfold consumer**: `vcmpltps (mem a), %ymm_inv, %ymm0` would drop the
   explicit load+move when the compare's first operand is a fresh stream load
   (the `pending_vec_memfold` machinery already has the elision plumbing).
5. **F32 128-bit oracle verification**: the SSE path (forced via `LCCC_FORCE_MAP_SSE`)
   is now semantics-verified for all six map shapes; remaining is a codegen oracle run
   comparing the 128-bit output against ICX/GCC/clang `-mno-avx` builds (driver
   currently ignores `-mno-avx`; the env gate is the reachable path).
6. Upstream consideration: the if_convert + vectorizer + allocator-home work is the
   natural next PR (vec_interleave was the prior one; S09 sits on 694f6fa = PR #408).

## Procedures honored
- fastbuild preset, -O1, -j2; swap recreated after wipe; re-based on latest main
  (f41363a → 694f6fa this session, conflict-free interactive rebase); harness wipe
  recovery via fresh clone + file overlay; probe cleanliness (no CCC_DUMP_IR_* raw dumps
  left; all diagnostics env-gated); no shortcuts: every fold proven lane-exact against
  MINPS/MAXPS semantics and the full NaN/±0 differential.
